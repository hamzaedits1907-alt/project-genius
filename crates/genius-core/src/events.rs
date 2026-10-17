//! Structured, event-driven activity system.
//!
//! Every real operation (AI request, file write, process run, git command, approval)
//! is represented as an [`ActivityEvent`] whose status transitions are persisted and
//! pushed to the UI. Nothing here is synthesized: an event is created by the code
//! that performs the operation, and its final status is set from the operation's
//! actual result.
//!
//! High-frequency streams (AI token deltas, process output lines) are pushed on
//! separate ephemeral channels and are not persisted as individual events.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::db::{now, Db};
use crate::redact::redact;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Task,
    Ai,
    Plan,
    FileRead,
    FileWrite,
    Changeset,
    Command,
    Install,
    Build,
    Test,
    Preview,
    Git,
    Github,
    Approval,
    Export,
    Diagnosis,
}

impl EventKind {
    pub fn as_str(&self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
    AwaitingApproval,
}

impl EventStatus {
    pub fn as_str(&self) -> String {
        serde_json::to_value(self).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
    }
    pub fn is_terminal(&self) -> bool {
        matches!(self, EventStatus::Completed | EventStatus::Failed | EventStatus::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEvent {
    pub id: String,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub kind: EventKind,
    pub status: EventStatus,
    pub title: String,
    pub detail: Value,
    pub ts: String,
    pub updated_ts: String,
}

/// Sink for pushing payloads to the UI. The Tauri layer implements this with
/// `AppHandle::emit`; tests use [`RecordingEmitter`].
pub trait Emitter: Send + Sync {
    fn emit(&self, channel: &str, payload: Value);
}

pub mod channels {
    pub const ACTIVITY: &str = "genius://activity";
    pub const AI_DELTA: &str = "genius://ai-delta";
    /// Reasoning ("thinking") deltas: `{ taskId, projectId, text }`, batched like AI_DELTA.
    pub const AI_THINKING: &str = "genius://ai-thinking";
    pub const FILE_STREAM: &str = "genius://file-stream";
    pub const PROCESS_OUTPUT: &str = "genius://process-output";
    pub const PROCESS_STATE: &str = "genius://process-state";
    pub const FS_CHANGED: &str = "genius://fs-changed";
    pub const PREVIEW: &str = "genius://preview";
    pub const PREVIEW_ERRORS: &str = "genius://preview-errors";
}

#[derive(Default)]
pub struct RecordingEmitter {
    pub events: std::sync::Mutex<Vec<(String, Value)>>,
}

impl Emitter for RecordingEmitter {
    fn emit(&self, channel: &str, payload: Value) {
        self.events.lock().unwrap().push((channel.to_string(), payload));
    }
}

impl RecordingEmitter {
    pub fn on(&self, channel: &str) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(c, _)| c == channel)
            .map(|(_, v)| v.clone())
            .collect()
    }
}

pub struct NullEmitter;
impl Emitter for NullEmitter {
    fn emit(&self, _: &str, _: Value) {}
}

const MAX_EVENTS_PER_PROJECT: i64 = 3000;

#[derive(Clone)]
pub struct EventBus {
    db: Arc<Db>,
    emitter: Arc<dyn Emitter>,
}

impl EventBus {
    pub fn new(db: Arc<Db>, emitter: Arc<dyn Emitter>) -> Self {
        Self { db, emitter }
    }

    pub fn emitter(&self) -> &Arc<dyn Emitter> {
        &self.emitter
    }

    /// Push an ephemeral payload (not persisted).
    pub fn push(&self, channel: &str, payload: Value) {
        self.emitter.emit(channel, payload);
    }

    pub fn start(
        &self,
        project_id: Option<&str>,
        task_id: Option<&str>,
        kind: EventKind,
        status: EventStatus,
        title: impl Into<String>,
        detail: Value,
    ) -> EventHandle {
        let ts = now();
        let ev = ActivityEvent {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.map(String::from),
            task_id: task_id.map(String::from),
            kind,
            status,
            title: redact(&title.into()),
            detail: redact_value(detail),
            ts: ts.clone(),
            updated_ts: ts,
        };
        self.persist(&ev);
        EventHandle { bus: self.clone(), ev }
    }

