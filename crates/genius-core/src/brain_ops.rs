//! Project Brain on top of a Genius project: persistence, refresh, and the one query that gathers
//! verified source facts and — kept separate — recorded knowledge (memory, earlier changes, Git
//! history). See `brain.rs` for what each part can and cannot be trusted for.

use crate::app::Genius;
use crate::brain::{self, BrainIndex, CallInfo, Hit, Impact, IndexStats, Overview};
use crate::error::Result;
use serde::Serialize;
use serde_json::json;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Excerpt {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub kind: String,
    /// "user-stated", "verified from repository" or "agent inference".
    pub origin: String,
    pub updated: String,
    pub text: String,
    /// Why this note may no longer be true (empty = nothing contradicts it).
    pub stale: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PastChange {
    pub title: String,
    pub status: String,
    pub when: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitEntry {
    pub path: String,
    pub short: String,
    pub date: String,
    pub subject: String,
}

/// Everything the Brain knows that is relevant to one question.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainAnswer {
    pub indexed_at: String,
    pub provenance: &'static str,
    /// Verified: ranked symbol and file matches with the code around them.
    pub hits: Vec<Hit>,
    pub excerpts: Vec<Excerpt>,
    pub impact: Option<Impact>,
    /// Verified: where a symbol is defined and which files use it.
    pub callers: Option<CallInfo>,
    /// Recorded, not verified against the code today.
    pub notes: Vec<Note>,
    pub past_changes: Vec<PastChange>,
    pub git_history: Vec<GitEntry>,
}

impl Genius {
    /// Bring the index of a project up to date (only changed files are re-read) and store it.
    pub fn brain_refresh(&self, project_id: &str) -> Result<(BrainIndex, IndexStats)> {
        let ws = self.workspace(project_id)?;
        let key = brain::storage_key(project_id);
        let previous: Option<BrainIndex> = self.db.get_setting(&key)?.and_then(|v| serde_json::from_value(v).ok());
        let (ix, stats) = brain::build(&ws, previous.as_ref())?;
        if previous.is_none() || stats.scanned > 0 || stats.removed > 0 || previous.as_ref().is_some_and(|p| p.version != ix.version || p.files.len() != ix.files.len()) {
            self.db.set_setting(&key, &serde_json::to_value(&ix)?)?;
        }
        Ok((ix, stats))
    }

    pub fn brain_overview(&self, project_id: &str) -> Result<Overview> {
        Ok(brain::overview(&self.brain_refresh(project_id)?.0))
    }

    /// Verified facts for `query` and/or the files in `impact_of`, plus the recorded knowledge about
    /// the same files. `allow` hides paths the caller must not see (a custom agent's scope).
    pub async fn brain_ask(&self, project_id: &str, query: &str, impact_of: &[String], allow: &(dyn Fn(&str) -> bool + Sync)) -> Result<BrainAnswer> {
        self.brain_ask_with(project_id, query, impact_of, None, allow).await
    }

    /// [`brain_ask`](Self::brain_ask) plus `callers_of`: the definition and the users of one symbol.
    pub async fn brain_ask_with(&self, project_id: &str, query: &str, impact_of: &[String], callers_of: Option<&str>, allow: &(dyn Fn(&str) -> bool + Sync)) -> Result<BrainAnswer> {
        let (ix, _) = self.brain_refresh(project_id)?;
        let ws = self.workspace(project_id)?;
        let mut hits: Vec<Hit> = if query.trim().is_empty() { vec![] } else { brain::search(&ix, query, 40) };
        hits.retain(|h| allow(&h.path));
        hits.truncate(12);

        let mut excerpts = Vec::new();
        for h in hits.iter().filter(|h| h.kind != "file").take(5) {
            if let Ok(text) = ws.read_text(&h.path) {
                let lines: Vec<&str> = text.lines().collect();
                let from = h.line.saturating_sub(1);
                let body: Vec<String> = lines.iter().enumerate().skip(from).take(8).map(|(i, l)| format!("{:>4}  {}", i + 1, l.chars().take(160).collect::<String>())).collect();
                excerpts.push(Excerpt { path: h.path.clone(), line: h.line, text: body.join("\n") });
            }
        }

        let mut impact = None;
        if !impact_of.is_empty() {
            let allowed: Vec<String> = impact_of.iter().filter(|p| allow(p)).cloned().collect();
            let mut i = brain::impact(&ix, &allowed, 4, 60);
            i.affected.retain(|a| allow(&a.path));
            i.tests.retain(|t| allow(t));
            impact = Some(i);
        }

        let callers = callers_of.filter(|s| !s.trim().is_empty()).map(|s| {
            let mut c = brain::callers(&ix, s, 40);
            c.definitions.retain(|d| allow(&d.path));
            c.callers.retain(|x| allow(&x.path));
            c
        });

        // Files the answer is about: hits and the changed files.
        let mut about: Vec<String> = hits.iter().map(|h| h.path.clone()).chain(impact.iter().flat_map(|i| i.changed.clone())).chain(callers.iter().flat_map(|c| c.definitions.iter().map(|d| d.path.clone()))).collect();
        about.sort();
        about.dedup();
        about.truncate(8);

        // Recorded knowledge: memory notes that mention the query's words or the files concerned.
        let toks: Vec<String> = query.split(|c: char| !c.is_alphanumeric() && c != '_').filter(|t| t.len() >= 3).map(|t| t.to_lowercase()).collect();
        let now = chrono::Utc::now();
        let mut notes = Vec::new();
        for m in self.db.list_memories(project_id)? {
            let lower = m.content.to_lowercase();
            let relevant = toks.iter().any(|t| lower.contains(t)) || about.iter().any(|p| m.content.contains(p.as_str())) || (toks.is_empty() && about.is_empty());
            if !relevant {
                continue;
            }
            let origin = match m.source.as_str() {
                "user" => "user-stated",
                "repo" => "verified from repository",
                _ => "agent inference",
            };
            let stale = brain::stale_reasons(&ix, &m.content, &m.source, &m.updated_at, now);
            notes.push(Note { kind: m.kind, origin: origin.into(), updated: m.updated_at.chars().take(10).collect(), text: m.content.chars().take(300).collect(), stale });
            if notes.len() >= 8 {
                break;
            }
        }

        // Earlier changes that touched these files.
        let mut past_changes = Vec::new();
        if !about.is_empty() {
            for c in self.list_changesets(project_id)? {
                let touched: Vec<String> = c.files.iter().map(|f| f.path.clone()).filter(|p| about.contains(p)).collect();
                if !touched.is_empty() {
                    past_changes.push(PastChange { title: c.title, status: c.status, when: c.created_at.chars().take(10).collect(), files: touched });
                    if past_changes.len() >= 6 {
                        break;
                    }
                }
            }
        }

        // Git history of the concerned files (empty without a repository).
        let mut git_history = Vec::new();
        if let (false, Ok(git)) = (about.is_empty(), crate::git::Git::detect()) {
            for p in about.iter().take(4) {
                for c in git.file_log(ws.root(), p, 3).await.unwrap_or_default() {
                    git_history.push(GitEntry { path: p.clone(), short: c.short, date: c.date.chars().take(10).collect(), subject: c.subject.chars().take(120).collect() });
                }
            }
        }

        Ok(BrainAnswer { indexed_at: ix.indexed_at.clone(), provenance: brain::PROVENANCE, hits, excerpts, impact, callers, notes, past_changes, git_history })
    }
}

impl BrainAnswer {
    /// Text for a model: the verified part and the recorded part are labelled differently on purpose.
    pub fn render(&self, max_chars: usize) -> String {
        let mut s = format!("PROJECT BRAIN — VERIFIED SOURCE FACTS (file scan at {}; pattern-based, not a compiler)\n", &self.indexed_at[..19.min(self.indexed_at.len())]);
        if self.hits.is_empty() && self.impact.is_none() && self.callers.is_none() {
            s.push_str("No matching symbols or files.\n");
        }
        for h in &self.hits {
            s.push_str(&format!("- {} {} — {}:{}\n", h.kind, h.name, h.path, h.line));
        }
        for e in &self.excerpts {
            s.push_str(&format!("\n{}:{}\n{}\n", e.path, e.line, e.text));
        }
        if let Some(i) = &self.impact {
            s.push_str(&format!("\nCHANGE IMPACT of {} (import-graph reachability — a lower bound)\n", i.changed.join(", ")));
            if !i.unknown.is_empty() {
                s.push_str(&format!("not in the index (new, ignored or unsupported language): {}\n", i.unknown.join(", ")));
            }
            for a in &i.affected {
                s.push_str(&format!("- {} (depth {}, via {})\n", a.path, a.depth, a.via));
            }
            if i.affected.is_empty() && !i.changed.is_empty() {
                s.push_str("- no other scanned file imports these files\n");
            }
            if i.truncated {
                s.push_str("- (list truncated)\n");
            }
            if !i.tests.is_empty() {
                s.push_str(&format!("Tests to run/check: {}\n", i.tests.join(", ")));
            }
        }
        if let Some(c) = &self.callers {
            s.push_str(&format!("\nSYMBOL {} — definition and users ({})\n", c.symbol, c.note));
            if c.definitions.is_empty() {
                s.push_str("- no definition found in the scanned files (it may come from a library or be generated)\n");
            }
            for d in &c.definitions {
                s.push_str(&format!("- defined as {} at {}:{}\n", d.kind, d.path, d.line));
            }
            if c.ambiguous {
                s.push_str("- the name is defined in several files: matches below may belong to another symbol\n");
            }
            for x in &c.callers {
                s.push_str(&format!("- used in {}{}\n", x.path, if x.imports_definition { " (imports the definition)" } else { " (same name only)" }));
            }
            if c.callers.is_empty() {
                s.push_str("- no other scanned file calls it\n");
            }
            if c.truncated {
                s.push_str("- (list truncated)\n");
            }
        }
        if !self.notes.is_empty() || !self.past_changes.is_empty() || !self.git_history.is_empty() {
            s.push_str("\nRECORDED KNOWLEDGE — history, not verified against the code today\n");
            for n in &self.notes {
                s.push_str(&format!("- [{}; {}; {}] {}{}\n", n.kind, n.origin, n.updated, n.text, if n.stale.is_empty() { String::new() } else { format!("  ⚠ possibly stale: {}", n.stale.join("; ")) }));
            }
            for c in &self.past_changes {
                s.push_str(&format!("- change \"{}\" ({}, {}): {}\n", c.title, c.status, c.when, c.files.join(", ")));
            }
            for g in &self.git_history {
                s.push_str(&format!("- git {} {} {}: {}\n", g.short, g.date, g.path, g.subject));
            }
        }
        s.chars().take(max_chars).collect()
    }
}

impl Genius {
    /// The overview a Code task starts with (empty text for projects without supported source files).
    pub fn brain_prompt(&self, project_id: &str) -> String {
        match self.brain_refresh(project_id) {
            Ok((ix, _)) if !ix.files.is_empty() => brain::overview(&ix).render(1800),
            Ok(_) => String::new(),
            Err(e) => {
                log::warn!("project brain unavailable: {e}");
                String::new()
            }
        }
    }

    /// For events: a summary of what the last refresh did.
    pub fn brain_event(&self, project_id: &str, stats: &IndexStats) {
        self.bus.record(Some(project_id), None, crate::events::EventKind::Task, crate::events::EventStatus::Completed, "Project Brain updated", json!({ "scanned": stats.scanned, "reused": stats.reused, "removed": stats.removed }));
    }
}
