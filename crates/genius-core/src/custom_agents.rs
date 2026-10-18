//! Custom agents: user-defined specialists that run on the real agent loop (`agent.rs`).
//!
//! A [`CustomAgent`] is data (instructions, tools, scope, budget, approval policy, …) stored in
//! SQLite. When it runs, it is resolved into an [`AgentProfile`] that the loop enforces:
//! every tool call, path and model call is checked against it, so the limits are real and not
//! only prompt text. Built-in agents are seeded from [`templates`] and can be duplicated but not
//! deleted. Statuses in [`AgentRuntime`] come from real state transitions only.

use crate::app::Genius;
use crate::db::{now, Db};
use crate::error::{GeniusError, Result};
use crate::settings::Effort;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const AGENT_STATUS_CHANNEL: &str = "genius://agent-status";
pub const AGENT_LOG_CHANNEL: &str = "genius://agent-log";

const WRITE_TOOLS: &[&str] = &["write_file", "edit_file", "write_files", "delete_file", "add_dependency"];
const WEB_TOOLS: &[&str] = &["web_search", "fetch_url"];
/// Never granted to a custom agent: a delegated subagent would run outside the agent's limits.
const NEVER_TOOLS: &[&str] = &["delegate"];
const MAX_LOGS_PER_AGENT: i64 = 2000;

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentScope {
    /// "current": only the project the agent is used in; "any": usable in any project.
    pub projects: String,
    /// Allowed project-relative path prefixes ([] = the whole project).
    pub paths: Vec<String>,
}

