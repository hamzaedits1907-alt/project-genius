//! Missions: a goal planned by a coordinator agent into a dependency-aware task graph that a
//! team of custom agents (see `custom_agents.rs`) executes.
//!
//! * **Planning.** `create_mission` asks the coordinator (a real model call through the router) for
//!   a JSON task graph, validates it (known assignees, no cycles, bounded size) and falls back to a
//!   single task when it is invalid.
//! * **Scheduling.** Independent ready tasks run concurrently (default 2); a task starts only after
//!   its dependencies are done; one agent works on one task at a time; higher priority first.
//! * **Isolation.** A task that may edit files runs in its own copy of the project (`isolation.rs`),
//!   registered as a hidden project so the whole real agent loop works unchanged on it. Nothing
//!   reaches the live project until the task's changes are collected, conflict-checked, verified in
//!   the integrated result and applied as a checkpointed changeset (revertible in History).
//! * **Honesty.** Statuses change only on real transitions; the mission summary is built from
//!   recorded results and verification outcomes, never written by a model.
//!
//! Pause: no new tasks are started; tasks already running are allowed to finish (and integrate).
//! Cancel: running tasks are cancelled through their cancellation tokens; tasks that are already
//! `done` (and integrated) stay; everything else is cancelled and its isolated workspace removed.

use crate::agent::custom_run::{CustomRunInput, RunEnd};
use crate::app::Genius;
use crate::changes::{ChangeStore, FileChange};
use crate::custom_agents::{AgentProfile, CustomAgent, Team};
use crate::db::{now, Db, ProjectRow};
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use crate::isolation::{self, Manifest};
use crate::nvidia::ChatMessage;
use crate::process::Operation;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

pub const MISSION_UPDATE_CHANNEL: &str = "genius://mission-update";
const MAX_TASKS: usize = 12;
/// Tasks of one mission that run at the same time.
pub const DEFAULT_CONCURRENCY: usize = 2;
const WORKSPACES_DIR: &str = "mission-workspaces";

// ------------------------------------------------------------------ types

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Verification {
    /// unknown | passed | failed
    pub build: String,
    pub tests: String,
    pub notes: String,
}

