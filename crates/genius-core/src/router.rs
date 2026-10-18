//! Automatic model routing ("Auto" mode): Genius picks the best model the user's key can
//! actually use for each job, so nobody has to choose (or choose wrong).
//!
//! Only models present in the user's cached `/models` listing are candidates. Each known
//! family has a curated quality/speed score; ids are matched by substring so newer versions
//! (deepseek-v3.2, kimi-k2.5, qwen3.5 …) still score well, and higher versions win within a
//! family. Models that failed as unavailable during this session are skipped.
//!
//! The static prior is combined with what Genius has observed ([`crate::model_stats`]):
//! models with many failures or malformed tool calls drop, recently unavailable models are
//! avoided, a model whose context window overflowed is skipped for requests that will not fit,
//! and speed-weighted jobs (Fast effort, quick/helper roles) prefer models that answered
//! quickly. Nemotron keeps its strong prior, so it leads while healthy but loses to a model
//! that observably performs better.
//!
//! Effort and work budget: [`estimate_complexity`] grades a request; with auto effort
//! (`Settings::auto_effort`) [`effort_for_complexity`] picks the effort (never above the
//! configured one), and [`needs_planning`] / [`needs_final_review`] decide when Max work gets an
//! architect plan first and a reviewer pass at the end ([`Genius::plan_work`]).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::app::Genius;
use crate::error::Result;
use crate::events::{EventKind, EventStatus};
use crate::model_stats::ModelStats;
use crate::nvidia::{infer_category, version_after, ChatMessage, ModelInfo, ProviderError};
use crate::settings::{Effort, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    /// Main coding agent and Build Doctor.
    Agent,
    /// Read-only helper analysts (architect, reviewer, …).
    Subagent,
    /// Short utility calls: commit messages, prompt enhancement.
    Quick,
    /// Image understanding.
    Vision,
    /// Spec planning.
    Plan,
    /// Whole-app generation.
    Code,
}

impl ModelRole {
    pub const ALL: [ModelRole; 6] = [ModelRole::Agent, ModelRole::Code, ModelRole::Plan, ModelRole::Subagent, ModelRole::Quick, ModelRole::Vision];

    pub fn as_str(&self) -> &'static str {
        match self {
            ModelRole::Agent => "agent",
            ModelRole::Subagent => "subagent",
            ModelRole::Quick => "quick",
            ModelRole::Vision => "vision",
            ModelRole::Plan => "plan",
            ModelRole::Code => "code",
        }
    }
}

/// Longest fallback chain returned by [`candidates`].
pub const MAX_CANDIDATES: usize = 4;

const VISION: u8 = 1;
const CODER: u8 = 2;
const REASONING: u8 = 4;

/// A model family: every `all` substring present, no `none` substring present.
struct Family {
    all: &'static [&'static str],
    none: &'static [&'static str],
    /// Version prefix and the version the scores are calibrated for.
    ver: (&'static str, f32),
    quality: f32,
    speed: f32,
    tags: u8,
}

const fn fam(all: &'static [&'static str], none: &'static [&'static str], ver: (&'static str, f32), quality: f32, speed: f32, tags: u8) -> Family {
    Family { all, none, ver, quality, speed, tags }
}

const NOVER: (&str, f32) = ("", 0.0);