impl Default for AgentScope {
    fn default() -> Self {
        Self { projects: "current".into(), paths: vec![] }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Budget {
    pub max_steps: u32,
    pub max_minutes: u32,
    pub max_tokens: u64,
    pub max_model_calls: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self { max_steps: 24, max_minutes: 20, max_tokens: 400_000, max_model_calls: 40 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CustomAgent {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub role: String,
    pub description: String,
    pub instructions: String,
    pub specialization: Vec<String>,
    pub knowledge_sources: Vec<String>,
    /// "auto" | "max" | "manual"
    pub model_policy: String,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub scope: AgentScope,
    pub network: bool,
    /// "none" | "project" | "agent"
    pub memory_policy: String,
    pub budget: Budget,
    pub retry_limit: u32,
    /// "ask" | "auto" | "plan_only"
    pub approval: String,
    pub acceptance_criteria: String,
    pub output_requirements: String,
    pub builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Tools a new agent gets by default: the read-only subset that needs no network.
pub fn default_tools() -> Vec<String> {
    crate::agent::tool_specs().iter().filter(|t| t.read_only && !WEB_TOOLS.contains(&t.name) && !NEVER_TOOLS.contains(&t.name)).map(|t| t.name.to_string()).collect()
}

impl Default for CustomAgent {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            avatar: "spark".into(),
            role: String::new(),
            description: String::new(),
            instructions: String::new(),
            specialization: vec![],
            knowledge_sources: vec![],
            model_policy: "auto".into(),
            model: None,
            tools: default_tools(),
            scope: AgentScope::default(),
            network: false,
            memory_policy: "project".into(),
            budget: Budget::default(),
            retry_limit: 1,
            approval: "ask".into(),
            acceptance_criteria: String::new(),
            output_requirements: String::new(),
            builtin: false,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }
}

fn clean_list(v: &mut Vec<String>, max_items: usize, max_len: usize, what: &str) -> Result<()> {
    let mut out: Vec<String> = Vec::new();
    for s in v.iter() {
        let s = s.trim().to_string();
        if s.is_empty() || out.contains(&s) {
            continue;
        }
        if s.chars().count() > max_len {
            return Err(GeniusError::Validation(format!("{what}: '{}…' is longer than {max_len} characters", s.chars().take(20).collect::<String>())));
        }
        out.push(s);
    }
    if out.len() > max_items {
        return Err(GeniusError::Validation(format!("{what}: at most {max_items} entries")));
    }
    *v = out;
    Ok(())
}

impl CustomAgent {
    /// Normalise and check every field. Called by `save_agent`; nothing invalid is stored.
    pub fn validate(&mut self) -> Result<()> {
        let v = |m: &str| Err(GeniusError::Validation(m.to_string()));
        self.name = self.name.trim().to_string();
        if self.name.is_empty() || self.name.chars().count() > 60 {
            return v("The agent needs a name of 1-60 characters.");
        }
        self.avatar = self.avatar.trim().to_string();
        if self.avatar.is_empty() {
            self.avatar = "spark".into();
        }
        if self.avatar.chars().count() > 24 {
            return v("The avatar must be an emoji or a short preset key.");
        }
        self.role = self.role.trim().to_string();
        self.description = self.description.trim().to_string();
        self.instructions = self.instructions.trim().to_string();
        self.acceptance_criteria = self.acceptance_criteria.trim().to_string();
        self.output_requirements = self.output_requirements.trim().to_string();
        if self.role.chars().count() > 80 {
            return v("The role is limited to 80 characters.");
        }
        if self.description.chars().count() > 600 {
            return v("The description is limited to 600 characters.");
        }
        if self.instructions.chars().count() > 8000 {
            return v("The instructions are limited to 8000 characters.");
        }
        if self.acceptance_criteria.chars().count() > 2000 || self.output_requirements.chars().count() > 2000 {
            return v("Acceptance criteria and output requirements are limited to 2000 characters each.");
        }
        clean_list(&mut self.specialization, 12, 40, "specialization")?;
        clean_list(&mut self.knowledge_sources, 12, 300, "knowledge sources")?;
        for s in &self.knowledge_sources {
            if s.starts_with("http://") {
                return v("Knowledge sources on the web must use https://.");
            }
            if !s.starts_with("https://") {
                crate::workspace::normalize_rel(s).map_err(|_| GeniusError::Validation(format!("Knowledge source '{s}' must be a project-relative path or an https:// URL.")))?;
            }
        }
        if !["auto", "max", "manual"].contains(&self.model_policy.as_str()) {
            return v("Model policy must be auto, max or manual.");
        }
        self.model = self.model.take().map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
        if self.model_policy == "manual" && self.model.is_none() {
            return v("Choose a model for a manual model policy.");
        }
        if self.model.as_ref().is_some_and(|m| m.len() > 200) {
            return v("The model id is too long.");
        }
        clean_list(&mut self.tools, 40, 40, "tools")?;
        let known: Vec<&str> = crate::agent::tool_specs().iter().map(|t| t.name).collect();
        for t in &self.tools {
            if NEVER_TOOLS.contains(&t.as_str()) {
                return Err(GeniusError::Validation(format!("The '{t}' tool is not available to custom agents (a subagent would run outside this agent's limits).")));
            }
            if !known.contains(&t.as_str()) {
                return Err(GeniusError::Validation(format!("Unknown tool '{t}'. Available: {}", known.join(", "))));
            }
        }
        if !["current", "any"].contains(&self.scope.projects.as_str()) {
            return v("Scope projects must be current or any.");
        }
        let mut paths = Vec::new();
        for p in &self.scope.paths {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            let norm = crate::workspace::to_slash(&crate::workspace::normalize_rel(p).map_err(|_| GeniusError::Validation(format!("Scope path '{p}' must be a project-relative folder.")))?);
            if !paths.contains(&norm) {
                paths.push(norm);
            }
        }
        if paths.len() > 20 {
            return v("At most 20 scope paths.");
        }
        self.scope.paths = paths;
        if !["none", "project", "agent"].contains(&self.memory_policy.as_str()) {
            return v("Memory policy must be none, project or agent.");
        }
        if !["ask", "auto", "plan_only"].contains(&self.approval.as_str()) {
            return v("Approval must be ask, auto or plan_only.");
        }
        let b = &self.budget;
        if !(1..=200).contains(&b.max_steps) || !(1..=240).contains(&b.max_minutes) || !(1_000..=5_000_000).contains(&b.max_tokens) || !(1..=500).contains(&b.max_model_calls) {
            return v("Budget limits out of range: steps 1-200, minutes 1-240, tokens 1,000-5,000,000, model calls 1-500.");
        }
        if self.retry_limit > 5 {
            return v("Retry limit is at most 5.");
        }
        Ok(())
    }

    /// Tools the agent may actually use: its `tools` ∩ the safe set for its policies.
    pub fn effective_tools(&self) -> BTreeSet<String> {
        let specs = crate::agent::tool_specs();
        self.tools
            .iter()
            .filter(|t| specs.iter().any(|s| s.name == t.as_str()))
            .filter(|t| !NEVER_TOOLS.contains(&t.as_str()))
            .filter(|t| self.network || !WEB_TOOLS.contains(&t.as_str()))
            .filter(|t| self.approval != "plan_only" || specs.iter().any(|s| s.name == t.as_str() && s.read_only))
            .filter(|t| t.as_str() != "remember" || self.memory_policy != "none")
            .cloned()
            .collect()
    }

    /// May this agent change files at all (and therefore needs an isolated workspace)?
    pub fn can_edit(&self) -> bool {
        self.approval != "plan_only" && self.effective_tools().iter().any(|t| WRITE_TOOLS.contains(&t.as_str()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplate {
    pub key: String,
    pub name: String,
    pub avatar: String,
    pub role: String,
    pub description: String,
    /// A CustomAgent without id, createdAt, updatedAt and builtin.
    pub agent: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentUsage {
    pub tokens: u64,
    pub calls: u64,
    pub tasks_done: u64,
    pub tasks_failed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntime {
    pub agent_id: String,
    /// idle | planning | working | blocked | awaiting_approval | reviewing | completed | failed | paused
    pub status: String,
    pub current_task_id: Option<String>,
    pub last_activity_at: Option<String>,
    pub usage: AgentUsage,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: String,
    pub coordinator_agent_id: String,
    pub agent_ids: Vec<String>,
    pub created_at: String,
}

impl Default for Team {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), description: String::new(), coordinator_agent_id: String::new(), agent_ids: vec![], created_at: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskLog {
    pub id: String,
    pub agent_id: String,
    pub mission_id: Option<String>,
    pub task_id: Option<String>,
    pub project_id: String,
    /// message | tool | status | result | error
    pub kind: String,
    pub text: String,
    pub data: Value,
    pub at: String,
}

// ------------------------------------------------------------------ built-in templates

fn tools_of(extra: &[&str]) -> Vec<String> {
    let mut t = default_tools();
    for e in extra {
        if !t.iter().any(|x| x == e) {
            t.push(e.to_string());
        }
    }
    t
}

fn agent(name: &str, avatar: &str, role: &str, description: &str, instructions: &str) -> CustomAgent {
    CustomAgent { name: name.into(), avatar: avatar.into(), role: role.into(), description: description.into(), instructions: instructions.into(), ..Default::default() }
}

/// (key, agent) for every built-in template. Used for the template gallery and to seed the
/// built-in agents (ids `builtin-<key>`).
pub fn template_agents() -> Vec<(&'static str, CustomAgent)> {
    let write = ["write_file", "edit_file", "write_files", "delete_file", "todo"];
    let mut designer = agent("Designer", "spark", "UI/UX designer", "Designs and builds clean, accessible, responsive interfaces.", "You are a senior product designer who also writes the code. Prioritise visual hierarchy, spacing, accessible contrast (WCAG AA), keyboard focus states and responsive layouts. Reuse the project's existing styles and components before adding new ones. Keep changes focused on presentation; do not touch business logic.");
    designer.specialization = vec!["UI".into(), "CSS".into(), "accessibility".into()];
    designer.tools = tools_of(&[&write[..], &["use_skill", "run_check"][..]].concat());
    designer.acceptance_criteria = "The UI renders without console errors, is usable at 375px and 1280px widths, and uses sufficient colour contrast.".into();

    let mut researcher = agent("Researcher", "owl", "Research analyst", "Investigates a question in the code and on the web and reports findings with sources. Never edits files.", "You investigate and report. Cite file paths with line numbers for code facts and URLs for web facts. Separate verified facts from assumptions. Never modify files.");
    researcher.network = true;
    researcher.approval = "plan_only".into();
    researcher.tools = tools_of(&["web_search", "fetch_url"]);
    researcher.output_requirements = "A concise report: findings, evidence (paths or URLs), open questions.".into();

    let mut developer = agent("Developer", "robot", "Software developer", "Implements features and fixes bugs with small, verified changes.", "You are a careful senior developer. Read the relevant code first, make the smallest correct change, keep the project's conventions, and run the type check/build before you finish. Add or update tests when behaviour changes.");
    developer.specialization = vec!["TypeScript".into(), "React".into()];
    developer.tools = tools_of(&[&write[..], &["run_check", "add_dependency", "remember", "use_skill"][..]].concat());
    developer.acceptance_criteria = "The change compiles (type check and build pass) and does what the task asked, without unrelated edits.".into();

    let mut tester = agent("Tester", "leaf", "QA engineer", "Writes and runs tests, finds gaps and reports failures honestly.", "You are a QA engineer. Find untested behaviour, write focused Vitest tests (no snapshot spam), run them with the real test runner, and report exactly what passed and failed. Never weaken or delete a test to make it pass.");
    tester.tools = tools_of(&[&write[..], &["run_check"][..]].concat());
    tester.acceptance_criteria = "New tests run, and the report states the real pass/fail result of the test command.".into();

    let mut security = agent("Security reviewer", "shield", "Security reviewer", "Reviews code for vulnerabilities and unsafe patterns. Read-only.", "You review for security: injection, XSS, unsafe HTML, secrets in code, insecure dependencies, missing validation, over-broad permissions. Report each finding with severity (high/medium/low), file:line, why it matters and a concrete fix. If you find nothing, say so and list what you checked. Never modify files.");
    security.approval = "plan_only".into();
    security.output_requirements = "Findings grouped by severity, each with file:line and a fix; or 'No issues found' with the list of what was checked.".into();

    let mut coordinator = agent("Coordinator", "compass", "Mission coordinator", "Breaks a goal into a dependency-aware task graph and assigns it to the team.", "You plan work for a team of specialist agents. Produce small, concrete tasks with clear deliverables, assign each to the best-suited agent, and mark real dependencies only. Prefer parallel tasks that touch different files.");
    coordinator.approval = "plan_only".into();

    let mut general = agent("General assistant", "star", "General assistant", "A capable all-rounder for everyday project tasks.", "You are a helpful, careful assistant for this project. Do what the task asks, nothing more, and verify your work.");
    general.tools = tools_of(&[&write[..], &["run_check", "remember"][..]].concat());

    vec![("designer", designer), ("researcher", researcher), ("developer", developer), ("tester", tester), ("security_reviewer", security), ("coordinator", coordinator), ("general_assistant", general)]
}

pub fn templates() -> Vec<AgentTemplate> {
    template_agents()
        .into_iter()
        .map(|(key, a)| {
            let mut v = serde_json::to_value(&a).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                for k in ["id", "createdAt", "updatedAt", "builtin"] {
                    o.remove(k);
                }
            }
            AgentTemplate { key: key.into(), name: a.name.clone(), avatar: a.avatar.clone(), role: a.role.clone(), description: a.description.clone(), agent: v }
        })
        .collect()
}

pub fn builtin_id(key: &str) -> String {
    format!("builtin-{key}")
}

// ------------------------------------------------------------------ persistence

fn row_to_agent(json: String, builtin: i64, id: String, created: String, updated: String) -> CustomAgent {
    let mut a: CustomAgent = serde_json::from_str(&json).unwrap_or_default();
    a.id = id;
    a.builtin = builtin != 0;
    a.created_at = created;
    a.updated_at = updated;
    a
}

impl Db {
    pub fn list_custom_agents(&self) -> Result<Vec<CustomAgent>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, json, builtin, created_at, updated_at FROM custom_agents ORDER BY builtin DESC, created_at ASC, rowid ASC")?;
        let rows = stmt.query_map([], |r| Ok(row_to_agent(r.get(1)?, r.get(2)?, r.get(0)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_custom_agent(&self, id: &str) -> Result<CustomAgent> {
        self.conn()
            .query_row("SELECT id, json, builtin, created_at, updated_at FROM custom_agents WHERE id = ?1", [id], |r| Ok(row_to_agent(r.get(1)?, r.get(2)?, r.get(0)?, r.get(3)?, r.get(4)?)))
            .optional()?
            .ok_or_else(|| GeniusError::NotFound(format!("agent {id}")))
    }

    pub fn put_custom_agent(&self, a: &CustomAgent) -> Result<()> {
        let mut body = serde_json::to_value(a)?;
        if let Some(o) = body.as_object_mut() {
            for k in ["id", "builtin", "createdAt", "updatedAt"] {
                o.remove(k);
            }
        }
        self.conn().execute(
            "INSERT INTO custom_agents(id, name, builtin, json, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, json = excluded.json, updated_at = excluded.updated_at",
            params![a.id, a.name, a.builtin as i64, body.to_string(), a.created_at, a.updated_at],
        )?;
        Ok(())
    }

    pub fn delete_custom_agent(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM custom_agents WHERE id = ?1 AND builtin = 0", [id])?;
        Ok(())
    }

    pub fn list_teams(&self) -> Result<Vec<Team>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, description, coordinator_agent_id, agent_ids_json, created_at FROM agent_teams ORDER BY created_at ASC, rowid ASC")?;
        let rows = stmt.query_map([], |r| {
            let ids: String = r.get(4)?;
            Ok(Team { id: r.get(0)?, name: r.get(1)?, description: r.get(2)?, coordinator_agent_id: r.get(3)?, agent_ids: serde_json::from_str(&ids).unwrap_or_default(), created_at: r.get(5)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_team(&self, id: &str) -> Result<Team> {
        self.list_teams()?.into_iter().find(|t| t.id == id).ok_or_else(|| GeniusError::NotFound(format!("team {id}")))
    }

    pub fn put_team(&self, t: &Team) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_teams(id, name, description, coordinator_agent_id, agent_ids_json, created_at) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, description = excluded.description, coordinator_agent_id = excluded.coordinator_agent_id, agent_ids_json = excluded.agent_ids_json",
            params![t.id, t.name, t.description, t.coordinator_agent_id, serde_json::to_string(&t.agent_ids)?, t.created_at],
        )?;
        Ok(())
    }

    pub fn delete_team(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM agent_teams WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_agent_usage(&self, agent_id: &str) -> Result<(AgentUsage, Option<String>)> {
        Ok(self
            .conn()
            .query_row("SELECT tokens, calls, tasks_done, tasks_failed, last_activity_at FROM agent_usage WHERE agent_id = ?1", [agent_id], |r| {
                Ok((AgentUsage { tokens: r.get::<_, i64>(0)? as u64, calls: r.get::<_, i64>(1)? as u64, tasks_done: r.get::<_, i64>(2)? as u64, tasks_failed: r.get::<_, i64>(3)? as u64 }, r.get(4)?))
            })
            .optional()?
            .unwrap_or_default())
    }

    /// Add to an agent's accumulated usage.
    pub fn add_agent_usage(&self, agent_id: &str, tokens: u64, calls: u64, done: u64, failed: u64) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_usage(agent_id, tokens, calls, tasks_done, tasks_failed, last_activity_at) VALUES(?1,?2,?3,?4,?5,?6)
             ON CONFLICT(agent_id) DO UPDATE SET tokens = tokens + excluded.tokens, calls = calls + excluded.calls, tasks_done = tasks_done + excluded.tasks_done, tasks_failed = tasks_failed + excluded.tasks_failed, last_activity_at = excluded.last_activity_at",
            params![agent_id, tokens as i64, calls as i64, done as i64, failed as i64, now()],
        )?;
        Ok(())
    }

    pub fn insert_agent_log(&self, l: &AgentTaskLog) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT INTO agent_logs(id, agent_id, mission_id, task_id, project_id, kind, text, data_json, at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![l.id, l.agent_id, l.mission_id, l.task_id, l.project_id, l.kind, l.text, l.data.to_string(), l.at],
        )?;
        // keep the log bounded per agent
        conn.execute(
            "DELETE FROM agent_logs WHERE agent_id = ?1 AND rowid <= (SELECT rowid FROM agent_logs WHERE agent_id = ?1 ORDER BY rowid DESC LIMIT 1 OFFSET ?2)",
            params![l.agent_id, MAX_LOGS_PER_AGENT],
        )?;
        Ok(())
    }

    /// Newest `limit` log entries of an agent, returned oldest first.
    pub fn list_agent_logs(&self, agent_id: &str, limit: usize) -> Result<Vec<AgentTaskLog>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, agent_id, mission_id, task_id, project_id, kind, text, data_json, at FROM agent_logs WHERE agent_id = ?1 ORDER BY rowid DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![agent_id, limit as i64], |r| {
            let d: String = r.get(7)?;
            Ok(AgentTaskLog { id: r.get(0)?, agent_id: r.get(1)?, mission_id: r.get(2)?, task_id: r.get(3)?, project_id: r.get(4)?, kind: r.get(5)?, text: r.get(6)?, data: serde_json::from_str(&d).unwrap_or(Value::Null), at: r.get(8)? })
        })?;
        let mut v = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        v.reverse();
        Ok(v)
    }

    pub fn add_agent_memory(&self, agent_id: &str, project_id: &str, content: &str, provenance: &Value) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.conn().execute(
            "INSERT INTO agent_memories(id, agent_id, project_id, content, provenance_json, created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, agent_id, project_id, content, provenance.to_string(), now()],
        )?;
        Ok(id)
    }

    /// (content, provenance, created_at) of an agent's memories for a project, newest first.
    pub fn list_agent_memories(&self, agent_id: &str, project_id: &str) -> Result<Vec<(String, Value, String)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT content, provenance_json, created_at FROM agent_memories WHERE agent_id = ?1 AND project_id = ?2 ORDER BY rowid DESC LIMIT 300")?;
        let rows = stmt.query_map(params![agent_id, project_id], |r| {
            let p: String = r.get(1)?;
            Ok((r.get(0)?, serde_json::from_str(&p).unwrap_or(Value::Null), r.get(2)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Insert the built-in agents (idempotent) and, on the very first run, a starter team.
pub fn seed_builtins(db: &Db) -> Result<()> {
    let t = now();
    for (key, mut a) in template_agents() {
        let id = builtin_id(key);
        if db.get_custom_agent(&id).is_ok() {
            continue;
        }
        a.id = id;
        a.builtin = true;
        a.created_at = t.clone();
        a.updated_at = t.clone();
        db.put_custom_agent(&a)?;
    }
    if db.get_setting("agents_seeded_v1")?.is_none() {
        let team = Team {
            id: "builtin-team-starter".into(),
            name: "Starter team".into(),
            description: "A coordinator with a designer, developer, tester and security reviewer.".into(),
            coordinator_agent_id: builtin_id("coordinator"),
            agent_ids: ["coordinator", "designer", "developer", "tester", "security_reviewer"].iter().map(|k| builtin_id(k)).collect(),
            created_at: t,
        };
        db.put_team(&team)?;
        db.set_setting("agents_seeded_v1", &json!(true))?;
    }
    Ok(())
}

// ------------------------------------------------------------------ runtime profile

/// What the agent loop enforces for one run of a custom agent.
#[derive(Debug, Clone)]
pub struct AgentProfile {
    pub agent_id: String,
    pub name: String,
    pub role: String,
    /// Effective tool names (agent tools ∩ safe set).
    pub tools: BTreeSet<String>,
    pub network: bool,
    pub scope: Vec<String>,
    pub budget: Budget,
    pub approval: String,
    pub memory: String,
    pub model_override: Option<String>,
    pub effort: Effort,
    pub instructions: String,
    pub acceptance: String,
    pub output_requirements: String,
    pub knowledge_sources: Vec<String>,
    pub mission_id: Option<String>,
    /// The user's real project (the run may execute in an isolated copy of it).
    pub live_project_id: String,
    pub started: Instant,
    /// Usage of earlier attempts of the same task (the budget covers all attempts).
    pub base_usage: [u64; 4],
    pub base_steps: usize,
}

impl AgentProfile {
    pub fn from_agent(a: &CustomAgent, live_project_id: &str, mission_id: Option<&str>) -> Self {
        let effort = match a.model_policy.as_str() {
            "max" => Effort::Max,
            _ => Effort::Balanced,
        };
        Self {
            agent_id: a.id.clone(),
            name: a.name.clone(),
            role: a.role.clone(),
            tools: a.effective_tools(),
            network: a.network,
            scope: a.scope.paths.clone(),
            budget: a.budget.clone(),
            approval: a.approval.clone(),
            memory: a.memory_policy.clone(),
            model_override: if a.model_policy == "manual" { a.model.clone() } else { None },
            effort,
            instructions: a.instructions.clone(),
            acceptance: a.acceptance_criteria.clone(),
            output_requirements: a.output_requirements.clone(),
            knowledge_sources: a.knowledge_sources.clone(),
            mission_id: mission_id.map(String::from),
            live_project_id: live_project_id.into(),
            started: Instant::now(),
            base_usage: [0; 4],
            base_steps: 0,
        }
    }

    pub fn allows_tool(&self, name: &str) -> bool {
        self.tools.contains(name) && (self.network || !WEB_TOOLS.contains(&name))
    }

    /// Writes (tools or file blocks) are possible at all.
    pub fn can_write(&self) -> bool {
        self.approval != "plan_only" && self.tools.iter().any(|t| WRITE_TOOLS.contains(&t.as_str()))
    }

    pub fn scoped(&self) -> bool {
        !self.scope.is_empty()
    }

    /// Is `path` (project-relative, as the model wrote it) inside the agent's scope?
    pub fn path_ok(&self, path: &str) -> bool {
        let Ok(norm) = crate::workspace::normalize_rel(path) else { return false };
        if self.scope.is_empty() {
            return true;
        }
        let p = crate::workspace::to_slash(&norm);
        self.scope.iter().any(|s| p == *s || p.starts_with(&format!("{s}/")))
    }

    pub fn scope_text(&self) -> String {
        if self.scope.is_empty() {
            "the whole project".into()
        } else {
            self.scope.iter().map(|s| format!("{s}/")).collect::<Vec<_>>().join(", ")
        }
    }

    /// Why a tool call is refused for this agent (None: permitted), before it runs.
    pub fn deny_reason(&self, name: &str, args: &Value) -> Option<String> {
        if !self.tools.contains(name) {
            if !self.network && WEB_TOOLS.contains(&name) {
                return Some(format!("'{name}' is refused: network access is disabled for the agent \"{}\".", self.name));
            }
            if self.approval == "plan_only" && WRITE_TOOLS.contains(&name) {
                return Some(format!("'{name}' is refused: the agent \"{}\" has the plan_only approval policy and never edits files. Describe the change in your answer instead.", self.name));
            }
            return Some(format!("'{name}' is not permitted for the agent \"{}\". Permitted tools: {}.", self.name, self.tools.iter().cloned().collect::<Vec<_>>().join(", ")));
        }
        if !self.network && WEB_TOOLS.contains(&name) {
            return Some(format!("'{name}' is refused: network access is disabled for the agent \"{}\".", self.name));
        }
        if self.approval == "plan_only" && WRITE_TOOLS.contains(&name) {
            return Some(format!("'{name}' is refused: the agent \"{}\" has the plan_only approval policy and never edits files.", self.name));
        }
        // Paths in the call (after the same normalisation the tools use).
        let (_, a) = crate::toolcall::normalize(name, args.clone());
        let mut paths: Vec<String> = Vec::new();
        let mut take = |v: Option<&Value>| {
            if let Some(Value::String(s)) = v {
                paths.push(s.clone());
            }
        };
        match name {
            "write_file" | "edit_file" | "delete_file" | "read_file" | "git_diff" => {
                take(a.get("path"));
                if let Some(Value::Array(ps)) = a.get("paths") {
                    paths.extend(ps.iter().filter_map(|p| p.as_str().map(String::from)));
                }
            }
            "write_files" => {
                if let Some(Value::Array(fs)) = a.get("files") {
                    paths.extend(fs.iter().filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(String::from)));
                }
            }
            "add_dependency" => paths.push("package.json".into()),
            "list_files" | "search" => {
                // results are filtered to the scope; an explicit prefix outside it is refused
                if let Some(Value::String(p)) = a.get("prefix") {
                    let p = p.trim().trim_start_matches("./").trim_end_matches('/');
                    if !p.is_empty() && self.scoped() && !self.path_ok(p) && !self.scope.iter().any(|s| s.starts_with(&format!("{p}/"))) {
                        paths.push(p.to_string());
                    }
                }
            }
            _ => {}
        }
        let bad: Vec<&String> = paths.iter().filter(|p| !self.path_ok(p)).collect();
        if !bad.is_empty() {
            return Some(format!("'{name}' is refused: {} outside the scope of the agent \"{}\" (allowed: {}).", bad.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", ") + if bad.len() == 1 { " is" } else { " are" }, self.name, self.scope_text()));
        }
        None
    }

    /// The budget limit that is used up, if any: `used_turns` model turns of this run so far,
    /// `usage` = [prompt, completion, total tokens, model calls] of this run.
    pub fn exhausted(&self, used_turns: usize, usage: &[u64; 4]) -> Option<String> {
        let steps = used_turns + self.base_steps;
        if steps >= self.budget.max_steps as usize {
            return Some(format!("maxSteps ({}) reached", self.budget.max_steps));
        }
        let mins = self.started.elapsed();
        if mins >= std::time::Duration::from_secs(self.budget.max_minutes as u64 * 60) {
            return Some(format!("maxMinutes ({}) reached", self.budget.max_minutes));
        }
        let tokens = usage[2] + self.base_usage[2];
        if tokens >= self.budget.max_tokens {
            return Some(format!("maxTokens ({}) reached after {} tokens", self.budget.max_tokens, tokens));
        }
        let calls = usage[3] + self.base_usage[3];
        if calls >= self.budget.max_model_calls as u64 {
            return Some(format!("maxModelCalls ({}) reached", self.budget.max_model_calls));
        }
        None
    }

    /// Deadline for the wall-clock budget.
    pub fn deadline(&self) -> Instant {
        self.started + std::time::Duration::from_secs(self.budget.max_minutes as u64 * 60)
    }
}

// ------------------------------------------------------------------ in-memory runtime state

#[derive(Debug, Clone, Default)]
pub(crate) struct RuntimeState {
    pub status: String,
    pub current_task_id: Option<String>,
    pub last_activity_at: Option<String>,
}

/// In-memory state of the agents/missions runtime (statuses, running tasks, locks). Everything
/// durable lives in SQLite; this only holds what is true right now in this process.
#[derive(Default)]
pub struct AgentsRuntime {
    pub(crate) state: Mutex<HashMap<String, RuntimeState>>,
    /// Agents currently executing a task (an agent works on one task at a time).
    pub(crate) busy: Mutex<HashSet<String>>,
    pub(crate) missions: Mutex<HashMap<String, crate::missions::MissionCtl>>,
    pub(crate) integrate: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Serialises read-modify-write of mission and task rows.
    pub(crate) rows: Mutex<()>,
}

impl Genius {
    // -------------------------------------------------------------- agents CRUD

    pub fn list_agents(&self) -> Result<Vec<CustomAgent>> {
        self.db.list_custom_agents()
    }

    pub fn get_agent(&self, id: &str) -> Result<CustomAgent> {
        self.db.get_custom_agent(id)
    }

    /// Create (no id, or an unknown id) or update an agent. Built-in agents can be edited but
    /// stay built-in.
    pub fn save_agent(&self, mut a: CustomAgent) -> Result<CustomAgent> {
        a.validate()?;
        let t = now();
        match if a.id.is_empty() { None } else { self.db.get_custom_agent(&a.id).ok() } {
            Some(old) => {
                a.builtin = old.builtin;
                a.created_at = old.created_at;
            }
            None => {
                if a.id.is_empty() || a.id.starts_with("builtin-") {
                    a.id = uuid::Uuid::new_v4().to_string();
                }
                a.builtin = false;
                a.created_at = t.clone();
            }
        }
        a.updated_at = t;
        self.db.put_custom_agent(&a)?;
        self.bus.record(None, None, crate::events::EventKind::Task, crate::events::EventStatus::Completed, format!("Saved agent {}", a.name), json!({ "agentId": a.id }));
        Ok(a)
    }

    pub fn duplicate_agent(&self, id: &str) -> Result<CustomAgent> {
        let mut a = self.db.get_custom_agent(id)?;
        a.id = String::new();
        a.name = format!("{} (copy)", a.name.chars().take(52).collect::<String>());
        a.builtin = false;
        self.save_agent(a)
    }

    pub fn delete_agent(&self, id: &str) -> Result<()> {
        let a = self.db.get_custom_agent(id)?;
        if a.builtin {
            return Err(GeniusError::Validation("Built-in agents cannot be deleted. Duplicate one to make your own.".into()));
        }
        if self.agents.busy.lock().unwrap().contains(id) {
            return Err(GeniusError::Conflict(format!("{} is working on a task right now. Cancel the task first.", a.name)));
        }
        for t in self.db.list_teams()? {
            if t.coordinator_agent_id == id {
                return Err(GeniusError::Conflict(format!("{} is the coordinator of the team \"{}\". Choose another coordinator or delete the team first.", a.name, t.name)));
            }
        }
        for mut t in self.db.list_teams()? {
            if t.agent_ids.iter().any(|x| x == id) {
                t.agent_ids.retain(|x| x != id);
                self.db.put_team(&t)?;
            }
        }
        self.db.delete_custom_agent(id)?;
        self.agents.state.lock().unwrap().remove(id);
        Ok(())
    }

    pub fn agent_templates(&self) -> Vec<AgentTemplate> {
        templates()
    }

    // -------------------------------------------------------------- teams

    pub fn list_teams(&self) -> Result<Vec<Team>> {
        self.db.list_teams()
    }

    pub fn save_team(&self, mut t: Team) -> Result<Team> {
        t.name = t.name.trim().to_string();
        if t.name.is_empty() || t.name.chars().count() > 60 {
            return Err(GeniusError::Validation("The team needs a name of 1-60 characters.".into()));
        }
        t.description = t.description.trim().chars().take(600).collect();
        let mut ids: Vec<String> = Vec::new();
        for id in t.agent_ids.iter().chain(std::iter::once(&t.coordinator_agent_id)) {
            if id.is_empty() {
                continue;
            }
            self.db.get_custom_agent(id).map_err(|_| GeniusError::Validation(format!("Agent {id} does not exist.")))?;
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        if t.coordinator_agent_id.is_empty() {
            return Err(GeniusError::Validation("A team needs a coordinator agent.".into()));
        }
        if ids.len() > 12 {
            return Err(GeniusError::Validation("A team has at most 12 agents.".into()));
        }
        t.agent_ids = ids;
        if t.id.is_empty() || self.db.get_team(&t.id).is_err() {
            if t.id.is_empty() {
                t.id = uuid::Uuid::new_v4().to_string();
            }
            t.created_at = now();
        } else {
            t.created_at = self.db.get_team(&t.id)?.created_at;
        }
        self.db.put_team(&t)?;
        Ok(t)
    }

    pub fn delete_team(&self, id: &str) -> Result<()> {
        self.db.get_team(id)?;
        self.db.delete_team(id)
    }

    // -------------------------------------------------------------- runtime / logs

    pub fn agent_runtimes(&self) -> Result<Vec<AgentRuntime>> {
        self.db.list_custom_agents()?.iter().map(|a| self.runtime_of(&a.id)).collect()
    }

    pub(crate) fn runtime_of(&self, agent_id: &str) -> Result<AgentRuntime> {
        let (usage, last_db) = self.db.get_agent_usage(agent_id)?;
        let st = self.agents.state.lock().unwrap().get(agent_id).cloned().unwrap_or_default();
        Ok(AgentRuntime {
            agent_id: agent_id.into(),
            status: if st.status.is_empty() { "idle".into() } else { st.status },
            current_task_id: st.current_task_id,
            last_activity_at: st.last_activity_at.or(last_db),
            usage,
        })
    }

    /// A real transition of an agent's state: stored, logged and pushed to the UI.
    pub(crate) fn set_agent_status(&self, agent_id: &str, status: &str, task_id: Option<&str>, project_id: &str, mission_id: Option<&str>, note: &str) {
        {
            let mut all = self.agents.state.lock().unwrap();
            let st = all.entry(agent_id.to_string()).or_default();
            if st.status == status && st.current_task_id.as_deref() == task_id {
                return;
            }
            st.status = status.into();
            st.current_task_id = task_id.map(String::from);
            st.last_activity_at = Some(now());
        }
        self.agent_log(agent_id, mission_id, task_id, project_id, "status", &format!("{status}{}", if note.is_empty() { String::new() } else { format!(": {note}") }), json!({ "status": status }));
        if let Ok(rt) = self.runtime_of(agent_id) {
            self.bus.push(AGENT_STATUS_CHANNEL, serde_json::to_value(rt).unwrap_or(Value::Null));
        }
    }

    /// Record one log entry (secrets redacted) and push it to the UI.
    pub(crate) fn agent_log(&self, agent_id: &str, mission_id: Option<&str>, task_id: Option<&str>, project_id: &str, kind: &str, text: &str, data: Value) {
        let log = AgentTaskLog {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent_id.into(),
            mission_id: mission_id.map(String::from),
            task_id: task_id.map(String::from),
            project_id: project_id.into(),
            kind: kind.into(),
            text: crate::redact::redact(&text.chars().take(4000).collect::<String>()),
            data: crate::events::redact_value(data),
            at: now(),
        };
        if self.db.insert_agent_log(&log).is_ok() {
            self.bus.push(AGENT_LOG_CHANNEL, serde_json::to_value(&log).unwrap_or(Value::Null));
        }
    }

    pub fn agent_logs(&self, agent_id: &str, limit: Option<usize>) -> Result<Vec<AgentTaskLog>> {
        self.db.get_custom_agent(agent_id)?;
        self.db.list_agent_logs(agent_id, limit.unwrap_or(200).clamp(1, 1000))
    }

    /// Memories of an agent relevant to `task` (keyword overlap, newest first), at most `n`.
    pub(crate) fn relevant_agent_memories(&self, agent_id: &str, project_id: &str, task: &str, n: usize) -> Vec<(String, Value)> {
        let words: Vec<String> = task.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 3).map(String::from).collect();
        let mut scored: Vec<(usize, usize, String, Value)> = self
            .db
            .list_agent_memories(agent_id, project_id)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, (c, p, _))| {
                let l = c.to_lowercase();
                (words.iter().filter(|w| l.contains(w.as_str())).count(), i, c, p)
            })
            .filter(|(s, _, _, _)| *s > 0)
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(n).map(|(_, _, c, p)| (c, p)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(f: impl FnOnce(&mut CustomAgent)) -> CustomAgent {
        let mut a = CustomAgent { name: "A".into(), ..Default::default() };
        f(&mut a);
        a
    }

    #[test]
    fn validation_rejects_bad_input() {
        assert!(agent(|a| a.name = " ".into()).validate().is_err());
        assert!(agent(|a| a.tools = vec!["rm_rf".into()]).validate().is_err());
        assert!(agent(|a| a.tools = vec!["delegate".into()]).validate().is_err());
        assert!(agent(|a| a.model_policy = "manual".into()).validate().is_err());
        assert!(agent(|a| a.scope.paths = vec!["../etc".into()]).validate().is_err());
        assert!(agent(|a| a.scope.paths = vec!["/abs".into()]).validate().is_err());
        assert!(agent(|a| a.budget.max_steps = 0).validate().is_err());
        assert!(agent(|a| a.approval = "yolo".into()).validate().is_err());
        assert!(agent(|a| a.knowledge_sources = vec!["http://x.y".into()]).validate().is_err());
        let mut ok = agent(|a| {
            a.scope.paths = vec!["./src/".into(), "src".into()];
            a.knowledge_sources = vec!["docs/spec.md".into(), "https://example.com/a".into()];
        });
        ok.validate().unwrap();
        assert_eq!(ok.scope.paths, vec!["src".to_string()]);
    }

    #[test]
    fn effective_tools_are_the_intersection_with_the_safe_set() {
        let a = agent(|a| {
            a.tools = vec!["read_file".into(), "write_file".into(), "web_search".into(), "run_check".into(), "remember".into()];
            a.network = false;
            a.approval = "plan_only".into();
            a.memory_policy = "none".into();
        });
        assert_eq!(a.effective_tools().into_iter().collect::<Vec<_>>(), vec!["read_file".to_string()]);
        assert!(!a.can_edit());
        let b = agent(|a| {
            a.tools = vec!["read_file".into(), "write_file".into(), "web_search".into()];
            a.network = true;
        });
        assert!(b.effective_tools().contains("web_search") && b.can_edit());
    }

    #[test]
    fn profile_denies_tools_paths_and_network() {
        let a = agent(|a| {
            a.tools = vec!["read_file".into(), "write_file".into(), "list_files".into(), "web_search".into()];
            a.scope.paths = vec!["src/ui".into()];
        });
        let p = AgentProfile::from_agent(&a, "p", None);
        assert!(p.deny_reason("read_file", &json!({ "path": "src/ui/a.tsx" })).is_none());
        assert!(p.deny_reason("read_file", &json!({ "path": "src/ui/../secret.ts" })).is_some());
        assert!(p.deny_reason("read_file", &json!({ "path": "src/other/a.tsx" })).unwrap().contains("outside the scope"));
        assert!(p.deny_reason("write_file", &json!({ "path": "package.json", "content": "x" })).is_some());
        assert!(p.deny_reason("write_file", &json!({ "path": "src/ui/a.tsx", "content": "x" })).is_none());
        assert!(p.deny_reason("web_search", &json!({ "query": "x" })).unwrap().contains("network"));
        assert!(p.deny_reason("run_check", &json!({})).unwrap().contains("not permitted"));
        assert!(p.deny_reason("list_files", &json!({ "prefix": "src" })).is_none(), "a parent of the scope may be listed (results are filtered)");
        assert!(p.deny_reason("list_files", &json!({ "prefix": "docs" })).is_some());
        assert!(p.path_ok("src/ui") && p.path_ok("./src/ui/x/y.ts") && !p.path_ok("src/uix/a"));
    }

    #[test]
    fn budget_limits_report_which_one_is_used_up() {
        let mut a = agent(|a| a.budget = Budget { max_steps: 3, max_minutes: 1, max_tokens: 1000, max_model_calls: 2 });
        a.validate().unwrap();
        let p = AgentProfile::from_agent(&a, "p", None);
        assert!(p.exhausted(0, &[0; 4]).is_none());
        assert!(p.exhausted(3, &[0; 4]).unwrap().contains("maxSteps"));
        assert!(p.exhausted(1, &[0, 0, 1200, 1]).unwrap().contains("maxTokens"));
        assert!(p.exhausted(1, &[0, 0, 10, 2]).unwrap().contains("maxModelCalls"));
    }

    #[test]
    fn templates_cover_the_required_roles_and_serialise_without_ids() {
        let t = templates();
        let keys: Vec<&str> = t.iter().map(|t| t.key.as_str()).collect();
        for k in ["designer", "researcher", "developer", "tester", "security_reviewer", "coordinator", "general_assistant"] {
            assert!(keys.contains(&k), "{k}");
        }
        for t in &t {
            assert!(t.agent.get("id").is_none() && t.agent.get("builtin").is_none() && t.agent.get("tools").is_some());
        }
        for (_, mut a) in template_agents() {
            a.validate().unwrap();
        }
    }

    #[test]
    fn seeding_is_idempotent_and_builtins_survive() {
        let db = Db::open_in_memory().unwrap();
        seed_builtins(&db).unwrap();
        seed_builtins(&db).unwrap();
        let all = db.list_custom_agents().unwrap();
        assert_eq!(all.len(), 7);
        assert!(all.iter().all(|a| a.builtin));
        db.delete_custom_agent(&all[0].id).unwrap();
        assert_eq!(db.list_custom_agents().unwrap().len(), 7, "built-ins are not deleted by the store");
        assert_eq!(db.list_teams().unwrap().len(), 1);
    }
}