    /// Convenience: record an operation that already finished.
    pub fn record(
        &self,
        project_id: Option<&str>,
        task_id: Option<&str>,
        kind: EventKind,
        status: EventStatus,
        title: impl Into<String>,
        detail: Value,
    ) -> ActivityEvent {
        self.start(project_id, task_id, kind, status, title, detail).ev
    }

    fn persist(&self, ev: &ActivityEvent) {
        let res = self.db.conn().execute(
            "INSERT INTO activity(id,project_id,task_id,kind,status,title,detail_json,ts,updated_ts)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET status=excluded.status, title=excluded.title,
               detail_json=excluded.detail_json, updated_ts=excluded.updated_ts",
            rusqlite::params![
                ev.id,
                ev.project_id,
                ev.task_id,
                ev.kind.as_str(),
                ev.status.as_str(),
                ev.title,
                ev.detail.to_string(),
                ev.ts,
                ev.updated_ts
            ],
        );
        if let Err(e) = res {
            log::error!("failed to persist activity event: {e}");
        }
        self.emitter.emit(channels::ACTIVITY, serde_json::to_value(ev).unwrap_or(Value::Null));
    }

    pub fn list(&self, project_id: Option<&str>, limit: usize) -> crate::error::Result<Vec<ActivityEvent>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM (SELECT *, rowid AS seq FROM activity WHERE (?1 IS NULL AND project_id IS NULL) OR project_id = ?1
               ORDER BY ts DESC, rowid DESC LIMIT ?2) ORDER BY ts ASC, seq ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![project_id, limit as i64], |r| {
            let kind: String = r.get("kind")?;
            let status: String = r.get("status")?;
            let detail: String = r.get("detail_json")?;
            Ok(ActivityEvent {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                task_id: r.get("task_id")?,
                kind: serde_json::from_value(Value::String(kind)).unwrap_or(EventKind::Task),
                status: serde_json::from_value(Value::String(status)).unwrap_or(EventStatus::Failed),
                title: r.get("title")?,
                detail: serde_json::from_str(&detail).unwrap_or(Value::Null),
                ts: r.get("ts")?,
                updated_ts: r.get("updated_ts")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Events still marked running/queued/awaiting from a previous session can never complete;
    /// mark them as cancelled so history stays truthful.
    pub fn close_stale(&self) -> crate::error::Result<usize> {
        Ok(self.db.conn().execute(
            "UPDATE activity SET status = 'cancelled', updated_ts = ?1,
               detail_json = json_set(detail_json, '$.note', 'Interrupted: Genius was closed before this finished.')
             WHERE status IN ('running','queued','awaiting_approval') AND kind != 'changeset'",
            [now()],
        )?)
    }

    /// When a proposed changeset is applied, rejected or superseded, resolve the
    /// "awaiting approval" events that announced it so the feed stays truthful.
    pub fn resolve_changeset(&self, changeset_id: &str, status: EventStatus, outcome: &str) {
        let ids: Vec<String> = {
            let conn = self.db.conn();
            let Ok(mut stmt) = conn.prepare(
                "SELECT id FROM activity WHERE kind = 'changeset' AND status = 'awaiting_approval'
                   AND json_extract(detail_json, '$.changesetId') = ?1",
            ) else {
                return;
            };
            let Ok(rows) = stmt.query_map([changeset_id], |r| r.get::<_, String>(0)) else { return };
            rows.flatten().collect()
        };
        for id in ids {
            let found = self.db.conn().query_row(
                "SELECT project_id, task_id, title, detail_json, ts FROM activity WHERE id = ?1",
                [&id],
                |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?)),
            );
            if let Ok((project_id, task_id, title, detail, ts)) = found {
                let mut detail: Value = serde_json::from_str(&detail).unwrap_or(json!({}));
                detail["resolution"] = json!(outcome);
                let ev = ActivityEvent { id, project_id, task_id, kind: EventKind::Changeset, status, title, detail, ts, updated_ts: now() };
                self.persist(&ev);
            }
        }
    }

    pub fn prune(&self, project_id: &str) {
        let _ = self.db.conn().execute(
            "DELETE FROM activity WHERE project_id = ?1 AND id NOT IN
               (SELECT id FROM activity WHERE project_id = ?1 ORDER BY ts DESC LIMIT ?2)",
            rusqlite::params![project_id, MAX_EVENTS_PER_PROJECT],
        );
    }
}

/// A live event whose status can be advanced as the underlying operation progresses.
pub struct EventHandle {
    bus: EventBus,
    ev: ActivityEvent,
}

impl EventHandle {
    pub fn id(&self) -> &str {
        &self.ev.id
    }