/// First match wins, so specific entries come before generic ones.
const FAMILIES: &[Family] = &[
    // --- vision-capable
    fam(&["nemotron", "vl"], &[], NOVER, 78.0, 85.0, VISION | REASONING),
    fam(&["qwen", "vl"], &[], ("/qwen", 2.5), 82.0, 55.0, VISION),
    fam(&["90b-vision"], &[], NOVER, 76.0, 45.0, VISION),
    fam(&["vision"], &[], NOVER, 64.0, 70.0, VISION),
    fam(&["multimodal"], &[], NOVER, 60.0, 80.0, VISION),
    fam(&["llama-4-maverick"], &[], NOVER, 80.0, 70.0, VISION),
    fam(&["llama-4-scout"], &[], NOVER, 72.0, 80.0, VISION),
    fam(&["gemma-3"], &[], NOVER, 66.0, 75.0, VISION),
    fam(&["mistral-medium-3"], &[], NOVER, 78.0, 65.0, VISION),
    // --- NVIDIA Nemotron (the user's favourite: nemotron-3-super leads for agent work)
    fam(&["nemotron", "super"], &["llama"], ("nemotron-", 3.0), 97.0, 55.0, REASONING | CODER),
    fam(&["nemotron", "ultra"], &["llama"], ("nemotron-", 3.0), 97.0, 35.0, REASONING | CODER),
    fam(&["nemotron-ultra"], &[], NOVER, 86.0, 30.0, REASONING),
    fam(&["llama", "nemotron-super"], &[], ("nemotron-super-49b-v", 1.0), 80.0, 60.0, REASONING),
    fam(&["nemotron-nano-9b"], &[], NOVER, 64.0, 92.0, REASONING),
    fam(&["nemotron", "nano"], &["llama"], ("nemotron-", 3.0), 78.0, 92.0, REASONING),
    fam(&["nemotron"], &["340b"], NOVER, 60.0, 70.0, 0),
    // --- Qwen
    fam(&["qwq"], &[], NOVER, 70.0, 40.0, REASONING),
    fam(&["qwen2"], &[], NOVER, 62.0, 70.0, 0),
    fam(&["qwen", "coder", "30b"], &[], NOVER, 76.0, 90.0, CODER),
    fam(&["qwen", "coder"], &[], ("/qwen", 3.0), 94.0, 55.0, CODER),
    fam(&["qwen", "397b"], &[], ("/qwen", 3.5), 93.0, 45.0, REASONING | CODER),
    fam(&["qwen", "235b"], &[], ("/qwen", 3.0), 89.0, 50.0, REASONING),
    fam(&["qwen", "next"], &[], ("/qwen", 3.0), 85.0, 85.0, 0),
    fam(&["qwen"], &[], ("/qwen", 3.0), 80.0, 70.0, 0),
    // --- DeepSeek
    fam(&["deepseek-r1"], &["distill"], NOVER, 84.0, 25.0, REASONING),
    fam(&["distill"], &[], NOVER, 58.0, 70.0, REASONING),
    fam(&["deepseek-v"], &[], ("deepseek-v", 3.1), 92.0, 40.0, CODER | REASONING),
    fam(&["deepseek"], &[], NOVER, 60.0, 50.0, 0),
    // --- other strong open coders
    fam(&["kimi-k"], &[], ("kimi-k", 2.0), 93.0, 50.0, CODER),
    fam(&["glm"], &[], ("glm", 4.5), 91.0, 50.0, CODER | REASONING),
    fam(&["minimax-m"], &[], ("minimax-m", 2.0), 89.0, 60.0, CODER),
    fam(&["gpt-oss", "120b"], &[], NOVER, 87.0, 75.0, REASONING),
    fam(&["gpt-oss", "20b"], &[], NOVER, 73.0, 95.0, REASONING),
    fam(&["seed-oss"], &[], NOVER, 82.0, 55.0, REASONING),
    fam(&["devstral"], &[], NOVER, 82.0, 60.0, CODER),
    fam(&["mistral-large"], &[], ("mistral-large", 2.0), 80.0, 45.0, 0),
    fam(&["mistral-medium"], &[], NOVER, 76.0, 65.0, 0),
    fam(&["mistral-small"], &[], NOVER, 66.0, 85.0, 0),
    fam(&["codestral"], &[], NOVER, 66.0, 70.0, CODER),
    fam(&["mixtral"], &[], NOVER, 58.0, 60.0, 0),
    // --- Llama and small models
    fam(&["llama-3.1-405b"], &[], NOVER, 76.0, 30.0, 0),
    fam(&["llama-3.3-70b"], &[], NOVER, 74.0, 80.0, 0),
    fam(&["llama-3.1-70b"], &[], NOVER, 70.0, 78.0, 0),
    fam(&["phi-4"], &[], NOVER, 58.0, 85.0, 0),
    fam(&["granite"], &[], NOVER, 55.0, 80.0, 0),
    fam(&["llama-3.1-8b"], &[], NOVER, 50.0, 98.0, 0),
    fam(&["gemma-2"], &[], NOVER, 52.0, 80.0, 0),
];

fn family(id: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.all.iter().all(|a| id.contains(a)) && !f.none.iter().any(|n| id.contains(n)))
}

/// Never route to non-chat models (embeddings, rerankers, guards, speech, OCR, base models).
fn routable(m: &ModelInfo) -> bool {
    let l = m.id.to_lowercase();
    matches!(infer_category(&m.id), "chat" | "code") && !["reward", "-base", "nemoguard", "translate"].iter().any(|x| l.contains(x))
}

/// Known vision-capable model (by id pattern; a vision probe can confirm or refute it).
pub fn is_vision(model: &str) -> bool {
    family(&model.to_lowercase()).is_some_and(|f| f.tags & VISION != 0)
}

/// Bonus for newer releases within a family.
fn version_bonus(id: &str, f: &Family) -> f32 {
    let mut b = 0.0;
    if !f.ver.0.is_empty() {
        if let Some(v) = version_after(id, f.ver.0) {
            b += ((v - f.ver.1) * 4.0).clamp(-10.0, 8.0);
        }
    }
    // dated refreshes (…-0905, …-2507) and "terminus" builds are newer revisions
    if id.split(['-', '_', '/']).any(|seg| seg.len() == 4 && seg.chars().all(|c| c.is_ascii_digit())) {
        b += 0.2;
    }
    if id.contains("terminus") {
        b += 0.3;
    }
    b
}