impl Default for Verification {
    fn default() -> Self {
        Self { build: "unknown".into(), tests: "unknown".into(), notes: String::new() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskUsage {
    pub tokens: u64,
    pub calls: u64,
}

/// Bookkeeping that is stored with the task but not part of the UI contract.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskInternal {
    pub shadow_project_id: Option<String>,
    pub task_dir: Option<String>,
    /// The task is blocked only because a dependency failed (it resumes when that is fixed).
    pub dep_blocked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MissionTask {
    pub id: String,
    pub mission_id: String,
    pub title: String,
    pub description: String,
    pub assignee_agent_id: Option<String>,
    pub depends_on: Vec<String>,
    /// Higher runs first.
    pub priority: i64,
    /// pending | ready | running | blocked | awaiting_approval | reviewing | done | failed | cancelled
    pub status: String,
    pub result: Option<String>,
    pub blocker: Option<String>,
    pub changeset_id: Option<String>,
    pub workspace: Option<String>,
    pub verification: Verification,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub usage: TaskUsage,
    // ---- additive fields (not in the original contract)
    /// Files the task changed (relative paths).
    pub changed_paths: Vec<String>,
    /// Files in conflict with changes made to the live project or by another task.
    pub conflicts: Vec<String>,
    /// Self-verification against the acceptance criteria: `{checked, met, unmet, evidence}`.
    pub acceptance: Value,
    pub attempts: u32,
    /// Operations the agent attempted that its limits refused.
    pub violations: Vec<String>,
    #[serde(skip)]
    pub internal: TaskInternal,
}

impl MissionTask {
    fn extra_json(&self) -> Value {
        json!({
            "changedPaths": self.changed_paths, "conflicts": self.conflicts, "acceptance": self.acceptance, "attempts": self.attempts, "violations": self.violations,
            "shadowProjectId": self.internal.shadow_project_id, "taskDir": self.internal.task_dir, "depBlocked": self.internal.dep_blocked,
        })
    }

    fn apply_extra(&mut self, v: &Value) {
        let strings = |k: &str| v.get(k).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
        self.changed_paths = strings("changedPaths");
        self.conflicts = strings("conflicts");
        self.violations = strings("violations");
        self.acceptance = v.get("acceptance").cloned().unwrap_or(Value::Null);
        self.attempts = v.get("attempts").and_then(|a| a.as_u64()).unwrap_or(0) as u32;
        self.internal = TaskInternal {
            shadow_project_id: v.get("shadowProjectId").and_then(|s| s.as_str()).map(String::from),
            task_dir: v.get("taskDir").and_then(|s| s.as_str()).map(String::from),
            dep_blocked: v.get("depBlocked").and_then(|s| s.as_bool()).unwrap_or(false),
        };
    }

    fn is_terminal(&self) -> bool {
        matches!(self.status.as_str(), "done" | "failed" | "cancelled")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Mission {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub goal: String,
    pub team_id: Option<String>,
    /// planning | ready | running | paused | integrating | verifying | completed | failed | cancelled
    pub status: String,
    pub tasks: Vec<MissionTask>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip)]
    pub meta: Value,
}

/// In-memory control block of a mission in this process.
pub struct MissionCtl {
    pub driver_active: bool,
    pub token: CancellationToken,
    /// Cancellation tokens of running tasks (children of `token`).
    pub tasks: HashMap<String, CancellationToken>,
    pub wake: Arc<Notify>,
}

impl MissionCtl {
    fn new() -> Self {
        Self { driver_active: false, token: CancellationToken::new(), tasks: HashMap::new(), wake: Arc::new(Notify::new()) }
    }
}

// ------------------------------------------------------------------ persistence

fn row_mission(r: &rusqlite::Row) -> rusqlite::Result<Mission> {
    let meta: String = r.get("meta_json")?;
    Ok(Mission {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        title: r.get("title")?,
        goal: r.get("goal")?,
        team_id: r.get("team_id")?,
        status: r.get("status")?,
        tasks: vec![],
        summary: r.get("summary")?,
        created_at: r.get("created_at")?,
        updated_at: r.get("updated_at")?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
    })
}

fn row_task(r: &rusqlite::Row) -> rusqlite::Result<MissionTask> {
    let deps: String = r.get("depends_on_json")?;
    let ver: String = r.get("verification_json")?;
    let usage: String = r.get("usage_json")?;
    let extra: String = r.get("extra_json")?;
    let mut t = MissionTask {
        id: r.get("id")?,
        mission_id: r.get("mission_id")?,
        title: r.get("title")?,
        description: r.get("description")?,
        assignee_agent_id: r.get("assignee_agent_id")?,
        depends_on: serde_json::from_str(&deps).unwrap_or_default(),
        priority: r.get("priority")?,
        status: r.get("status")?,
        result: r.get("result")?,
        blocker: r.get("blocker")?,
        changeset_id: r.get("changeset_id")?,
        workspace: r.get("workspace")?,
        verification: serde_json::from_str(&ver).unwrap_or_default(),
        started_at: r.get("started_at")?,
        ended_at: r.get("ended_at")?,
        usage: serde_json::from_str(&usage).unwrap_or_default(),
        ..Default::default()
    };
    t.apply_extra(&serde_json::from_str(&extra).unwrap_or(Value::Null));
    Ok(t)
}

impl Db {
    pub fn insert_mission_row(&self, m: &Mission) -> Result<()> {
        self.conn().execute(
            "INSERT INTO missions(id, project_id, title, goal, team_id, status, summary, meta_json, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![m.id, m.project_id, m.title, m.goal, m.team_id, m.status, m.summary, m.meta.to_string(), m.created_at, m.updated_at],
        )?;
        Ok(())
    }

    pub fn save_mission_row(&self, m: &Mission) -> Result<()> {
        self.conn().execute(
            "UPDATE missions SET title = ?2, status = ?3, summary = ?4, meta_json = ?5, updated_at = ?6 WHERE id = ?1",
            params![m.id, m.title, m.status, m.summary, m.meta.to_string(), now()],
        )?;
        Ok(())
    }

    pub fn get_mission_row(&self, id: &str) -> Result<Mission> {
        self.conn().query_row("SELECT * FROM missions WHERE id = ?1", [id], row_mission).optional()?.ok_or_else(|| GeniusError::NotFound(format!("mission {id}")))
    }

    pub fn list_mission_rows(&self, project_id: Option<&str>) -> Result<Vec<Mission>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM missions WHERE (?1 IS NULL OR project_id = ?1) ORDER BY created_at DESC, rowid DESC")?;
        let rows = stmt.query_map(params![project_id], row_mission)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn insert_task_row(&self, ordinal: usize, t: &MissionTask) -> Result<()> {
        self.conn().execute(
            "INSERT INTO mission_tasks(id, mission_id, ordinal, title, description, assignee_agent_id, depends_on_json, priority, status, result, blocker, changeset_id, workspace, verification_json, started_at, ended_at, usage_json, extra_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![t.id, t.mission_id, ordinal as i64, t.title, t.description, t.assignee_agent_id, serde_json::to_string(&t.depends_on)?, t.priority, t.status, t.result, t.blocker, t.changeset_id, t.workspace, serde_json::to_string(&t.verification)?, t.started_at, t.ended_at, serde_json::to_string(&t.usage)?, t.extra_json().to_string()],
        )?;
        Ok(())
    }

    pub fn save_task_row(&self, t: &MissionTask) -> Result<()> {
        self.conn().execute(
            "UPDATE mission_tasks SET title = ?2, description = ?3, assignee_agent_id = ?4, depends_on_json = ?5, priority = ?6, status = ?7, result = ?8, blocker = ?9, changeset_id = ?10, workspace = ?11, verification_json = ?12, started_at = ?13, ended_at = ?14, usage_json = ?15, extra_json = ?16 WHERE id = ?1",
            params![t.id, t.title, t.description, t.assignee_agent_id, serde_json::to_string(&t.depends_on)?, t.priority, t.status, t.result, t.blocker, t.changeset_id, t.workspace, serde_json::to_string(&t.verification)?, t.started_at, t.ended_at, serde_json::to_string(&t.usage)?, t.extra_json().to_string()],
        )?;
        Ok(())
    }

    pub fn get_task_row(&self, id: &str) -> Result<MissionTask> {
        self.conn().query_row("SELECT * FROM mission_tasks WHERE id = ?1", [id], row_task).optional()?.ok_or_else(|| GeniusError::NotFound(format!("mission task {id}")))
    }

    pub fn tasks_of(&self, mission_id: &str) -> Result<Vec<MissionTask>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM mission_tasks WHERE mission_id = ?1 ORDER BY ordinal ASC, rowid ASC")?;
        let rows = stmt.query_map([mission_id], row_task)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Paths of other tasks on the same project whose changes are waiting to be applied.
    pub fn pending_paths(&self, project_id: &str, except_task: &str) -> Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT t.extra_json FROM mission_tasks t JOIN missions m ON m.id = t.mission_id WHERE m.project_id = ?1 AND t.id != ?2 AND t.status IN ('awaiting_approval','reviewing')")?;
        let rows = stmt.query_map(params![project_id, except_task], |r| r.get::<_, String>(0))?;
        let mut out = HashSet::new();
        for e in rows {
            let v: Value = serde_json::from_str(&e?).unwrap_or(Value::Null);
            if let Some(a) = v.get("changedPaths").and_then(|a| a.as_array()) {
                out.extend(a.iter().filter_map(|p| p.as_str().map(String::from)));
            }
        }
        Ok(out)
    }
}

/// Called when Genius starts: nothing survives a restart except what is in the database, so
/// anything that was in flight is closed honestly and workspaces nobody can use are removed.
pub fn recover(db: &Db, data_dir: &Path) -> Result<()> {
    let active: Vec<Mission> = db.list_mission_rows(None)?.into_iter().filter(|m| matches!(m.status.as_str(), "planning" | "running" | "integrating" | "verifying")).collect();
    for mut m in active {
        let tasks = db.tasks_of(&m.id)?;
        for mut t in tasks {
            match t.status.as_str() {
                "running" => {
                    t.status = "failed".into();
                    t.blocker = Some("interrupted: Genius was closed while this task was running. Retry it to run it again.".into());
                    t.ended_at = Some(now());
                    db.save_task_row(&t)?;
                }
                "reviewing" => {
                    if t.changeset_id.is_some() && t.internal.task_dir.as_deref().is_some_and(|d| Path::new(d).exists()) {
                        t.status = "awaiting_approval".into();
                        t.blocker = None;
                    } else {
                        t.status = "failed".into();
                        t.blocker = Some("interrupted: Genius was closed while this task's changes were being verified. Retry it.".into());
                        t.ended_at = Some(now());
                    }
                    db.save_task_row(&t)?;
                }
                _ => {}
            }
        }
        if m.status == "planning" {
            m.status = "failed".into();
            m.summary = Some("Planning was interrupted because Genius was closed. Create the mission again.".into());
        } else {
            m.status = "paused".into();
            m.summary = Some("Genius was closed while this mission was running. Tasks that were running are marked failed; resume the mission and retry them.".into());
        }
        db.save_mission_row(&m)?;
    }
    // Isolated workspaces that no task can use any more.
    let keep: HashSet<String> = db
        .list_mission_rows(None)?
        .iter()
        .flat_map(|m| db.tasks_of(&m.id).unwrap_or_default())
        .filter(|t| matches!(t.status.as_str(), "awaiting_approval" | "blocked" | "reviewing"))
        .filter_map(|t| t.internal.shadow_project_id)
        .collect();
    let orphans: Vec<ProjectRow> = db.list_hidden_projects()?.into_iter().filter(|p| !keep.contains(&p.id)).collect();
    for p in orphans {
        remove_workspace(data_dir, &p.path);
        let _ = db.delete_project(&p.id);
    }
    Ok(())
}

impl Db {
    fn list_hidden_projects(&self) -> Result<Vec<ProjectRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM projects WHERE kind = 'mission_workspace'")?;
        let rows = stmt.query_map([], |r| {
            Ok(ProjectRow {
                id: r.get("id")?,
                name: r.get("name")?,
                description: r.get("description")?,
                path: r.get("path")?,
                kind: r.get("kind")?,
                spec: None,
                notes: String::new(),
                status: Value::Null,
                created_at: r.get("created_at")?,
                updated_at: r.get("updated_at")?,
                last_opened_at: None,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Delete an isolated workspace directory (the task directory that contains `ws/`), but only
/// inside Genius's own `mission-workspaces` folder. Links (`node_modules`) are removed, never followed.
fn remove_workspace(data_dir: &Path, any_path_in_task_dir: &str) {
    let root = data_dir.join(WORKSPACES_DIR);
    let Ok(root) = root.canonicalize() else { return };
    let p = PathBuf::from(any_path_in_task_dir);
    // <root>/<mission>/<task>[/ws]
    let task_dir = if p.file_name().is_some_and(|n| n == "ws") { p.parent().map(Path::to_path_buf) } else { Some(p) };
    let Some(task_dir) = task_dir.and_then(|d| d.canonicalize().ok()) else { return };
    if task_dir.starts_with(&root) && task_dir != root && task_dir.parent() != Some(root.as_path()).filter(|_| false) {
        let _ = std::fs::remove_dir_all(&task_dir);
    }
}

// ------------------------------------------------------------------ plan validation

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedTask {
    pub key: String,
    pub title: String,
    pub description: String,
    pub assignee: String,
    pub deps: Vec<String>,
    pub priority: i64,
}

/// Validate the coordinator's JSON. `roster` is (agent id, agent name) of the allowed assignees.
/// Returns the plan summary and tasks, or the reason the plan is unusable.
pub fn validate_plan(v: &Value, roster: &[(String, String)]) -> std::result::Result<(String, Vec<PlannedTask>), String> {
    let tasks = v.get("tasks").and_then(|t| t.as_array()).ok_or("the plan has no \"tasks\" array")?;
    if tasks.is_empty() {
        return Err("the plan has no tasks".into());
    }
    if tasks.len() > MAX_TASKS {
        return Err(format!("the plan has {} tasks; at most {MAX_TASKS} are allowed", tasks.len()));
    }
    let mut out: Vec<PlannedTask> = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let key = t.get("id").or_else(|| t.get("key")).map(|k| match k {
            Value::String(s) => s.trim().to_string(),
            other => other.to_string(),
        });
        let key = key.filter(|k| !k.is_empty()).unwrap_or_else(|| format!("t{}", i + 1));
        if out.iter().any(|o| o.key == key) {
            return Err(format!("duplicate task id '{key}'"));
        }
        let title = t.get("title").and_then(|s| s.as_str()).map(|s| s.trim().to_string()).unwrap_or_default();
        if title.is_empty() {
            return Err(format!("task '{key}' has no title"));
        }
        let description = t.get("description").and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
        let who = t.get("assignee").or_else(|| t.get("assigneeAgentId")).or_else(|| t.get("agent")).and_then(|s| s.as_str()).unwrap_or("").trim().to_string();
        let assignee = roster
            .iter()
            .find(|(id, _)| *id == who)
            .or_else(|| roster.iter().find(|(_, name)| !who.is_empty() && name.eq_ignore_ascii_case(&who)))
            .map(|(id, _)| id.clone())
            .ok_or_else(|| format!("task '{key}' is assigned to '{who}', who is not on the team"))?;
        let deps: Vec<String> = t
            .get("dependsOn")
            .or_else(|| t.get("depends_on"))
            .and_then(|d| d.as_array())
            .map(|a| a.iter().map(|d| d.as_str().map(String::from).unwrap_or_else(|| d.to_string())).collect())
            .unwrap_or_default();
        let priority = t.get("priority").and_then(|p| p.as_i64().or_else(|| p.as_f64().map(|f| f as i64))).unwrap_or(3).clamp(1, 9);
        out.push(PlannedTask { key, title: title.chars().take(140).collect(), description: description.chars().take(4000).collect(), assignee, deps, priority });
    }
    let keys: Vec<String> = out.iter().map(|t| t.key.clone()).collect();
    for t in &out {
        for d in &t.deps {
            if !keys.contains(d) {
                return Err(format!("task '{}' depends on '{d}', which does not exist", t.key));
            }
            if *d == t.key {
                return Err(format!("task '{}' depends on itself", t.key));
            }
        }
    }
    // Kahn: any task left over is on a cycle
    let mut indeg: HashMap<&str, usize> = out.iter().map(|t| (t.key.as_str(), t.deps.len())).collect();
    let mut ready: Vec<&str> = indeg.iter().filter(|(_, n)| **n == 0).map(|(k, _)| *k).collect();
    let mut seen = 0;
    while let Some(k) = ready.pop() {
        seen += 1;
        for t in &out {
            if t.deps.iter().any(|d| d == k) {
                let n = indeg.get_mut(t.key.as_str()).unwrap();
                *n -= 1;
                if *n == 0 {
                    ready.push(t.key.as_str());
                }
            }
        }
    }
    if seen != out.len() {
        return Err("the task dependencies contain a cycle".into());
    }
    let summary = v.get("summary").and_then(|s| s.as_str()).unwrap_or("").trim().chars().take(600).collect();
    Ok((summary, out))
}

/// First JSON object in a model reply (fenced, or surrounded by prose).
fn extract_json_object(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    crate::agent::lenient_json(&text[start..]).ok().filter(|v| v.is_object())
}

// ------------------------------------------------------------------ isolated workspaces

struct Isolated {
    shadow_pid: String,
    task_dir: PathBuf,
    root: PathBuf,
}

fn clip_text(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn first_line(s: &str, max: usize) -> String {
    clip_text(s.trim().lines().next().unwrap_or(""), max)
}

enum Step {
    Launch(Vec<String>),
    Wait,
    Idle,
    Finalize,
    Fail,
}

/// How a task ended; applied to the database by `finish_task`.
enum TaskEnd {
    Done { result: String, verification: Verification, changeset_id: Option<String> },
    Awaiting { result: String, changeset_id: String, verification: Verification },
    Blocked { result: String, blocker: String, conflicts: Vec<String>, changeset_id: Option<String> },
    Failed { result: String, blocker: String, verification: Verification, changeset_id: Option<String> },
    Cancelled { result: String },
}

struct VerifyResult {
    v: Verification,
    /// Something was wrong that makes the result unusable even though no check failed
    /// (a check could not run).
    error: Option<String>,
    failed: bool,
}

impl Genius {
    // ============================================================== reading / emitting

    pub fn get_mission(&self, id: &str) -> Result<Mission> {
        let mut m = self.db.get_mission_row(id)?;
        m.tasks = self.db.tasks_of(id)?;
        Ok(m)
    }

    pub fn list_missions(&self, project_id: Option<&str>) -> Result<Vec<Mission>> {
        let mut v = self.db.list_mission_rows(project_id)?;
        for m in &mut v {
            m.tasks = self.db.tasks_of(&m.id)?;
        }
        Ok(v)
    }

    /// Push the mission's current state to the UI.
    pub(crate) fn emit_mission(&self, id: &str) {
        if let Ok(m) = self.get_mission(id) {
            self.bus.push(MISSION_UPDATE_CHANNEL, serde_json::to_value(&m).unwrap_or(Value::Null));
        }
    }

    fn update_task<F: FnOnce(&mut MissionTask)>(&self, task_id: &str, f: F) -> Result<MissionTask> {
        let t = {
            let _g = self.agents.rows.lock().unwrap();
            let mut t = self.db.get_task_row(task_id)?;
            f(&mut t);
            self.db.save_task_row(&t)?;
            t
        };
        self.emit_mission(&t.mission_id);
        Ok(t)
    }

    fn update_mission<F: FnOnce(&mut Mission)>(&self, id: &str, f: F) -> Result<Mission> {
        let m = {
            let _g = self.agents.rows.lock().unwrap();
            let mut m = self.db.get_mission_row(id)?;
            f(&mut m);
            self.db.save_mission_row(&m)?;
            m
        };
        self.emit_mission(id);
        Ok(m)
    }

    fn mission_ctl(&self, id: &str) -> (CancellationToken, Arc<Notify>) {
        let mut all = self.agents.missions.lock().unwrap();
        let c = all.entry(id.to_string()).or_insert_with(MissionCtl::new);
        (c.token.clone(), c.wake.clone())
    }

    fn task_token(&self, mission_id: &str, task_id: &str) -> CancellationToken {
        let mut all = self.agents.missions.lock().unwrap();
        let c = all.entry(mission_id.to_string()).or_insert_with(MissionCtl::new);
        let t = c.token.child_token();
        c.tasks.insert(task_id.to_string(), t.clone());
        t
    }

    fn drop_task_token(&self, mission_id: &str, task_id: &str) {
        if let Some(c) = self.agents.missions.lock().unwrap().get_mut(mission_id) {
            c.tasks.remove(task_id);
        }
    }

    fn wake_driver(&self, mission_id: &str) {
        if let Some(c) = self.agents.missions.lock().unwrap().get(mission_id) {
            c.wake.notify_one();
        }
    }

    // ============================================================== creating missions

    fn resolve_team(&self, team_id: Option<&str>) -> Result<(Option<Team>, CustomAgent, Vec<CustomAgent>)> {
        match team_id {
            Some(id) => {
                let team = self.db.get_team(id)?;
                let coordinator = self.db.get_custom_agent(&team.coordinator_agent_id).map_err(|_| GeniusError::Validation("The team's coordinator agent no longer exists.".into()))?;
                let members: Vec<CustomAgent> = team.agent_ids.iter().filter(|a| **a != coordinator.id).filter_map(|a| self.db.get_custom_agent(a).ok()).collect();
                Ok((Some(team), coordinator, members))
            }
            None => {
                let all = self.db.list_custom_agents()?;
                let coordinator = all.iter().find(|a| a.id == crate::custom_agents::builtin_id("coordinator")).or_else(|| all.first()).cloned().ok_or_else(|| GeniusError::Validation("There are no agents yet.".into()))?;
                let members: Vec<CustomAgent> = all.into_iter().filter(|a| a.id != coordinator.id).take(MAX_TASKS).collect();
                Ok((None, coordinator, members))
            }
        }
    }

    /// Start a mission: stores it as `planning` and lets the coordinator plan it in the
    /// background (`genius://mission-update` fires when it is `ready`).
    pub fn create_mission(self: &Arc<Self>, project_id: &str, goal: &str, team_id: Option<&str>) -> Result<Mission> {
        let goal = goal.trim();
        if goal.chars().count() < 3 || goal.chars().count() > 4000 {
            return Err(GeniusError::Validation("Describe the goal in 3-4000 characters.".into()));
        }
        let project = self.db.get_project(project_id)?;
        if project.kind == "mission_workspace" {
            return Err(GeniusError::NotFound(format!("project {project_id}")));
        }
        self.resolve_team(team_id)?;
        self.has_usable_model()?;
        self.client()?;
        let t = now();
        let m = Mission { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), title: first_line(goal, 80), goal: goal.into(), team_id: team_id.map(String::from), status: "planning".into(), tasks: vec![], summary: None, created_at: t.clone(), updated_at: t, meta: json!({}) };
        self.db.insert_mission_row(&m)?;
        self.bus.record(Some(project_id), None, EventKind::Task, EventStatus::Running, format!("Mission: {}", m.title), json!({ "missionId": m.id }));
        self.emit_mission(&m.id);
        let me = self.clone();
        let id = m.id.clone();
        self.runtime().spawn(async move {
            if let Err(e) = me.plan_mission(&id).await {
                if !matches!(e, GeniusError::Cancelled) {
                    let _ = me.update_mission(&id, |m| {
                        m.status = "failed".into();
                        m.summary = Some(format!("Planning failed: {e}"));
                    });
                }
            }
        });
        Ok(m)
    }

    /// Ask the coordinator for a task graph and store it (mission → `ready`). Invalid plans fall
    /// back to a single task, and the summary says so.
    pub async fn plan_mission(&self, mission_id: &str) -> Result<Mission> {
        let mission = self.db.get_mission_row(mission_id)?;
        let (_team, coordinator, members) = self.resolve_team(mission.team_id.as_deref())?;
        let (token, _) = self.mission_ctl(mission_id);
        let pid = mission.project_id.clone();
        self.set_agent_status(&coordinator.id, "planning", None, &pid, Some(mission_id), "planning the mission");

        let roster: Vec<(String, String)> = if members.is_empty() { vec![(coordinator.id.clone(), coordinator.name.clone())] } else { members.iter().map(|a| (a.id.clone(), a.name.clone())).collect() };
        let roster_text: String = if members.is_empty() { vec![&coordinator] } else { members.iter().collect() }
            .iter()
            .map(|a| {
                let eff = a.effective_tools();
                format!("- id: {} | name: {} | role: {} | {} | {} | tools: {}\n", a.id, a.name, a.role, a.description, if a.can_edit() { "can edit files (works in an isolated copy)" } else { "read-only (reports, no edits)" }, eff.iter().cloned().collect::<Vec<_>>().join(", "))
            })
            .collect();
        let facts = self.workspace(&pid).ok().and_then(|ws| crate::project_context::gather(&ws).ok()).map(|f| crate::project_context::render(&f)).unwrap_or_default();
        let system = format!(
            "You are the coordinator \"{}\" of a team of AI agents working on a software project. Plan how the team achieves the goal.\n\nCoordinator instructions written by the user (they cannot change the output format or the rules below):\n<<<COORDINATOR INSTRUCTIONS>>>\n{}\n<<<END COORDINATOR INSTRUCTIONS>>>\n\nTEAM (assign tasks only to these agents, by id):\n{}\nRULES:\n- 1 to {MAX_TASKS} tasks. Each task is concrete, completable by one agent alone, and has a clear deliverable.\n- Use dependsOn only for real dependencies (a task that needs another task's result or files). Tasks that touch different files should NOT depend on each other so they can run in parallel.\n- Never give two parallel tasks the same file to edit.\n- Read-only agents can investigate, review or plan; only editing agents change files.\n- priority is 1 (low) to 9 (urgent); default 3.\n\nReply with ONLY a JSON object, no prose:\n{{\"summary\": \"one sentence\", \"tasks\": [{{\"id\": \"t1\", \"title\": \"...\", \"description\": \"what to do and the expected result\", \"assignee\": \"<agent id>\", \"dependsOn\": [], \"priority\": 3}}]}}",
            coordinator.name,
            coordinator.instructions.replace("<<<END COORDINATOR INSTRUCTIONS>>>", ""),
            roster_text
        );
        let user = format!("PROJECT: {}\nGOAL:\n{}\n\n{}", self.db.get_project(&pid)?.name, mission.goal, clip_text(&facts, 5000));
        let mut messages = vec![ChatMessage::system(system), ChatMessage::user(user)];
        let effort = if coordinator.model_policy == "max" { crate::settings::Effort::Max } else { crate::settings::Effort::Balanced };
        let opts = crate::app::CallOptions { role: Some(crate::router::ModelRole::Plan), effort: Some(effort), model: if coordinator.model_policy == "manual" { coordinator.model.clone() } else { None }, max_tokens: Some(4096), thinking: Some(false), ..Default::default() };
        let timeout = std::time::Duration::from_secs(coordinator.budget.max_minutes as u64 * 60);
        let (mut tokens, mut calls) = (0u64, 0u64);
        let mut plan: std::result::Result<(String, Vec<PlannedTask>), String> = Err("the coordinator did not answer".into());
        let attempts = if coordinator.budget.max_model_calls >= 2 { 2 } else { 1 };
        let mut call_error: Option<GeniusError> = None;
        for attempt in 0..attempts {
            let r = tokio::time::timeout(timeout, self.ai_call_opts(&pid, "", "Mission planning", &messages, false, &token, &opts)).await;
            let res = match r {
                Err(_) => {
                    call_error = Some(GeniusError::Validation(format!("the coordinator exceeded its time budget ({} min)", coordinator.budget.max_minutes)));
                    break;
                }
                Ok(Err(GeniusError::Cancelled)) => {
                    self.set_agent_status(&coordinator.id, "idle", None, &pid, Some(mission_id), "planning cancelled");
                    return Err(GeniusError::Cancelled);
                }
                Ok(Err(e)) => {
                    call_error = Some(e);
                    break;
                }
                Ok(Ok((r, _))) => r,
            };
            calls += 1;
            tokens += res.usage.as_ref().map(|u| crate::app::usage_tokens(u).2).unwrap_or(0);
            self.agent_log(&coordinator.id, Some(mission_id), None, &pid, "message", &clip_text(&res.content, 1500), json!({ "purpose": "mission plan", "attempt": attempt + 1 }));
            plan = match extract_json_object(&res.content) {
                Some(v) => validate_plan(&v, &roster),
                None => Err("the reply was not a JSON object".into()),
            };
            match &plan {
                Ok(_) => break,
                Err(why) if attempt + 1 < attempts && tokens < coordinator.budget.max_tokens => {
                    messages.push(ChatMessage::assistant(clip_text(&res.content, 3000)));
                    messages.push(ChatMessage::user(format!("That plan is not usable: {why}. Reply again with ONLY the corrected JSON object.")));
                }
                Err(_) => {}
            }
        }
        let _ = self.db.add_agent_usage(&coordinator.id, tokens, calls, 0, 0);
        if token.is_cancelled() || self.db.get_mission_row(mission_id)?.status != "planning" {
            return Err(GeniusError::Cancelled);
        }
        let mut note = String::new();
        let planned = match plan {
            Ok((summary, tasks)) => {
                note = summary;
                tasks
            }
            Err(why) => {
                if let Some(e) = &call_error {
                    self.agent_log(&coordinator.id, Some(mission_id), None, &pid, "error", &format!("planning call failed: {e}"), json!({}));
                }
                // single-task fallback: the first editing member, else the first member
                let who = members.iter().find(|a| a.can_edit()).or_else(|| members.first()).unwrap_or(&coordinator);
                note = format!("The coordinator's plan could not be used ({}), so the mission runs as a single task assigned to {}.", call_error.as_ref().map(|e| e.to_string()).unwrap_or(why), who.name);
                vec![PlannedTask { key: "t1".into(), title: first_line(&mission.goal, 120), description: mission.goal.clone(), assignee: who.id.clone(), deps: vec![], priority: 3 }]
            }
        };
        // store tasks (keys → ids)
        let ids: HashMap<String, String> = planned.iter().map(|p| (p.key.clone(), uuid::Uuid::new_v4().to_string())).collect();
        {
            let _g = self.agents.rows.lock().unwrap();
            for (i, p) in planned.iter().enumerate() {
                let deps: Vec<String> = p.deps.iter().filter_map(|d| ids.get(d).cloned()).collect();
                let t = MissionTask { id: ids[&p.key].clone(), mission_id: mission_id.into(), title: p.title.clone(), description: p.description.clone(), assignee_agent_id: Some(p.assignee.clone()), status: if deps.is_empty() { "ready".into() } else { "pending".into() }, depends_on: deps, priority: p.priority, ..Default::default() };
                self.db.insert_task_row(i, &t)?;
            }
        }
        let m = self.update_mission(mission_id, |m| {
            m.status = "ready".into();
            m.meta = json!({ "plan": note, "coordinatorAgentId": coordinator.id });
        })?;
        self.set_agent_status(&coordinator.id, "completed", None, &pid, Some(mission_id), &format!("planned {} task(s)", planned.len()));
        self.get_mission(&m.id)
    }

    /// One agent, one task: a single-task mission that starts immediately.
    pub fn run_agent_task(self: &Arc<Self>, agent_id: &str, project_id: &str, task: &str) -> Result<Mission> {
        let task = task.trim();
        if task.chars().count() < 3 || task.chars().count() > 8000 {
            return Err(GeniusError::Validation("Describe the task in 3-8000 characters.".into()));
        }
        let agent = self.db.get_custom_agent(agent_id)?;
        let project = self.db.get_project(project_id)?;
        if project.kind == "mission_workspace" {
            return Err(GeniusError::NotFound(format!("project {project_id}")));
        }
        self.has_usable_model()?;
        self.client()?;
        let t = now();
        let mission = Mission { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), title: first_line(task, 80), goal: task.into(), team_id: None, status: "ready".into(), tasks: vec![], summary: None, created_at: t.clone(), updated_at: t, meta: json!({ "plan": format!("Single task for {}", agent.name), "singleAgent": true }) };
        self.db.insert_mission_row(&mission)?;
        let tk = MissionTask { id: uuid::Uuid::new_v4().to_string(), mission_id: mission.id.clone(), title: first_line(task, 120), description: task.into(), assignee_agent_id: Some(agent.id.clone()), status: "ready".into(), priority: 3, ..Default::default() };
        self.db.insert_task_row(0, &tk)?;
        self.start_mission(&mission.id)
    }

    // ============================================================== controls

    pub fn start_mission(self: &Arc<Self>, mission_id: &str) -> Result<Mission> {
        let m = self.db.get_mission_row(mission_id)?;
        match m.status.as_str() {
            "ready" => {}
            "running" => return self.get_mission(mission_id),
            "planning" => return Err(GeniusError::Conflict("The coordinator is still planning this mission.".into())),
            s => return Err(GeniusError::Conflict(format!("A {s} mission cannot be started."))),
        }
        self.update_mission(mission_id, |m| m.status = "running".into())?;
        self.bus.record(Some(&m.project_id), None, EventKind::Task, EventStatus::Running, format!("Mission started: {}", m.title), json!({ "missionId": m.id }));
        self.ensure_driver(mission_id);
        self.get_mission(mission_id)
    }

    /// Stop starting new tasks. Tasks that are already running finish (and integrate); nothing
    /// is cancelled.
    pub fn pause_mission(&self, mission_id: &str) -> Result<Mission> {
        let m = self.db.get_mission_row(mission_id)?;
        if !matches!(m.status.as_str(), "running" | "ready") {
            return Err(GeniusError::Conflict(format!("A {} mission cannot be paused.", m.status)));
        }
        self.update_mission(mission_id, |m| m.status = "paused".into())?;
        // agents with queued work in this mission show as paused (agents that are working keep their real status)
        for t in self.db.tasks_of(mission_id)? {
            if matches!(t.status.as_str(), "pending" | "ready") {
                if let Some(a) = &t.assignee_agent_id {
                    if !self.agents.busy.lock().unwrap().contains(a) {
                        self.set_agent_status(a, "paused", None, &m.project_id, Some(mission_id), "mission paused");
                    }
                }
            }
        }
        self.get_mission(mission_id)
    }

    pub fn resume_mission(self: &Arc<Self>, mission_id: &str) -> Result<Mission> {
        let m = self.db.get_mission_row(mission_id)?;
        if m.status != "paused" {
            return Err(GeniusError::Conflict(format!("A {} mission cannot be resumed.", m.status)));
        }
        for t in self.db.tasks_of(mission_id)? {
            if let Some(a) = &t.assignee_agent_id {
                let paused = self.agents.state.lock().unwrap().get(a).is_some_and(|s| s.status == "paused");
                if paused {
                    self.set_agent_status(a, "idle", None, &m.project_id, Some(mission_id), "mission resumed");
                }
            }
        }
        self.update_mission(mission_id, |m| m.status = "running".into())?;
        self.ensure_driver(mission_id);
        self.get_mission(mission_id)
    }

    /// Cancel the mission: running tasks are cancelled, queued/blocked/awaiting tasks are
    /// cancelled and their workspaces removed. Tasks that are `done` are untouched (their
    /// changes were already integrated and stay revertible in History).
    pub fn cancel_mission(self: &Arc<Self>, mission_id: &str) -> Result<Mission> {
        let m = self.db.get_mission_row(mission_id)?;
        if matches!(m.status.as_str(), "completed" | "failed" | "cancelled") {
            return Err(GeniusError::Conflict(format!("The mission is already {}.", m.status)));
        }
        self.update_mission(mission_id, |m| m.status = "cancelled".into())?;
        let (token, _) = self.mission_ctl(mission_id);
        token.cancel();
        for t in self.db.tasks_of(mission_id)? {
            match t.status.as_str() {
                "pending" | "ready" | "blocked" | "awaiting_approval" => {
                    self.cancel_idle_task(&m, &t, "the mission was cancelled");
                }
                _ => {} // running / reviewing: the runner sees the cancelled token and finishes as cancelled
            }
        }
        self.bus.record(Some(&m.project_id), None, EventKind::Task, EventStatus::Cancelled, format!("Mission cancelled: {}", m.title), json!({ "missionId": m.id }));
        self.ensure_driver(mission_id);
        self.refresh_summary(mission_id);
        self.get_mission(mission_id)
    }

    fn cancel_idle_task(&self, m: &Mission, t: &MissionTask, why: &str) {
        if let Some(cs) = &t.changeset_id {
            let _ = self.reject_changeset(cs);
        }
        self.cleanup_task_workspace(t);
        let _ = self.update_task(&t.id, |x| {
            x.status = "cancelled".into();
            x.blocker = Some(why.into());
            x.ended_at = Some(now());
            x.workspace = None;
        });
        if let Some(a) = &t.assignee_agent_id {
            if !self.agents.busy.lock().unwrap().contains(a) {
                self.set_agent_status(a, "idle", None, &m.project_id, Some(&m.id), &format!("task cancelled: {why}"));
            }
        }
    }

    pub fn cancel_mission_task(self: &Arc<Self>, task_id: &str) -> Result<Mission> {
        let t = self.db.get_task_row(task_id)?;
        let m = self.db.get_mission_row(&t.mission_id)?;
        match t.status.as_str() {
            "done" | "failed" | "cancelled" => return Err(GeniusError::Conflict(format!("The task is already {}.", t.status))),
            "running" | "reviewing" => {
                let tok = self.agents.missions.lock().unwrap().get(&t.mission_id).and_then(|c| c.tasks.get(task_id).cloned());
                match tok {
                    Some(tok) => tok.cancel(), // the runner finishes the task as cancelled
                    None => self.cancel_idle_task(&m, &t, "cancelled by the user"),
                }
            }
            _ => self.cancel_idle_task(&m, &t, "cancelled by the user"),
        }
        self.wake_driver(&t.mission_id);
        self.ensure_driver(&t.mission_id);
        self.get_mission(&t.mission_id)
    }

    pub fn reprioritize_task(&self, task_id: &str, priority: i64) -> Result<Mission> {
        if !(1..=9).contains(&priority) {
            return Err(GeniusError::Validation("Priority is 1 (low) to 9 (urgent).".into()));
        }
        let t = self.update_task(task_id, |t| t.priority = priority)?;
        self.get_mission(&t.mission_id)
    }

    /// Run a failed, blocked or cancelled task again from the current state of the project.
    pub fn retry_mission_task(self: &Arc<Self>, task_id: &str) -> Result<Mission> {
        let t = self.db.get_task_row(task_id)?;
        let m = self.db.get_mission_row(&t.mission_id)?;
        if !matches!(t.status.as_str(), "failed" | "blocked" | "cancelled") {
            return Err(GeniusError::Conflict(format!("Only failed, blocked or cancelled tasks can be retried (this one is {}).", t.status)));
        }
        if m.status == "cancelled" {
            return Err(GeniusError::Conflict("The mission was cancelled; create a new mission instead.".into()));
        }
        if matches!(m.status.as_str(), "planning" | "verifying" | "integrating") {
            return Err(GeniusError::Conflict(format!("The mission is {} right now; try again in a moment.", m.status)));
        }
        if let Some(cs) = &t.changeset_id {
            if self.changeset(cs).map(|(c, _)| c.status == "proposed").unwrap_or(false) {
                let _ = self.reject_changeset(cs);
            }
        }
        self.cleanup_task_workspace(&t);
        self.update_task(task_id, |t| {
            t.status = "pending".into();
            t.result = None;
            t.blocker = None;
            t.changeset_id = None;
            t.workspace = None;
            t.conflicts.clear();
            t.changed_paths.clear();
            t.violations.clear();
            t.acceptance = Value::Null;
            t.verification = Verification::default();
            t.started_at = None;
            t.ended_at = None;
            t.internal = TaskInternal::default();
        })?;
        if m.status != "paused" {
            self.update_mission(&m.id, |m| {
                m.status = "running".into();
                m.summary = None;
            })?;
            self.ensure_driver(&m.id);
        }
        self.get_mission(&m.id)
    }

    /// Approve or reject the changes of a task that is `awaiting_approval`. Approved changes are
    /// re-checked for conflicts, verified in the integrated result and then applied.
    pub fn approve_mission_task(self: &Arc<Self>, task_id: &str, approved: bool) -> Result<Mission> {
        let t = self.db.get_task_row(task_id)?;
        let m = self.db.get_mission_row(&t.mission_id)?;
        if t.status != "awaiting_approval" {
            return Err(match t.status.as_str() {
                "blocked" => GeniusError::Conflict(format!("This task is blocked ({}). Retry it to run it again on top of the current project, or cancel it.", t.blocker.clone().unwrap_or_default())),
                s => GeniusError::Conflict(format!("This task is {s}, not awaiting approval.")),
            });
        }
        if !approved {
            if let Some(cs) = &t.changeset_id {
                let _ = self.reject_changeset(cs);
            }
            self.cleanup_task_workspace(&t);
            self.update_task(task_id, |x| {
                x.status = "cancelled".into();
                x.blocker = Some("rejected by the user".into());
                x.ended_at = Some(now());
                x.workspace = None;
            })?;
            if let Some(a) = &t.assignee_agent_id {
                self.set_agent_status(a, "idle", None, &m.project_id, Some(&m.id), "changes rejected by the user");
            }
            self.ensure_driver(&m.id);
            return self.get_mission(&m.id);
        }
        self.update_task(task_id, |x| x.status = "reviewing".into())?;
        let token = self.task_token(&m.id, task_id);
        if let Some(a) = &t.assignee_agent_id {
            self.set_agent_status(a, "reviewing", Some(task_id), &m.project_id, Some(&m.id), "verifying the approved changes");
        }
        let me = self.clone();
        let (mid, tid) = (m.id.clone(), task_id.to_string());
        self.runtime().spawn(async move {
            let t = me.db.get_task_row(&tid);
            if let Ok(t) = t {
                let end = me.integrate_task(&mid, &t, t.result.clone().unwrap_or_default(), &token).await;
                me.finish_task(&mid, &tid, end).await;
            }
            me.drop_task_token(&mid, &tid);
            me.ensure_driver(&mid);
        });
        self.get_mission(&m.id)
    }

    // ============================================================== scheduler

    /// Make sure a driver is scheduling this mission.
    pub(crate) fn ensure_driver(self: &Arc<Self>, mission_id: &str) {
        let spawn = {
            let mut all = self.agents.missions.lock().unwrap();
            let c = all.entry(mission_id.to_string()).or_insert_with(MissionCtl::new);
            if c.driver_active {
                c.wake.notify_one();
                false
            } else {
                c.driver_active = true;
                true
            }
        };
        if spawn {
            let me = self.clone();
            let id = mission_id.to_string();
            self.runtime().spawn(async move { me.drive_mission(id).await });
        }
    }

    async fn drive_mission(self: Arc<Self>, mission_id: String) {
        let (token, wake) = self.mission_ctl(&mission_id);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut running: HashSet<String> = HashSet::new();
        loop {
            while let Ok(done) = rx.try_recv() {
                running.remove(&done);
            }
            let step = match self.plan_step(&mission_id, running.len()) {
                Ok(s) => s,
                Err(e) => {
                    log::warn!("mission {mission_id}: scheduler error: {e}");
                    Step::Idle
                }
            };
            match step {
                Step::Launch(ids) => {
                    for id in ids {
                        if let Some(done) = self.launch_task(&mission_id, &id, tx.clone()) {
                            running.insert(done);
                        }
                    }
                }
                Step::Wait => {
                    tokio::select! {
                        Some(done) = rx.recv() => { running.remove(&done); }
                        _ = wake.notified() => {}
                        _ = token.cancelled(), if running.is_empty() => {}
                        _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {}
                    }
                }
                Step::Finalize => self.finalize_mission(&mission_id).await,
                Step::Fail => self.fail_mission(&mission_id),
                Step::Idle => {
                    if !running.is_empty() {
                        tokio::select! {
                            Some(done) = rx.recv() => { running.remove(&done); }
                            _ = wake.notified() => {}
                        }
                        continue;
                    }
                    // exit protocol: re-check under the lock so a concurrent change is not lost
                    let exit = {
                        let mut all = self.agents.missions.lock().unwrap();
                        let still_idle = matches!(self.plan_step(&mission_id, 0), Ok(Step::Idle));
                        if still_idle {
                            if let Some(c) = all.get_mut(&mission_id) {
                                c.driver_active = false;
                            }
                        }
                        still_idle
                    };
                    if exit {
                        if self.db.get_mission_row(&mission_id).map(|m| m.status == "cancelled").unwrap_or(false) {
                            self.refresh_summary(&mission_id);
                        }
                        return;
                    }
                }
            }
        }
    }

    /// What the scheduler should do now. Also keeps dependent tasks' states current.
    fn plan_step(&self, mission_id: &str, running: usize) -> Result<Step> {
        let m = self.get_mission(mission_id)?;
        if m.status != "running" {
            return Ok(if running > 0 { Step::Wait } else { Step::Idle });
        }
        let mut changed = false;
        for t in &m.tasks {
            let reevaluate = matches!(t.status.as_str(), "pending" | "ready") || (t.status == "blocked" && t.internal.dep_blocked);
            if !reevaluate {
                continue;
            }
            let dep_status = |id: &String| m.tasks.iter().find(|x| x.id == *id).map(|x| (x.title.clone(), x.status.clone()));
            let bad = t.depends_on.iter().filter_map(dep_status).find(|(_, s)| matches!(s.as_str(), "failed" | "cancelled" | "blocked"));
            let all_done = t.depends_on.iter().all(|d| dep_status(d).is_some_and(|(_, s)| s == "done"));
            let (status, blocker, dep_blocked) = match bad {
                Some((title, s)) => ("blocked", Some(format!("waiting for \"{title}\", which is {s}")), true),
                None if all_done => ("ready", None, false),
                None => ("pending", None, false),
            };
            if status != t.status || blocker != t.blocker {
                let _ = self.update_task(&t.id, |x| {
                    x.status = status.into();
                    x.blocker = blocker.clone();
                    x.internal.dep_blocked = dep_blocked;
                });
                changed = true;
            }
        }
        let m = if changed { self.get_mission(mission_id)? } else { m };
        let mut ready: Vec<&MissionTask> = m.tasks.iter().filter(|t| t.status == "ready").collect();
        ready.sort_by(|a, b| b.priority.cmp(&a.priority));
        let capacity = DEFAULT_CONCURRENCY.saturating_sub(running);
        let busy = self.agents.busy.lock().unwrap().clone();
        let mut chosen: Vec<String> = Vec::new();
        let mut agents_used: HashSet<String> = HashSet::new();
        let mut waiting_for_agent = false;
        for t in ready {
            let agent = t.assignee_agent_id.clone().unwrap_or_default();
            if busy.contains(&agent) || agents_used.contains(&agent) {
                waiting_for_agent = true;
                continue;
            }
            if chosen.len() >= capacity {
                waiting_for_agent = true;
                continue;
            }
            agents_used.insert(agent);
            chosen.push(t.id.clone());
        }
        if !chosen.is_empty() {
            return Ok(Step::Launch(chosen));
        }
        if running > 0 || waiting_for_agent {
            return Ok(Step::Wait);
        }
        if !m.tasks.is_empty() && m.tasks.iter().all(|t| t.status == "done") {
            return Ok(Step::Finalize);
        }
        if m.tasks.iter().any(|t| matches!(t.status.as_str(), "awaiting_approval" | "reviewing" | "running")) {
            return Ok(Step::Idle); // waiting for the user (or for an approval integration)
        }
        Ok(Step::Fail)
    }

    /// Mark the task running and start it. Returns the task id to wait for.
    fn launch_task(self: &Arc<Self>, mission_id: &str, task_id: &str, done: tokio::sync::mpsc::UnboundedSender<String>) -> Option<String> {
        let t = self.db.get_task_row(task_id).ok()?;
        let agent = t.assignee_agent_id.clone().unwrap_or_default();
        if !self.agents.busy.lock().unwrap().insert(agent.clone()) {
            return None;
        }
        let ok = self.update_task(task_id, |x| {
            x.status = "running".into();
            x.started_at = Some(now());
            x.ended_at = None;
            x.blocker = None;
        });
        if ok.is_err() {
            self.agents.busy.lock().unwrap().remove(&agent);
            return None;
        }
        let token = self.task_token(mission_id, task_id);
        let me = self.clone();
        let (mid, tid) = (mission_id.to_string(), task_id.to_string());
        tokio::spawn(async move {
            me.run_mission_task(&mid, &tid, &agent, &token).await;
            me.agents.busy.lock().unwrap().remove(&agent);
            me.drop_task_token(&mid, &tid);
            let _ = done.send(tid);
        });
        Some(task_id.to_string())
    }

    // ============================================================== running one task

    fn task_context(&self, m: &Mission, t: &MissionTask) -> String {
        let mut s = format!("Mission goal: {}\n", m.goal);
        if let Some(plan) = m.meta.get("plan").and_then(|p| p.as_str()).filter(|p| !p.is_empty()) {
            s.push_str(&format!("Plan summary: {plan}\n"));
        }
        let deps: Vec<&MissionTask> = m.tasks.iter().filter(|x| t.depends_on.contains(&x.id)).collect();
        if !deps.is_empty() {
            s.push_str("\nResults of the tasks you depend on (already integrated into the project):\n");
            for d in deps {
                s.push_str(&format!("- \"{}\" [{}]: {}\n", d.title, d.status, clip_text(d.result.as_deref().unwrap_or("(no result)"), 1500)));
            }
        }
        let others: Vec<String> = m.tasks.iter().filter(|x| x.id != t.id).map(|x| format!("- \"{}\" ({})", x.title, x.status)).collect();
        if !others.is_empty() {
            s.push_str(&format!("\nOther tasks of this mission (done by teammates; do not redo them):\n{}\n", others.join("\n")));
        }
        s
    }

    async fn run_mission_task(self: &Arc<Self>, mission_id: &str, task_id: &str, agent_id: &str, cancel: &CancellationToken) {
        let end = match self.run_mission_task_inner(mission_id, task_id, agent_id, cancel).await {
            Ok(Some(end)) => end,
            Ok(None) => return, // already finished inside
            Err(GeniusError::Cancelled) => TaskEnd::Cancelled { result: String::new() },
            Err(e) => TaskEnd::Failed { result: String::new(), blocker: format!("internal error: {e}"), verification: Verification::default(), changeset_id: None },
        };
        self.finish_task(mission_id, task_id, end).await;
    }

    async fn run_mission_task_inner(self: &Arc<Self>, mission_id: &str, task_id: &str, agent_id: &str, cancel: &CancellationToken) -> Result<Option<TaskEnd>> {
        let mission = self.get_mission(mission_id)?;
        let task = self.db.get_task_row(task_id)?;
        let live = mission.project_id.clone();
        let Ok(agent) = self.db.get_custom_agent(agent_id) else {
            return Ok(Some(TaskEnd::Failed { result: String::new(), blocker: "the assigned agent no longer exists".into(), verification: Verification::default(), changeset_id: None }));
        };
        self.set_agent_status(&agent.id, "planning", Some(task_id), &live, Some(mission_id), &format!("starting \"{}\"", task.title));
        self.bus.record(Some(&live), Some(task_id), EventKind::Task, EventStatus::Running, format!("{}: {}", agent.name, task.title), json!({ "missionId": mission_id, "agentId": agent.id }));
        let editing = agent.can_edit();
        let iso = if editing { Some(self.create_isolated(&live, mission_id, task_id, "workspace").await?) } else { None };
        if let Some(i) = &iso {
            self.update_task(task_id, |x| {
                x.workspace = Some(i.root.to_string_lossy().to_string());
                x.internal.shadow_project_id = Some(i.shadow_pid.clone());
                x.internal.task_dir = Some(i.task_dir.to_string_lossy().to_string());
            })?;
        }
        let run_pid = iso.as_ref().map(|i| i.shadow_pid.clone()).unwrap_or_else(|| live.clone());
        let context = self.task_context(&mission, &task);
        let task_text = format!("{}\n\n{}", task.title, task.description);

        let started = std::time::Instant::now();
        let mut total = [0u64; 4];
        let mut turns = 0usize;
        let mut attempt = 0u32;
        let out = loop {
            attempt += 1;
            let mut profile = AgentProfile::from_agent(&agent, &live, Some(mission_id));
            profile.started = started;
            profile.base_usage = total;
            profile.base_steps = turns;
            let input = CustomRunInput { agent: agent.clone(), profile, project_id: run_pid.clone(), task_id: task_id.into(), task_text: task_text.clone(), context: context.clone() };
            let _ = self.update_task(task_id, |x| x.attempts = attempt);
            let out = match self.run_custom_agent(input, cancel).await {
                Ok(o) => o,
                Err(e) => crate::agent::custom_run::CustomRunOutput { end: RunEnd::Failed(e.to_string()), text: String::new(), usage: [0; 4], turns: 0, checks: vec![], acceptance: Value::Null, applied_paths: vec![], violations: vec![] },
            };
            for i in 0..4 {
                total[i] += out.usage[i];
            }
            turns += out.turns;
            match &out.end {
                RunEnd::Failed(msg) if attempt <= agent.retry_limit && !cancel.is_cancelled() => {
                    self.agent_log(&agent.id, Some(mission_id), Some(task_id), &live, "error", &format!("attempt {attempt} failed: {msg}; retrying"), json!({ "attempt": attempt }));
                    continue;
                }
                _ => break out,
            }
        };
        let _ = self.db.add_agent_usage(&agent.id, total[2], total[3], 0, 0);
        let _ = self.update_task(task_id, |x| {
            x.usage = TaskUsage { tokens: total[2], calls: total[3] };
            x.acceptance = out.acceptance.clone();
            x.violations = out.violations.clone();
        });
        self.agent_log(&agent.id, Some(mission_id), Some(task_id), &live, "result", &out.text, json!({ "usage": { "tokens": total[2], "calls": total[3] }, "turns": turns, "checks": out.checks, "acceptance": out.acceptance, "violations": out.violations }));

        let result = out.text.clone();
        match out.end {
            RunEnd::Cancelled => Ok(Some(TaskEnd::Cancelled { result })),
            RunEnd::Failed(msg) => Ok(Some(TaskEnd::Failed { result, blocker: format!("the agent failed: {msg}"), verification: Verification::default(), changeset_id: None })),
            RunEnd::Budget(why) => {
                // keep partial work as a proposed changeset the user can review
                let mut changeset_id = None;
                if let Some(i) = &iso {
                    if let Ok(Some(cs)) = self.propose_task_changes(&mission, &task, i, &format!("Partial work of {} (budget exhausted)", agent.name)) {
                        changeset_id = Some(cs);
                    }
                }
                Ok(Some(TaskEnd::Failed { result, blocker: format!("budget exhausted: {why}"), verification: Verification { notes: "Not verified: the agent ran out of budget before finishing.".into(), ..Default::default() }, changeset_id }))
            }
            RunEnd::Completed => {
                if iso.is_none() {
                    return Ok(Some(TaskEnd::Done { result, verification: Verification { notes: "Read-only task: it changed no files, so there is nothing to build or test.".into(), ..Default::default() }, changeset_id: None }));
                }
                self.update_task(task_id, |x| x.status = "reviewing".into())?;
                self.set_agent_status(&agent.id, "reviewing", Some(task_id), &live, Some(mission_id), "collecting and verifying the changes");
                let t = self.db.get_task_row(task_id)?;
                if agent.approval == "auto" {
                    Ok(Some(self.integrate_task(mission_id, &t, result, cancel).await))
                } else {
                    Ok(Some(self.hold_for_approval(&mission, &t, result)?))
                }
            }
        }
    }

    // ============================================================== workspaces

    async fn create_isolated(&self, live_pid: &str, mission_id: &str, task_id: &str, label: &str) -> Result<Isolated> {
        let live = self.db.get_project(live_pid)?;
        let task_dir = if label == "workspace" { self.data_dir().join(WORKSPACES_DIR).join(mission_id).join(task_id) } else { self.data_dir().join(WORKSPACES_DIR).join(mission_id).join(format!("{label}-{}", &uuid::Uuid::new_v4().to_string()[..8])) };
        let _ = std::fs::remove_dir_all(&task_dir);
        std::fs::create_dir_all(&task_dir)?;
        let root = task_dir.join("ws");
        let (src, dst) = (PathBuf::from(&live.path), root.clone());
        let base = tokio::task::spawn_blocking(move || isolation::copy_project(&src, &dst, true)).await.map_err(|e| GeniusError::Internal(e.to_string()))??;
        std::fs::write(task_dir.join("base.json"), serde_json::to_vec(&base)?)?;
        let root = root.canonicalize()?;
        let shadow_pid = uuid::Uuid::new_v4().to_string();
        let row = ProjectRow {
            id: shadow_pid.clone(),
            name: live.name.clone(),
            description: live.description.clone(),
            path: root.to_string_lossy().to_string(),
            kind: "mission_workspace".into(),
            spec: live.spec.clone(),
            notes: live.notes.clone(),
            status: json!({ "stage": "mission-workspace", "liveProjectId": live_pid, "label": label }),
            created_at: now(),
            updated_at: now(),
            last_opened_at: None,
        };
        self.db.insert_project(&row)?;
        Ok(Isolated { shadow_pid, task_dir, root })
    }

    fn load_isolated(&self, t: &MissionTask) -> Option<Isolated> {
        let task_dir = PathBuf::from(t.internal.task_dir.as_ref()?);
        let root = task_dir.join("ws");
        root.is_dir().then(|| Isolated { shadow_pid: t.internal.shadow_project_id.clone().unwrap_or_default(), task_dir, root })
    }

    fn load_base(&self, task_dir: &Path) -> Result<Manifest> {
        let b = std::fs::read(task_dir.join("base.json")).map_err(|_| GeniusError::NotFound("the task's base snapshot (its workspace was removed)".into()))?;
        Ok(serde_json::from_slice(&b)?)
    }

    fn remove_isolated(&self, shadow_pid: Option<&str>, task_dir: Option<&str>) {
        if let Some(d) = task_dir {
            remove_workspace(self.data_dir(), d);
        }
        if let Some(p) = shadow_pid {
            let _ = self.db.delete_project(p);
        }
    }

    fn cleanup_task_workspace(&self, t: &MissionTask) {
        self.remove_isolated(t.internal.shadow_project_id.as_deref(), t.internal.task_dir.as_deref());
    }

    /// Store the task's current changes as a *proposed* changeset against the live project
    /// (used for partial work and for tasks waiting for approval). Returns its id.
    fn propose_task_changes(&self, mission: &Mission, task: &MissionTask, iso: &Isolated, title: &str) -> Result<Option<String>> {
        let base = self.load_base(&iso.task_dir)?;
        let live = self.workspace(&mission.project_id)?;
        let (changes, _) = isolation::collect_changes(&iso.root, &base, live.root())?;
        if changes.is_empty() {
            return Ok(None);
        }
        let cs = ChangeStore::new(&self.db).create(&mission.project_id, Some(&task.id), "mission", title, &format!("Mission \"{}\" · task \"{}\"", mission.title, task.title), &changes)?;
        self.bus.record(Some(&mission.project_id), Some(&task.id), EventKind::Changeset, EventStatus::AwaitingApproval, format!("Proposed: {title}"), json!({ "changesetId": cs.id, "missionId": mission.id }));
        let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        self.update_task(&task.id, |x| {
            x.changeset_id = Some(cs.id.clone());
            x.changed_paths = paths;
        })?;
        Ok(Some(cs.id))
    }

    /// `ask` approval: the task's changes wait for the user. They are conflict-checked now so a
    /// task that can no longer be applied says so immediately.
    fn hold_for_approval(&self, mission: &Mission, task: &MissionTask, result: String) -> Result<TaskEnd> {
        let Some(iso) = self.load_isolated(task) else {
            return Ok(TaskEnd::Failed { result, blocker: "the task's workspace is missing".into(), verification: Verification::default(), changeset_id: None });
        };
        let base = self.load_base(&iso.task_dir)?;
        let live = self.workspace(&mission.project_id)?;
        let (changes, skipped) = isolation::collect_changes(&iso.root, &base, live.root())?;
        if changes.is_empty() {
            return Ok(TaskEnd::Done { result, verification: Verification { notes: "The agent changed no files.".into(), ..Default::default() }, changeset_id: None });
        }
        let conflicts = self.conflicts_for(mission, task, live.root(), &base, &changes)?;
        if !conflicts.is_empty() {
            let cs = self.propose_task_changes(mission, task, &iso, &format!("{} (conflicting)", first_line(&task.title, 60)))?;
            return Ok(TaskEnd::Blocked { result, blocker: format!("conflict: {} changed in the project (or by another task) since this task started; nothing was overwritten. Retry the task to redo it on top of the current project.", conflicts.join(", ")), conflicts, changeset_id: cs });
        }
        let cs = self.propose_task_changes(mission, task, &iso, &first_line(&task.title, 80))?;
        let mut note = String::from("Not verified yet: the build/type check runs on the integrated result when you approve.");
        if !skipped.is_empty() {
            note.push_str(&format!(" Skipped (not text): {}.", skipped.join(", ")));
        }
        Ok(TaskEnd::Awaiting { result, changeset_id: cs.unwrap_or_default(), verification: Verification { notes: note, ..Default::default() } })
    }

    fn conflicts_for(&self, mission: &Mission, task: &MissionTask, live_root: &Path, base: &Manifest, changes: &[FileChange]) -> Result<Vec<String>> {
        let mut c = isolation::detect_conflicts(live_root, base, changes);
        let pending = self.db.pending_paths(&mission.project_id, &task.id)?;
        for ch in changes {
            if pending.contains(&ch.path) && !c.contains(&ch.path) {
                c.push(ch.path.clone());
            }
        }
        c.sort();
        Ok(c)
    }

    // ============================================================== integration

    /// Collect the task's changes, check them against the live project, verify the integrated
    /// result and apply them as a checkpointed changeset. The live project is touched only by the
    /// final apply; any earlier failure leaves it as it was.
    async fn integrate_task(self: &Arc<Self>, mission_id: &str, task: &MissionTask, result: String, cancel: &CancellationToken) -> TaskEnd {
        match self.integrate_task_inner(mission_id, task, result.clone(), cancel).await {
            Ok(end) => end,
            Err(GeniusError::Cancelled) => TaskEnd::Cancelled { result },
            Err(e) => TaskEnd::Failed { result, blocker: format!("integration failed: {e}"), verification: Verification::default(), changeset_id: task.changeset_id.clone() },
        }
    }

    async fn integrate_task_inner(self: &Arc<Self>, mission_id: &str, task: &MissionTask, result: String, cancel: &CancellationToken) -> Result<TaskEnd> {
        let mission = self.db.get_mission_row(mission_id)?;
        let live_pid = mission.project_id.clone();
        let Some(iso) = self.load_isolated(task) else {
            return Ok(TaskEnd::Failed { result, blocker: "the task's isolated workspace is missing, so its changes cannot be integrated".into(), verification: Verification::default(), changeset_id: task.changeset_id.clone() });
        };
        // one integration at a time per project
        let lock = self.agents.integrate.lock().unwrap().entry(live_pid.clone()).or_default().clone();
        let _guard = tokio::select! {
            g = lock.lock() => g,
            _ = cancel.cancelled() => return Err(GeniusError::Cancelled),
        };
        if cancel.is_cancelled() {
            return Err(GeniusError::Cancelled);
        }
        self.update_mission(mission_id, |m| {
            if m.status == "running" {
                m.status = "integrating".into();
            }
        })?;
        let r = self.integrate_locked(&mission, task, &iso, result, cancel).await;
        let _ = self.update_mission(mission_id, |m| {
            if m.status == "integrating" || m.status == "verifying" {
                m.status = "running".into();
            }
        });
        r
    }

    async fn integrate_locked(self: &Arc<Self>, mission: &Mission, task: &MissionTask, iso: &Isolated, result: String, cancel: &CancellationToken) -> Result<TaskEnd> {
        let live_pid = mission.project_id.as_str();
        let live = self.workspace(live_pid)?;
        let base = self.load_base(&iso.task_dir)?;
        let (changes, skipped) = isolation::collect_changes(&iso.root, &base, live.root())?;
        if changes.is_empty() {
            return Ok(TaskEnd::Done { result, verification: Verification { notes: "The agent changed no files.".into(), ..Default::default() }, changeset_id: None });
        }
        let paths: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        self.update_task(&task.id, |x| x.changed_paths = paths.clone())?;
        let conflicts = self.conflicts_for(mission, task, live.root(), &base, &changes)?;
        if !conflicts.is_empty() {
            let cs = self.propose_task_changes(mission, task, iso, &format!("{} (conflicting)", first_line(&task.title, 60)))?;
            return Ok(TaskEnd::Blocked { result, blocker: format!("conflict: {} changed in the project (or by another task) since this task started; nothing was overwritten. Retry the task to redo it on top of the current project.", conflicts.join(", ")), conflicts, changeset_id: cs });
        }
        // the changeset (reuse the proposal when it is still the same)
        let store = ChangeStore::new(&self.db);
        let existing = task.changeset_id.as_deref().and_then(|id| store.get(id).ok().filter(|c| c.status == "proposed").and_then(|c| store.files(&c.id).ok().map(|f| (c, f))));
        let cs = match existing {
            Some((c, files)) if files == changes => c,
            other => {
                if let Some((c, _)) = other {
                    let _ = self.reject_changeset(&c.id);
                }
                let c = store.create(live_pid, Some(&task.id), "mission", &first_line(&task.title, 80), &format!("Mission \"{}\" · task \"{}\"", mission.title, task.title), &changes)?;
                self.update_task(&task.id, |x| x.changeset_id = Some(c.id.clone()))?;
                c
            }
        };
        // verify the integrated result
        self.update_mission(&mission.id, |m| {
            if m.status == "integrating" {
                m.status = "verifying".into();
            }
        })?;
        let vr = self.verify_integrated(mission, task, iso, &base, &changes, cancel).await?;
        let mut v = vr.v;
        if !skipped.is_empty() {
            v.notes.push_str(&format!(" Skipped (not text): {}.", skipped.join(", ")));
        }
        if vr.failed {
            return Ok(TaskEnd::Failed { result, blocker: format!("verification failed in the integrated result; the project was not changed. {}", v.notes.trim()), verification: v, changeset_id: Some(cs.id) });
        }
        if let Some(err) = vr.error {
            return Ok(TaskEnd::Blocked { result, blocker: format!("the changes could not be verified ({err}); the project was not changed. Retry once the check can run."), conflicts: vec![], changeset_id: Some(cs.id) });
        }
        if cancel.is_cancelled() {
            return Err(GeniusError::Cancelled);
        }
        // apply (a checkpointed changeset: History can revert it)
        match self.apply_changeset_inner(live_pid, Some(&task.id), &cs.id) {
            Ok(_) => Ok(TaskEnd::Done { result, verification: v, changeset_id: Some(cs.id) }),
            Err(e) => Ok(TaskEnd::Blocked { result, blocker: format!("the project changed while the task was being verified, so the changes were not applied: {e}"), conflicts: vec![], changeset_id: Some(cs.id) }),
        }
    }

    /// Verify "live project + this task's changes". When the live project is exactly what the
    /// task started from, the task's own workspace already is that result; otherwise a fresh copy
    /// of the live project with the changes laid over it is built and checked.
    async fn verify_integrated(self: &Arc<Self>, mission: &Mission, task: &MissionTask, iso: &Isolated, base: &Manifest, changes: &[FileChange], cancel: &CancellationToken) -> Result<VerifyResult> {
        let live = self.workspace(&mission.project_id)?;
        let unchanged = {
            let root = live.root().to_path_buf();
            tokio::task::spawn_blocking(move || isolation::compute_manifest(&root)).await.map_err(|e| GeniusError::Internal(e.to_string()))??
        } == *base;
        if unchanged {
            return self.verify_project(&iso.shadow_pid, &task.id, cancel).await;
        }
        let tmp = self.create_isolated(&mission.project_id, &mission.id, &task.id, "integration").await?;
        let r = async {
            let (root, ch) = (tmp.root.clone(), changes.to_vec());
            // package.json changed: the copy needs its own install, not the shared node_modules
            if ch.iter().any(|c| c.path == "package.json") {
                self.detach_shared_modules(&tmp.shadow_pid);
            }
            tokio::task::spawn_blocking(move || isolation::overlay(&root, &ch)).await.map_err(|e| GeniusError::Internal(e.to_string()))??;
            self.verify_project(&tmp.shadow_pid, &task.id, cancel).await
        }
        .await;
        self.remove_isolated(Some(&tmp.shadow_pid), Some(&tmp.task_dir.to_string_lossy()));
        r
    }

    /// Run the project's real typecheck, build and tests (the ones its package.json defines).
    /// Missing scripts are reported as "unknown", never as passed.
    async fn verify_project(&self, pid: &str, task_id: &str, cancel: &CancellationToken) -> Result<VerifyResult> {
        let ws = self.workspace(pid)?;
        let pkg: Value = ws.read_text_opt("package.json")?.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
        let has = |s: &str| pkg.pointer(&format!("/scripts/{s}")).is_some();
        let mut v = Verification::default();
        let mut notes: Vec<String> = Vec::new();
        let mut failed = false;
        let mut error: Option<String> = None;
        if pkg.is_null() {
            v.notes = "Not verified: the project has no package.json, so there is no build or test to run.".into();
            return Ok(VerifyResult { v, error: None, failed: false });
        }
        if !has("typecheck") && !has("build") && !has("test") {
            v.notes = "Not verified: package.json defines no typecheck, build or test script.".into();
            return Ok(VerifyResult { v, error: None, failed: false });
        }
        if self.needs_install(pid)? {
            self.detach_shared_modules(pid);
            match self.run_operation(pid, Some(task_id), Operation::Install, "Install dependencies to verify a mission task", false, cancel).await {
                Ok(out) if out.ok() => notes.push("dependencies installed".into()),
                Ok(out) => {
                    v.build = "failed".into();
                    v.notes = format!("npm install failed (exit {}).", out.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into()));
                    return Ok(VerifyResult { v, error: None, failed: true });
                }
                Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                Err(e) => return Ok(VerifyResult { v, error: Some(format!("install could not run: {e}")), failed: false }),
            }
        }
        let mut build_states: Vec<bool> = Vec::new();
        for (script, op) in [("typecheck", Operation::Typecheck), ("build", Operation::Build)] {
            if !has(script) {
                continue;
            }
            match self.run_operation(pid, Some(task_id), op, "Verify a mission task's integrated result", false, cancel).await {
                Ok(out) if out.ok() => {
                    build_states.push(true);
                    notes.push(format!("{script} passed"));
                }
                Ok(out) => {
                    build_states.push(false);
                    failed = true;
                    let diags = crate::diagnostics::parse(&out.output_tail);
                    let first = diags.first().map(|d| format!(" — {}:{} {}", d.file.clone().unwrap_or_default(), d.line.unwrap_or(0), clip_text(&d.message, 160))).unwrap_or_else(|| format!(" — {}", clip_text(out.output_tail.trim().lines().last().unwrap_or(""), 160)));
                    notes.push(format!("{script} FAILED (exit {}){first}", out.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())));
                }
                Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                Err(e) => {
                    error = Some(format!("{script} could not run: {e}"));
                    notes.push(format!("{script} not verified: {e}"));
                }
            }
        }
        if has("test") && !failed && error.is_none() {
            match self.run_operation(pid, Some(task_id), Operation::Test, "Verify a mission task's integrated result", false, cancel).await {
                Ok(out) if out.ok() => {
                    v.tests = "passed".into();
                    notes.push("tests passed".into());
                }
                Ok(out) => {
                    v.tests = "failed".into();
                    failed = true;
                    notes.push(format!("tests FAILED (exit {})", out.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())));
                }
                Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                Err(e) => {
                    error = Some(format!("tests could not run: {e}"));
                    notes.push(format!("tests not verified: {e}"));
                }
            }
        } else if !has("test") {
            notes.push("no test script (tests not run)".into());
        }
        if !build_states.is_empty() {
            v.build = if build_states.iter().all(|b| *b) { "passed" } else { "failed" }.into();
        } else if !has("typecheck") && !has("build") {
            notes.push("no typecheck/build script (build not verified)".into());
        }
        v.notes = notes.join("; ") + ".";
        Ok(VerifyResult { v, error, failed })
    }

    // ============================================================== finishing

    /// Apply a task's end state to the database, the agent's runtime state and the logs.
    async fn finish_task(self: &Arc<Self>, mission_id: &str, task_id: &str, end: TaskEnd) {
        let Ok(task) = self.db.get_task_row(task_id) else { return };
        let Ok(mission) = self.db.get_mission_row(mission_id) else { return };
        let agent = task.assignee_agent_id.clone().unwrap_or_default();
        // A task cancelled by the user while it was finishing stays cancelled.
        let user_cancelled = task.status == "cancelled";
        let (status, agent_status, note, keep_workspace, done, failed) = match &end {
            TaskEnd::Done { .. } => ("done", "completed", "task completed", false, 1, 0),
            TaskEnd::Awaiting { .. } => ("awaiting_approval", "awaiting_approval", "waiting for your approval", true, 0, 0),
            TaskEnd::Blocked { .. } => ("blocked", "blocked", "blocked", true, 0, 0),
            TaskEnd::Failed { .. } => ("failed", "failed", "task failed", false, 0, 1),
            TaskEnd::Cancelled { .. } => ("cancelled", "idle", "task cancelled", false, 0, 0),
        };
        let status = if user_cancelled { "cancelled" } else { status };
        let keep_workspace = keep_workspace && !user_cancelled;
        let updated = self.update_task(task_id, |x| {
            x.status = status.into();
            match &end {
                TaskEnd::Done { result, verification, changeset_id } => {
                    x.result = Some(result.clone());
                    x.verification = verification.clone();
                    x.changeset_id = changeset_id.clone().or(x.changeset_id.take());
                    x.blocker = None;
                }
                TaskEnd::Awaiting { result, changeset_id, verification } => {
                    x.result = Some(result.clone());
                    x.verification = verification.clone();
                    x.changeset_id = Some(changeset_id.clone());
                    x.blocker = None;
                }
                TaskEnd::Blocked { result, blocker, conflicts, changeset_id } => {
                    x.result = Some(result.clone());
                    x.blocker = Some(blocker.clone());
                    x.conflicts = conflicts.clone();
                    x.changeset_id = changeset_id.clone().or(x.changeset_id.take());
                    x.internal.dep_blocked = false;
                }
                TaskEnd::Failed { result, blocker, verification, changeset_id } => {
                    x.result = Some(result.clone());
                    x.blocker = Some(blocker.clone());
                    x.verification = verification.clone();
                    x.changeset_id = changeset_id.clone().or(x.changeset_id.take());
                }
                TaskEnd::Cancelled { result } => {
                    if !result.is_empty() {
                        x.result = Some(result.clone());
                    }
                    if x.blocker.is_none() {
                        x.blocker = Some("cancelled".into());
                    }
                }
            }
            if status != "awaiting_approval" && status != "blocked" {
                x.ended_at = Some(now());
            }
            if !keep_workspace {
                x.workspace = None;
            }
        });
        if !keep_workspace {
            self.cleanup_task_workspace(&task);
        }
        if let Ok(t) = &updated {
            let _ = self.db.add_agent_usage(&agent, 0, 0, done, failed);
            let blocker = t.blocker.clone().unwrap_or_default();
            let text = match status {
                "done" => format!("done: {}", clip_text(t.result.as_deref().unwrap_or(""), 300)),
                _ if blocker.is_empty() => note.to_string(),
                _ => format!("{note}: {blocker}"),
            };
            self.set_agent_status(&agent, agent_status, if matches!(status, "awaiting_approval" | "blocked") { Some(task_id) } else { None }, &mission.project_id, Some(mission_id), &text);
            let es = match status {
                "done" => EventStatus::Completed,
                "failed" | "blocked" => EventStatus::Failed,
                "cancelled" => EventStatus::Cancelled,
                _ => EventStatus::AwaitingApproval,
            };
            self.bus.record(Some(&mission.project_id), Some(task_id), EventKind::Task, es, format!("{}: {}", t.title, status), json!({ "missionId": mission_id, "blocker": t.blocker, "verification": t.verification }));
        }
        self.wake_driver(mission_id);
    }

    fn fail_mission(&self, mission_id: &str) {
        let summary = self.compose_summary(mission_id, None);
        let _ = self.update_mission(mission_id, |m| {
            m.status = "failed".into();
            m.summary = Some(summary);
        });
        if let Ok(m) = self.db.get_mission_row(mission_id) {
            self.bus.record(Some(&m.project_id), None, EventKind::Task, EventStatus::Failed, format!("Mission failed: {}", m.title), json!({ "missionId": m.id }));
        }
    }

    /// Every task is done: verify the whole project once more, then write the summary.
    async fn finalize_mission(self: &Arc<Self>, mission_id: &str) {
        let Ok(m) = self.get_mission(mission_id) else { return };
        let changed = m.tasks.iter().any(|t| !t.changed_paths.is_empty());
        let mut fin: Option<(String, String)> = None;
        if changed {
            let _ = self.update_mission(mission_id, |m| m.status = "verifying".into());
            let (token, _) = self.mission_ctl(mission_id);
            match self.verify_project(&m.project_id, mission_id, &token).await {
                Ok(vr) => {
                    let state = if vr.failed { "failed" } else if vr.error.is_some() { "not run" } else if vr.v.build == "unknown" && vr.v.tests == "unknown" { "unknown" } else { "passed" };
                    fin = Some((state.into(), vr.error.map(|e| format!("{} ({e})", vr.v.notes)).unwrap_or(vr.v.notes)));
                }
                Err(e) => fin = Some(("not run".into(), format!("final verification could not run: {e}"))),
            }
        }
        // the user may have cancelled or paused meanwhile
        let status_now = self.db.get_mission_row(mission_id).map(|m| m.status).unwrap_or_default();
        if status_now == "cancelled" {
            return;
        }
        let failed = fin.as_ref().is_some_and(|(s, _)| s == "failed");
        let summary = self.compose_summary(mission_id, fin.as_ref().map(|(s, n)| (s.as_str(), n.as_str())));
        let _ = self.update_mission(mission_id, |m| {
            m.status = if failed { "failed".into() } else { "completed".into() };
            m.summary = Some(summary);
        });
        self.bus.record(Some(&m.project_id), None, EventKind::Task, if failed { EventStatus::Failed } else { EventStatus::Completed }, format!("Mission {}: {}", if failed { "failed" } else { "completed" }, m.title), json!({ "missionId": m.id }));
    }

    fn refresh_summary(&self, mission_id: &str) {
        let s = self.compose_summary(mission_id, None);
        let _ = self.update_mission(mission_id, |m| m.summary = Some(s));
    }

    /// What each task did and how it was verified, from recorded state only.
    fn compose_summary(&self, mission_id: &str, final_verification: Option<(&str, &str)>) -> String {
        let Ok(m) = self.get_mission(mission_id) else { return String::new() };
        let done = m.tasks.iter().filter(|t| t.status == "done").count();
        let mut s = format!("Mission \"{}\": {} of {} task(s) done.\n", m.title, done, m.tasks.len());
        if let Some(plan) = m.meta.get("plan").and_then(|p| p.as_str()).filter(|p| !p.is_empty()) {
            s.push_str(&format!("Plan: {plan}\n"));
        }
        s.push('\n');
        for (i, t) in m.tasks.iter().enumerate() {
            let who = t.assignee_agent_id.as_deref().and_then(|a| self.db.get_custom_agent(a).ok()).map(|a| a.name).unwrap_or_else(|| "unassigned".into());
            s.push_str(&format!("{}. {} — {} ({}).", i + 1, t.title, t.status.replace('_', " "), who));
            if let Some(r) = t.result.as_deref().filter(|r| !r.trim().is_empty()) {
                s.push_str(&format!(" {}", clip_text(&r.split_whitespace().collect::<Vec<_>>().join(" "), 280)));
            }
            if !t.changed_paths.is_empty() {
                s.push_str(&format!(" Files: {}.", t.changed_paths.join(", ")));
            }
            if let Some(b) = t.blocker.as_deref().filter(|b| !b.is_empty() && t.status != "done") {
                s.push_str(&format!(" Blocker: {b}"));
            }
            if !t.changed_paths.is_empty() || t.verification.build != "unknown" || t.verification.tests != "unknown" {
                s.push_str(&format!(" Verification: build {}, tests {}. {}", t.verification.build, t.verification.tests, t.verification.notes.trim()));
            }
            if t.acceptance.get("met").and_then(|a| a.as_bool()) == Some(false) {
                s.push_str(" Acceptance criteria: NOT met.");
            }
            s.push('\n');
        }
        match final_verification {
            Some((state, notes)) => s.push_str(&format!("\nFinal verification of the integrated project: {state}. {notes}\n")),
            None if m.tasks.iter().any(|t| !t.changed_paths.is_empty() && t.status == "done") => s.push_str("\nThe integrated project was not re-verified at the end of the mission.\n"),
            None => {}
        }
        let open: Vec<&MissionTask> = m.tasks.iter().filter(|t| !matches!(t.status.as_str(), "done")).collect();
        if !open.is_empty() {
            s.push_str(&format!("\nNot completed: {}. Completed tasks stay integrated; their changes are in History and can be reverted.\n", open.iter().map(|t| format!("\"{}\" ({})", t.title, t.status.replace('_', " "))).collect::<Vec<_>>().join(", ")));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Vec<(String, String)> {
        vec![("a1".into(), "Dev".into()), ("a2".into(), "Designer".into())]
    }

    #[test]
    fn plan_validation_accepts_a_good_graph_and_resolves_names() {
        let v = json!({ "summary": "s", "tasks": [
            { "id": "t1", "title": "Build header", "description": "d", "assignee": "a1", "dependsOn": [], "priority": 5 },
            { "id": "t2", "title": "Style header", "assignee": "designer", "dependsOn": ["t1"] }
        ]});
        let (s, tasks) = validate_plan(&v, &roster()).unwrap();
        assert_eq!(s, "s");
        assert_eq!(tasks[1].assignee, "a2", "assignee by name, case-insensitive");
        assert_eq!((tasks[0].priority, tasks[1].priority), (5, 3));
        assert_eq!(tasks[1].deps, vec!["t1".to_string()]);
    }

    #[test]
    fn plan_validation_rejects_cycles_unknown_assignees_and_oversized_plans() {
        let cyc = json!({ "tasks": [{ "id": "a", "title": "A", "assignee": "a1", "dependsOn": ["b"] }, { "id": "b", "title": "B", "assignee": "a1", "dependsOn": ["a"] }] });
        assert!(validate_plan(&cyc, &roster()).unwrap_err().contains("cycle"));
        let selfdep = json!({ "tasks": [{ "id": "a", "title": "A", "assignee": "a1", "dependsOn": ["a"] }] });
        assert!(validate_plan(&selfdep, &roster()).unwrap_err().contains("itself"));
        let unknown = json!({ "tasks": [{ "id": "a", "title": "A", "assignee": "ghost" }] });
        assert!(validate_plan(&unknown, &roster()).unwrap_err().contains("not on the team"));
        let missing_dep = json!({ "tasks": [{ "id": "a", "title": "A", "assignee": "a1", "dependsOn": ["zzz"] }] });
        assert!(validate_plan(&missing_dep, &roster()).unwrap_err().contains("does not exist"));
        let many = json!({ "tasks": (0..13).map(|i| json!({ "id": format!("t{i}"), "title": "x", "assignee": "a1" })).collect::<Vec<_>>() });
        assert!(validate_plan(&many, &roster()).unwrap_err().contains("at most"));
        assert!(validate_plan(&json!({ "tasks": [] }), &roster()).is_err());
        assert!(validate_plan(&json!({ "tasks": [{ "id": "a", "assignee": "a1" }] }), &roster()).unwrap_err().contains("no title"));
        assert!(validate_plan(&json!({ "tasks": [{ "id": "a", "title": "A", "assignee": "a1" }, { "id": "a", "title": "B", "assignee": "a1" }] }), &roster()).unwrap_err().contains("duplicate"));
    }

    #[test]
    fn json_is_found_in_prose_and_fences() {
        assert!(extract_json_object("Sure!\n```json\n{\"tasks\": []}\n```").is_some());
        assert!(extract_json_object("no json").is_none());
    }

    #[test]
    fn task_extra_round_trips_through_the_database() {
        let db = Db::open_in_memory().unwrap();
        db.insert_project(&ProjectRow { id: "p".into(), name: "P".into(), description: String::new(), path: "/p".into(), kind: "k".into(), spec: None, notes: String::new(), status: json!({}), created_at: now(), updated_at: now(), last_opened_at: None }).unwrap();
        let m = Mission { id: "m".into(), project_id: "p".into(), title: "T".into(), goal: "G".into(), status: "ready".into(), created_at: now(), updated_at: now(), meta: json!({ "plan": "x" }), ..Default::default() };
        db.insert_mission_row(&m).unwrap();
        let mut t = MissionTask { id: "t".into(), mission_id: "m".into(), title: "Task".into(), status: "ready".into(), priority: 4, depends_on: vec!["u".into()], ..Default::default() };
        t.changed_paths = vec!["src/a.ts".into()];
        t.internal = TaskInternal { shadow_project_id: Some("s".into()), task_dir: Some("/d".into()), dep_blocked: true };
        db.insert_task_row(0, &t).unwrap();
        let back = db.get_task_row("t").unwrap();
        assert_eq!(back, t);
        assert_eq!(db.get_mission_row("m").unwrap().meta["plan"], "x");
        assert_eq!(db.list_mission_rows(Some("p")).unwrap().len(), 1);
        assert!(db.list_mission_rows(Some("other")).unwrap().is_empty());
    }
}