    pub fn event(&self) -> &ActivityEvent {
        &self.ev
    }

    pub fn update(&mut self, status: EventStatus, detail_patch: Value) {
        self.ev.status = status;
        merge(&mut self.ev.detail, redact_value(detail_patch));
        self.ev.updated_ts = now();
        self.bus.persist(&self.ev);
    }

    pub fn set_title(&mut self, title: impl Into<String>) {
        self.ev.title = redact(&title.into());
    }

    pub fn complete(mut self, detail_patch: Value) -> ActivityEvent {
        self.update(EventStatus::Completed, detail_patch);
        self.ev
    }

    pub fn fail(mut self, error: &crate::error::GeniusError) -> ActivityEvent {
        let status = if matches!(error, crate::error::GeniusError::Cancelled) {
            EventStatus::Cancelled
        } else {
            EventStatus::Failed
        };
        self.update(status, json!({ "error": { "kind": error.kind(), "message": error.to_string() } }));
        self.ev
    }

    pub fn finish<T>(self, result: &crate::error::Result<T>, ok_detail: Value) -> ActivityEvent {
        match result {
            Ok(_) => self.complete(ok_detail),
            Err(e) => self.fail(e),
        }
    }
}

fn merge(base: &mut Value, patch: Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                b.insert(k, v);
            }
        }
        (b, p) if !p.is_null() => *b = p,
        _ => {}
    }
}

pub fn redact_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(redact_value).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, redact_value(v))).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_is_persisted_and_emitted() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let rec = Arc::new(RecordingEmitter::default());
        let bus = EventBus::new(db, rec.clone());
        let mut h = bus.start(None, Some("t1"), EventKind::Build, EventStatus::Running, "npm run build", json!({"cwd": "/x"}));
        h.update(EventStatus::Running, json!({"pid": 5}));
        let id = h.id().to_string();
        h.complete(json!({"exitCode": 0}));
        let all = bus.list(None, 10).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, id);
        assert_eq!(all[0].status, EventStatus::Completed);
        assert_eq!(all[0].detail["pid"], 5);
        assert_eq!(all[0].detail["exitCode"], 0);
        assert_eq!(rec.on(channels::ACTIVITY).len(), 3);
    }

    #[test]
    fn details_are_redacted() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let bus = EventBus::new(db, Arc::new(NullEmitter));
        let ev = bus.record(None, None, EventKind::Ai, EventStatus::Failed, "x", json!({"msg": "Bearer nvapi-123456789abcdef"}));
        assert!(!ev.detail.to_string().contains("nvapi-1234"));
    }

    #[test]
    fn proposals_resolve_and_survive_restart() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let bus = EventBus::new(db, Arc::new(NullEmitter));
        bus.record(None, None, EventKind::Changeset, EventStatus::AwaitingApproval, "Review", json!({"changesetId": "c1"}));
        bus.record(None, None, EventKind::Changeset, EventStatus::AwaitingApproval, "Review", json!({"changesetId": "c2"}));
        bus.close_stale().unwrap();
        assert!(bus.list(None, 5).unwrap().iter().all(|e| e.status == EventStatus::AwaitingApproval), "still pending after restart");
        bus.resolve_changeset("c1", EventStatus::Completed, "applied");
        let all = bus.list(None, 5).unwrap();
        assert_eq!(all[0].status, EventStatus::Completed);
        assert_eq!(all[0].detail["resolution"], "applied");
        assert_eq!(all[1].status, EventStatus::AwaitingApproval);
    }

    #[test]
    fn stale_events_are_closed() {
        let db = Arc::new(Db::open_in_memory().unwrap());
        let bus = EventBus::new(db, Arc::new(NullEmitter));
        let _h = bus.start(None, None, EventKind::Preview, EventStatus::Running, "dev server", json!({}));
        assert_eq!(bus.close_stale().unwrap(), 1);
        assert_eq!(bus.list(None, 5).unwrap()[0].status, EventStatus::Cancelled);
    }
}