fn score(id: &str, f: &Family, role: ModelRole, effort: Effort) -> Option<f32> {
    let q = f.quality + version_bonus(id, f);
    let s = f.speed;
    let tag = |t: u8| f.tags & t != 0;
    Some(match role {
        ModelRole::Vision if !tag(VISION) => return None,
        ModelRole::Vision => match effort {
            Effort::Max => q,
            _ => q * 0.7 + s * 0.3,
        },
        ModelRole::Agent | ModelRole::Code | ModelRole::Plan => {
            let q = q + if role == ModelRole::Code && tag(CODER) { 1.0 } else { 0.0 } + if role == ModelRole::Plan && tag(REASONING) { 1.0 } else { 0.0 };
            match effort {
                Effort::Max => q,
                Effort::Balanced => q * 0.9 + s * 0.1,
                Effort::Fast => q * 0.6 + s * 0.4,
            }
        }
        ModelRole::Subagent | ModelRole::Quick => match effort {
            Effort::Max => q,
            Effort::Balanced => q * 0.5 + s * 0.5,
            Effort::Fast => q * 0.35 + s * 0.65,
        },
    })
}

/// What Genius has observed, for [`rank`]: per-model stats and the size of the request.
#[derive(Debug, Clone, Default)]
pub struct Observed {
    pub stats: HashMap<String, ModelStats>,
    /// Characters of the request about to be sent (0: unknown).
    pub request_chars: usize,
    /// Unix seconds (for the expiry of "recently unavailable").
    pub now: i64,
}

/// A request at least this large avoids models that overflowed without a measured limit.
pub const LARGE_REQUEST_CHARS: usize = 60_000;
/// Latency the speed bonus is measured against (time to first token).
const REFERENCE_LATENCY_MS: f64 = 4000.0;

/// Rough token estimate for a request of `chars` characters (~4 characters per token).
pub fn estimate_tokens(chars: usize) -> usize {
    chars.div_ceil(4)
}

/// How much latency matters for a job (score points for an instant answer vs the reference).
fn latency_weight(role: ModelRole, effort: Effort) -> f32 {
    match (role, effort) {
        (_, Effort::Max) => 0.0,
        (ModelRole::Quick | ModelRole::Subagent, Effort::Fast) => 14.0,
        (_, Effort::Fast) => 10.0,
        (ModelRole::Quick | ModelRole::Subagent, Effort::Balanced) => 8.0,
        (_, Effort::Balanced) => 2.0,
    }
}

/// Score adjustment from observed behaviour; None when the request will not fit the model's
/// observed context window.
pub fn observed_adjustment(st: Option<&ModelStats>, role: ModelRole, effort: Effort, request_chars: usize, now: i64) -> Option<f32> {
    let Some(st) = st else { return Some(0.0) };
    if request_chars > 0 && !st.fits(request_chars) {
        return None;
    }
    let mut a = -(st.failure_rate() as f32) * 40.0;
    let tool_weight = match role {
        ModelRole::Agent | ModelRole::Subagent => 50.0,
        ModelRole::Code | ModelRole::Plan => 15.0,
        ModelRole::Quick | ModelRole::Vision => 0.0,
    };
    a -= st.malformed_rate() as f32 * tool_weight;
    if st.recently_unavailable(now) {
        a -= 60.0;
    }
    if st.context_limit_chars.is_none() && st.context_overflows > 0 && request_chars >= LARGE_REQUEST_CHARS {
        a -= 25.0;
    }
    if let Some(lat) = st.latency_ms.filter(|_| st.latency_samples > 0) {
        let confidence = st.latency_samples.min(5) as f32 / 5.0;
        let rel = ((REFERENCE_LATENCY_MS - lat) / REFERENCE_LATENCY_MS).clamp(-1.0, 1.0) as f32;
        a += latency_weight(role, effort) * rel * confidence;
    }
    // Proven reliability is worth a little (at most 2 points).
    a += st.successes.min(20) as f32 * 0.1 * (1.0 - st.failure_rate() as f32);
    Some(a)
}

/// Ordered fallback chain from the static prior alone (no observations); see [`rank`].
pub fn candidates(models: &[ModelInfo], role: ModelRole, effort: Effort, failed: &HashSet<String>) -> Vec<String> {
    rank(models, role, effort, failed, &Observed::default())
}

/// Ordered fallback chain (best first, at most [`MAX_CANDIDATES`]) among `models` — the
/// user's `/models` listing, so nothing unavailable is ever hardcoded — for a job: the static
/// family prior plus [`observed_adjustment`]. Unknown chat models are used only when no known
/// family is available (never for vision). Models whose observed context window is too small
/// for the request are used only when nothing else is left.
pub fn rank(models: &[ModelInfo], role: ModelRole, effort: Effort, failed: &HashSet<String>, obs: &Observed) -> Vec<String> {
    let usable: Vec<&ModelInfo> = models.iter().filter(|m| routable(m) && !failed.contains(&m.id)).collect();
    let adjust = |id: &str| observed_adjustment(obs.stats.get(id), role, effort, obs.request_chars, obs.now);
    let mut too_small: Vec<(f32, &str)> = Vec::new();
    let mut scored: Vec<(f32, &str)> = Vec::new();
    for m in &usable {
        let l = m.id.to_lowercase();
        let Some(f) = family(&l) else { continue };
        let Some(base) = score(&l, f, role, effort) else { continue };
        match adjust(&m.id) {
            Some(a) => scored.push((base + a, m.id.as_str())),
            None => too_small.push((base, m.id.as_str())),
        }
    }
    let by_score = |a: &(f32, &str), b: &(f32, &str)| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(a.1));
    scored.sort_by(by_score);
    let mut out: Vec<String> = scored.into_iter().map(|(_, id)| id.to_string()).collect();
    if out.is_empty() && role != ModelRole::Vision {
        // Unknown chat models, healthiest first.
        let mut unknown: Vec<(f32, &str)> = usable.iter().filter(|m| !too_small.iter().any(|(_, t)| *t == m.id)).filter_map(|m| adjust(&m.id).map(|a| (a, m.id.as_str()))).collect();
        unknown.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
        out = unknown.into_iter().map(|(_, id)| id.to_string()).collect();
    }
    if out.is_empty() {
        too_small.sort_by(by_score);
        out = too_small.into_iter().map(|(_, id)| id.to_string()).collect();
    }
    out.dedup();
    out.truncate(MAX_CANDIDATES);
    out
}

/// How demanding a request is (drives auto effort, planning and the final review).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Complexity {
    Simple,
    Normal,
    Complex,
}

impl Complexity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Complexity::Simple => "simple",
            Complexity::Normal => "normal",
            Complexity::Complex => "complex",
        }
    }
}

const COMPLEX_WORDS: &[&str] = &[
    "from scratch", "whole app", "entire app", "full app", "complete app", "multi-page", "multiple pages", "several pages", "authentication", "login", "sign up", "signup", "database", "backend", "supabase", "api ", "refactor", "rewrite", "architecture", "migrate", "migration", "dashboard", "checkout", "payment", "real-time", "realtime", "offline", "drag and drop", "i18n", "internationali", "test suite", "end-to-end", "state management", "routing", "redesign", "every page", "all pages", "across the",
];
const SIMPLE_WORDS: &[&str] = &[
    "typo", "rename", "colour", "color", "font size", "spacing", "margin", "padding", "change the text", "change the title", "wording", "small", "tiny", "quick", "just ", "only ", "one line", "bump", "label", "placeholder", "what is", "what's", "why does", "explain", "how do",
];
const BUILD_VERBS: &[&str] = &["build", "create", "make", "generate", "develop", "baue", "erstelle", "crée", "crea "];
const APP_NOUNS: &[&str] = &["app", "website", "site", "game", "tool", "platform", "portfolio", "shop", "store", "landing page"];

/// Grade a request from its wording and the project size. Heuristic and cheap: length,
/// enumerated requirements, scope words ("authentication", "refactor", "multi-page" …) versus
/// small-change words ("typo", "rename", "color" …), and whether a whole app is being built
/// in a (nearly) empty project.
pub fn estimate_complexity(request: &str, project_files: usize) -> Complexity {
    let text = request.trim();
    let l = format!("{} ", text.to_lowercase());
    let words = text.split_whitespace().count();
    let is_item = |t: &str| t.starts_with("- ") || t.starts_with("* ") || t.starts_with("• ") || t.split_once(['.', ')']).is_some_and(|(n, _)| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit()));
    let items = text.lines().map(str::trim_start).filter(|t| is_item(t)).count();
    // Short is not the same as simple ("add a login page"): only small-change wording or a
    // question makes a request simple.
    let mut score: i32 = match words {
        0..=40 => 0,
        41..=120 => 1,
        _ => 2,
    };
    score += match items {
        0..=2 => 0,
        3..=5 => 1,
        _ => 2,
    };
    score += COMPLEX_WORDS.iter().filter(|w| l.contains(**w)).count().min(3) as i32;
    score -= SIMPLE_WORDS.iter().filter(|w| l.contains(**w)).count().min(2) as i32;
    let builds_app = BUILD_VERBS.iter().any(|v| l.contains(v)) && APP_NOUNS.iter().any(|n| l.contains(n));
    if builds_app && project_files < 8 {
        score += 3; // a whole app in an (almost) empty project
    }
    if project_files > 150 && score >= 1 {
        score += 1; // broad changes in a large codebase
    }
    if score >= 3 {
        Complexity::Complex
    } else if score <= -1 {
        Complexity::Simple
    } else {
        Complexity::Normal
    }
}

/// Effort auto mode uses for a request: never above the configured effort (`ceiling`).
pub fn effort_for_complexity(c: Complexity, ceiling: Effort) -> Effort {
    let want = match c {
        Complexity::Simple => Effort::Fast,
        Complexity::Normal => Effort::Balanced,
        Complexity::Complex => Effort::Max,
    };
    let level = |e: Effort| match e {
        Effort::Fast => 0,
        Effort::Balanced => 1,
        Effort::Max => 2,
    };
    if level(want) <= level(ceiling) {
        want
    } else {
        ceiling
    }
}

/// Max effort on complex work: an Architect subagent plans before the main agent edits.
pub fn needs_planning(effort: Effort, complexity: Complexity) -> bool {
    effort == Effort::Max && complexity == Complexity::Complex
}

/// Max effort on complex work: a Reviewer subagent checks the result before it is presented.
pub fn needs_final_review(effort: Effort, complexity: Complexity) -> bool {
    effort == Effort::Max && complexity == Complexity::Complex
}

/// How Genius will work on one request (see [`Genius::plan_work`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkPlan {
    pub complexity: Complexity,
    pub effort: Effort,
    /// The effort was chosen automatically (not fixed by the request or settings).
    pub auto_effort: bool,
    pub planning: bool,
    pub final_review: bool,
}

/// Instruction for the vision model that sees images on behalf of a text-only model.
pub const VISION_PROMPT: &str = "You are the eyes of a software engineer who cannot see images. Describe the attached image so precisely that it could be rebuilt from your words alone. If it is a user interface (screenshot, mockup, sketch): describe the overall layout and structure (regions, grid, alignment, spacing), every component (navigation, header, sidebar, buttons, inputs, cards, tables, lists, charts, icons, images) with its position, size and state, ALL visible text verbatim, colours (approximate hex values for background, surfaces, text, accents, borders), typography (relative sizes, weights), corner radii, shadows and any notable visual details. Otherwise describe precisely what it shows, including any text verbatim. Use concise bullet points grouped by region. Do not speculate beyond what is visible.";

/// Replace image attachments with the vision model's text descriptions (vision handoff).
/// `descriptions[i]` holds one description per image of `messages[i]`.
pub fn replace_images_with_descriptions(messages: &[ChatMessage], descriptions: &[Vec<String>], vision_model: &str) -> Vec<ChatMessage> {
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if m.images.is_empty() {
                return m.clone();
            }
            let mut content = m.content.clone();
            for (n, _) in m.images.iter().enumerate() {
                let d = descriptions.get(i).and_then(|d| d.get(n)).map(|d| d.trim()).filter(|d| !d.is_empty()).unwrap_or("(the image could not be described)");
                content.push_str(&format!("\n\n[Attached image {} — you cannot see images, so {vision_model} described it for you:]\n{d}", n + 1));
            }
            ChatMessage { role: m.role.clone(), content, images: vec![] }
        })
        .collect()
}

impl Genius {
    /// The cached model listing for the current base URL (no network).
    pub fn cached_models(&self) -> Vec<ModelInfo> {
        let Ok(s) = self.settings() else { return vec![] };
        let base = crate::nvidia::normalize_base_url(&s.base_url).unwrap_or(s.base_url);
        self.db.get_model_cache(&base).ok().flatten().and_then(|(_, v)| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    /// Cached models; when there is no cache yet (first run), fetch the listing — at most once
    /// every two minutes, so a failed first attempt (offline, key just added) is retried later.
    async fn ensure_models(&self) -> Vec<ModelInfo> {
        let cached = self.cached_models();
        if !cached.is_empty() {
            return cached;
        }
        {
            let mut at = self.models_fetch_at.lock().unwrap();
            if at.is_some_and(|t| t.elapsed() < Duration::from_secs(120)) {
                return cached;
            }
            *at = Some(Instant::now());
        }
        let Ok((client, _)) = self.client() else { return cached };
        let fetch = tokio::time::timeout(Duration::from_secs(20), client.list_models()).await;
        match fetch {
            Ok(Ok(models)) => {
                if let Ok(v) = serde_json::to_value(&models) {
                    let _ = self.db.put_model_cache(client.base_url(), &v);
                }
                models
            }
            _ => cached,
        }
    }

    /// Known vision-capable model: verified by a probe, or a vision family not refuted by one.
    pub fn is_vision_capable(&self, model: &str) -> bool {
        match self.vision_probe(model) {
            Some(ok) => ok,
            None => is_vision(model),
        }
    }

    /// Remember for this session that a model is unavailable, so routing skips it (the
    /// persisted stats keep a decaying penalty across restarts).
    pub fn mark_model_failed(&self, model: &str) {
        self.failed_models.lock().unwrap().insert(model.to_string());
    }

    /// Observations for ranking a request of `request_chars`.
    fn observed(&self, request_chars: usize) -> Observed {
        Observed { stats: self.model_stats(), request_chars, now: chrono::Utc::now().timestamp() }
    }

    /// Fallback chain from a given model list (see [`resolve_models`](Self::resolve_models)).
    fn chain_for(&self, s: &Settings, models: &[ModelInfo], role: ModelRole, effort: Effort, request_chars: usize) -> Result<Vec<String>> {
        let failed = self.failed_models.lock().unwrap().clone();
        let mut auto = rank(models, role, effort, &failed, &self.observed(request_chars));
        if role == ModelRole::Vision {
            // Models whose image input was verified with a real request come first.
            let verified: Vec<String> = models.iter().map(|m| m.id.clone()).filter(|m| !failed.contains(m) && self.vision_probe(m) == Some(true)).collect();
            auto.retain(|m| self.vision_probe(m) != Some(false) && !verified.contains(m));
            auto = verified.into_iter().chain(auto).take(MAX_CANDIDATES).collect();
        }
        let selected = s.model.clone().filter(|m| !m.trim().is_empty());
        if !s.is_auto() {
            let m = selected.ok_or(ProviderError::NoModel)?;
            if role == ModelRole::Vision && !self.is_vision_capable(&m) {
                return Ok(auto);
            }
            // The chosen model first; the rest are used only if it turns out to be unavailable.
            auto.retain(|x| *x != m);
            return Ok(std::iter::once(m).chain(auto).collect());
        }
        if let Some(m) = selected.filter(|m| !failed.contains(m) && !auto.contains(m)) {
            if role != ModelRole::Vision || self.is_vision_capable(&m) {
                auto.push(m);
            }
        }
        if auto.is_empty() && role != ModelRole::Vision {
            return Err(ProviderError::NoModel.into());
        }
        Ok(auto)
    }

    /// Models to try, in order, for a job. Manual mode: `[settings.model]` followed by
    /// fallbacks used only if it is unavailable. Auto mode: the best available models, then
    /// any chat model, then `settings.model`. Vision may be empty (no vision model available).
    pub async fn resolve_models(&self, role: ModelRole, effort: Effort) -> Result<Vec<String>> {
        self.resolve_models_for(role, effort, 0).await
    }

    /// [`resolve_models`](Self::resolve_models) for a request of `request_chars` characters
    /// (models whose observed context window is too small for it are avoided).
    pub async fn resolve_models_for(&self, role: ModelRole, effort: Effort, request_chars: usize) -> Result<Vec<String>> {
        let s = self.settings()?;
        let models = if s.is_auto() || role == ModelRole::Vision { self.ensure_models().await } else { self.cached_models() };
        self.chain_for(&s, &models, role, effort, request_chars)
    }

    /// [`resolve_models`](Self::resolve_models) from the cache only (never touches the network).
    pub fn resolve_models_cached(&self, role: ModelRole, effort: Effort) -> Result<Vec<String>> {
        let s = self.settings()?;
        self.chain_for(&s, &self.cached_models(), role, effort, 0)
    }

    /// Decide how to work on a request: complexity, effort (from the request if it names one,
    /// else from complexity when auto effort is on, else the settings), and whether Max work
    /// gets an architect plan first and a reviewer pass at the end. The effort is remembered
    /// for `task_id`, so every AI call of the task routes and budgets with it.
    ///
    /// Agent hook (agent.rs `run_agent`): call this before building the prompt, use
    /// `effort` for the run, run an Architect subagent first when `planning`, and a Reviewer
    /// before presenting the result when `final_review`.
    pub fn plan_work(&self, project_id: &str, task_id: &str, request: &str, requested: Option<&str>) -> WorkPlan {
        let s = self.settings().unwrap_or_default();
        let files = self.list_files(project_id).map(|f| f.len()).unwrap_or(0);
        let complexity = estimate_complexity(request, files);
        let (effort, auto_effort) = match requested.and_then(Effort::parse) {
            Some(e) => (e, false),
            None if s.auto_effort => (effort_for_complexity(complexity, s.effort), true),
            None => (s.effort, false),
        };
        let plan = WorkPlan { complexity, effort, auto_effort, planning: needs_planning(effort, complexity), final_review: needs_final_review(effort, complexity) };
        if !task_id.is_empty() {
            {
                let mut all = self.task_effort.lock().unwrap();
                if all.len() > 200 {
                    all.clear();
                }
                all.insert(task_id.to_string(), effort);
            }
            let how = if auto_effort { format!("auto effort {} for a {} request", effort.as_str(), complexity.as_str()) } else { format!("effort {} ({} request)", effort.as_str(), complexity.as_str()) };
            let extra = match (plan.planning, plan.final_review) {
                (true, true) => "; architect plan first and a final review",
                (true, false) => "; architect plan first",
                (false, true) => "; final review",
                _ => "",
            };
            self.bus.record(Some(project_id), Some(task_id), EventKind::Ai, EventStatus::Completed, format!("Working with {how}{extra}"), serde_json::to_value(plan).unwrap_or_default());
        }
        plan
    }

    /// Forget the effort chosen for a finished task.
    pub fn end_work(&self, task_id: &str) {
        self.task_effort.lock().unwrap().remove(task_id);
    }

    /// Fail fast before starting AI work: manual mode needs a selected model; auto mode needs a
    /// key (and, once the model list is known, at least one usable model).
    pub fn has_usable_model(&self) -> Result<()> {
        let s = self.settings()?;
        if !s.is_auto() {
            return match s.model.as_deref().map(str::trim) {
                Some(m) if !m.is_empty() => Ok(()),
                _ => Err(ProviderError::NoModel.into()),
            };
        }
        if !self.key_status().stored {
            return Err(ProviderError::MissingKey.into());
        }
        let models = self.cached_models();
        let failed = self.failed_models.lock().unwrap().clone();
        if !models.is_empty() && s.model.is_none() && rank(&models, ModelRole::Agent, s.effort, &failed, &self.observed(0)).is_empty() {
            return Err(ProviderError::NoModel.into());
        }
        Ok(())
    }

    /// What auto mode will use for each job with the current settings: `{ agent: [..], code, … }`,
    /// plus `stats: { model: {calls, successPct, latencyMs, toolCallErrors, …} }` for every
    /// listed model Genius has observed (models without data are absent: "no data yet").
    pub async fn resolved_models(&self) -> Result<Value> {
        let s = self.settings()?;
        let mut out = json!({});
        let all = self.model_stats();
        let mut stats = serde_json::Map::new();
        for role in ModelRole::ALL {
            let chain = self.resolve_models(role, s.effort).await.unwrap_or_default();
            for m in &chain {
                if let Some(st) = all.get(m) {
                    stats.insert(m.clone(), st.view());
                }
            }
            out[role.as_str()] = json!(chain);
        }
        out["stats"] = Value::Object(stats);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ids from the build.nvidia.com catalog (plus non-chat models that must never be picked).
    const CATALOG: &[&str] = &[
        "nvidia/nemotron-3-super-120b-a12b",
        "nvidia/nemotron-3-nano-30b-a3b",
        "qwen/qwen3-coder-480b-a35b-instruct",
        "qwen/qwen3-235b-a22b",
        "qwen/qwen3-next-80b-a3b-instruct",
        "deepseek-ai/deepseek-v3.1",
        "deepseek-ai/deepseek-v3.1-terminus",
        "deepseek-ai/deepseek-r1",
        "moonshotai/kimi-k2-instruct",
        "moonshotai/kimi-k2-instruct-0905",
        "openai/gpt-oss-120b",
        "openai/gpt-oss-20b",
        "z-ai/glm4.7",
        "minimaxai/minimax-m2",
        "nvidia/llama-3.1-nemotron-ultra-253b-v1",
        "nvidia/llama-3.3-nemotron-super-49b-v1.5",
        "meta/llama-4-maverick-17b-128e-instruct",
        "meta/llama-3.3-70b-instruct",
        "mistralai/mistral-medium-3-instruct",
        "meta/llama-3.2-90b-vision-instruct",
        "nvidia/nemotron-nano-12b-v2-vl",
        "microsoft/phi-4-multimodal-instruct",
        "nvidia/nv-embedqa-e5-v5",
        "nvidia/llama-3.2-nv-rerankqa-1b-v2",
        "nvidia/llama-3.1-nemoguard-8b-content-safety",
        "nvidia/nemotron-parse",
    ];

    fn models(ids: &[&str]) -> Vec<ModelInfo> {
        ids.iter().map(|id| ModelInfo { id: id.to_string(), owner: None, created: None, inferred_category: infer_category(id).into() }).collect()
    }

    fn pick(ids: &[&str], role: ModelRole, effort: Effort) -> Vec<String> {
        candidates(&models(ids), role, effort, &HashSet::new())
    }

    #[test]
    fn nemotron_3_super_leads_agent_work_when_available() {
        for effort in [Effort::Max, Effort::Balanced] {
            let c = pick(CATALOG, ModelRole::Agent, effort);
            assert_eq!(c[0], "nvidia/nemotron-3-super-120b-a12b", "{effort:?}: {c:?}");
            assert!(c.len() <= MAX_CANDIDATES);
            // strong coders right behind
            for m in &c[1..] {
                assert!(["qwen3-coder", "kimi-k2", "deepseek-v3", "glm", "qwen3-235b", "minimax"].iter().any(|f| m.contains(f)), "{m}");
            }
        }
        assert_eq!(pick(CATALOG, ModelRole::Plan, Effort::Max)[0], "nvidia/nemotron-3-super-120b-a12b");
        let code = pick(CATALOG, ModelRole::Code, Effort::Max);
        assert_eq!(code[0], "nvidia/nemotron-3-super-120b-a12b");
        assert_eq!(code[1], "qwen/qwen3-coder-480b-a35b-instruct");
    }

    #[test]
    fn without_nemotron_strong_coders_win() {
        let ids: Vec<&str> = CATALOG.iter().copied().filter(|m| !m.contains("nemotron-3-super")).collect();
        let c = pick(&ids, ModelRole::Agent, Effort::Max);
        assert_eq!(c[0], "qwen/qwen3-coder-480b-a35b-instruct");
        assert!(c.contains(&"moonshotai/kimi-k2-instruct-0905".to_string()), "dated refresh beats the original: {c:?}");
        assert!(!c.contains(&"moonshotai/kimi-k2-instruct".to_string()) || c.iter().position(|m| m.ends_with("0905")) < c.iter().position(|m| m.ends_with("k2-instruct")));
    }

    #[test]
    fn newer_versions_win_within_a_family() {
        let c = pick(&["deepseek-ai/deepseek-v3.1", "deepseek-ai/deepseek-v3.2", "moonshotai/kimi-k2-instruct", "moonshotai/kimi-k2.5"], ModelRole::Agent, Effort::Max);
        assert_eq!(c[0], "moonshotai/kimi-k2.5");
        assert!(c.iter().position(|m| m.ends_with("v3.2")) < c.iter().position(|m| m.ends_with("v3.1")));
        let c = pick(&["deepseek-ai/deepseek-v3.1", "deepseek-ai/deepseek-v3.1-terminus"], ModelRole::Agent, Effort::Max);
        assert_eq!(c[0], "deepseek-ai/deepseek-v3.1-terminus");
        let c = pick(&["qwen/qwen3-235b-a22b", "qwen/qwen3.5-235b-a22b", "z-ai/glm4.5", "z-ai/glm-5"], ModelRole::Agent, Effort::Max);
        assert_eq!(c[0], "z-ai/glm-5");
        assert!(c.iter().position(|m| m.contains("qwen3.5")) < c.iter().position(|m| m.contains("qwen3-")));
    }

    #[test]
    fn quick_and_subagent_prefer_fast_models_unless_max() {
        for role in [ModelRole::Quick, ModelRole::Subagent] {
            let fast = pick(CATALOG, role, Effort::Balanced);
            assert!(["nemotron-3-nano", "gpt-oss-20b", "qwen3-next", "llama-3.3-70b", "gpt-oss-120b"].iter().any(|f| fast[0].contains(f)), "{role:?}: {fast:?}");
            assert_ne!(fast[0], "nvidia/nemotron-3-super-120b-a12b");
            let fastest = pick(CATALOG, role, Effort::Fast);
            assert!(["nemotron-3-nano", "gpt-oss-20b", "qwen3-next"].iter().any(|f| fastest[0].contains(f)), "{fastest:?}");
            assert_eq!(pick(CATALOG, role, Effort::Max)[0], "nvidia/nemotron-3-super-120b-a12b");
        }
    }

    #[test]
    fn vision_only_picks_vision_models() {
        let c = pick(CATALOG, ModelRole::Vision, Effort::Max);
        assert!(!c.is_empty());
        for m in &c {
            assert!(is_vision(m), "{m}");
        }
        assert!(pick(&["nvidia/nemotron-3-super-120b-a12b", "meta/llama-3.3-70b-instruct", "x/unknown-chat"], ModelRole::Vision, Effort::Max).is_empty());
        assert!(is_vision("meta/llama-3.2-90b-vision-instruct") && is_vision("nvidia/nemotron-nano-12b-v2-vl"));
        assert!(!is_vision("nvidia/nemotron-3-super-120b-a12b"));
    }

    #[test]
    fn excludes_non_chat_and_failed_models() {
        let all = pick(CATALOG, ModelRole::Agent, Effort::Max);
        assert!(all.iter().all(|m| !m.contains("embed") && !m.contains("rerank") && !m.contains("safety") && !m.contains("parse")));
        assert!(pick(&["nvidia/nv-embedqa-e5-v5", "nvidia/nemotron-parse"], ModelRole::Quick, Effort::Fast).is_empty());
        let failed: HashSet<String> = ["nvidia/nemotron-3-super-120b-a12b".to_string()].into();
        let c = candidates(&models(CATALOG), ModelRole::Agent, Effort::Max, &failed);
        assert_eq!(c[0], "qwen/qwen3-coder-480b-a35b-instruct");
        assert!(!c.contains(&"nvidia/nemotron-3-super-120b-a12b".to_string()));
    }

    #[test]
    fn unknown_chat_models_are_a_last_resort() {
        assert_eq!(pick(&["acme/mystery-chat", "nvidia/nv-embed-v1"], ModelRole::Agent, Effort::Max), vec!["acme/mystery-chat"]);
        assert_eq!(pick(&["acme/mystery-chat", "meta/llama-3.3-70b-instruct"], ModelRole::Agent, Effort::Max), vec!["meta/llama-3.3-70b-instruct"]);
        assert!(pick(&[], ModelRole::Agent, Effort::Max).is_empty());
    }

    #[test]
    fn images_are_replaced_by_descriptions() {
        let msgs = vec![
            ChatMessage::system("sys"),
            ChatMessage { role: "user".into(), content: "Rebuild this page".into(), images: vec!["data:image/png;base64,AAA".into(), "data:image/png;base64,BBB".into()] },
            ChatMessage::assistant("ok"),
        ];
        let out = replace_images_with_descriptions(&msgs, &[vec![], vec!["- dark navbar, logo left".into(), " ".into()], vec![]], "meta/llama-3.2-90b-vision-instruct");
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|m| m.images.is_empty()));
        assert_eq!(out[0].content, "sys");
        assert!(out[1].content.starts_with("Rebuild this page"));
        assert!(out[1].content.contains("[Attached image 1 — you cannot see images, so meta/llama-3.2-90b-vision-instruct described it"));
        assert!(out[1].content.contains("- dark navbar, logo left"));
        assert!(out[1].content.contains("[Attached image 2") && out[1].content.contains("could not be described"));
        assert_eq!(out[2].content, "ok");
    }
}
