//! SQLite persistence with versioned, forward-only migrations.
//!
//! The schema version lives in `PRAGMA user_version`. Each migration runs in a
//! transaction; a failure leaves the database at the previous version instead of
//! half-migrated. Secrets are never stored here (see `secrets.rs`).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::error::{GeniusError, Result};

const MIGRATIONS: &[&str] = &[
    // v1 — initial schema
    r#"
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE projects (
        id             TEXT PRIMARY KEY,
        name           TEXT NOT NULL,
        description    TEXT NOT NULL DEFAULT '',
        path           TEXT NOT NULL UNIQUE,
        kind           TEXT NOT NULL DEFAULT 'vite-react-ts',
        spec_json      TEXT,
        notes          TEXT NOT NULL DEFAULT '',
        status_json    TEXT NOT NULL DEFAULT '{}',
        created_at     TEXT NOT NULL,
        updated_at     TEXT NOT NULL,
        last_opened_at TEXT
    );
    CREATE TABLE messages (
        id         TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        task_id    TEXT,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL,
        created_at TEXT NOT NULL
    );
    CREATE INDEX idx_messages_project ON messages(project_id, created_at);
    CREATE TABLE activity (
        id          TEXT PRIMARY KEY,
        project_id  TEXT,
        task_id     TEXT,
        kind        TEXT NOT NULL,
        status      TEXT NOT NULL,
        title       TEXT NOT NULL,
        detail_json TEXT NOT NULL DEFAULT '{}',
        ts          TEXT NOT NULL,
        updated_ts  TEXT NOT NULL
    );
    CREATE INDEX idx_activity_project ON activity(project_id, ts);
    CREATE TABLE changesets (
        id         TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        task_id    TEXT,
        kind       TEXT NOT NULL,
        title      TEXT NOT NULL,
        summary    TEXT NOT NULL DEFAULT '',
        status     TEXT NOT NULL,
        created_at TEXT NOT NULL,
        applied_at TEXT
    );
    CREATE INDEX idx_changesets_project ON changesets(project_id, created_at);
    CREATE TABLE changeset_files (
        changeset_id TEXT NOT NULL REFERENCES changesets(id) ON DELETE CASCADE,
        path         TEXT NOT NULL,
        op           TEXT NOT NULL,
        before_text  TEXT,
        after_text   TEXT,
        PRIMARY KEY (changeset_id, path)
    );
    CREATE TABLE runs (
        id             TEXT PRIMARY KEY,
        project_id     TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        op             TEXT NOT NULL,
        command        TEXT NOT NULL,
        cwd            TEXT NOT NULL,
        reason         TEXT NOT NULL DEFAULT '',
        status         TEXT NOT NULL,
        exit_code      INTEGER,
        output_tail    TEXT NOT NULL DEFAULT '',
        started_at     TEXT NOT NULL,
        ended_at       TEXT
    );
    CREATE INDEX idx_runs_project ON runs(project_id, started_at);
    CREATE TABLE model_cache (
        base_url   TEXT PRIMARY KEY,
        fetched_at TEXT NOT NULL,
        json       TEXT NOT NULL
    );
    "#,
    // v2 — project conversations, agent provenance, memory, attachments, integrations
    r#"
    CREATE TABLE conversations (
        id         TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        title      TEXT NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX idx_conversations_project ON conversations(project_id, updated_at);
    ALTER TABLE messages ADD COLUMN conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE;
    ALTER TABLE messages ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat';
    ALTER TABLE messages ADD COLUMN meta_json TEXT NOT NULL DEFAULT '{}';
    -- Keep earlier messages: each project with history gets an "Earlier messages" conversation.
    INSERT INTO conversations(id, project_id, title, created_at, updated_at)
        SELECT 'legacy-' || project_id, project_id, 'Earlier messages', MIN(created_at), MAX(created_at)
        FROM messages GROUP BY project_id;
    UPDATE messages SET conversation_id = 'legacy-' || project_id WHERE conversation_id IS NULL;
    CREATE INDEX idx_messages_conversation ON messages(conversation_id, created_at);
    CREATE TABLE agent_steps (
        id              TEXT PRIMARY KEY,
        project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        task_id         TEXT NOT NULL,
        conversation_id TEXT,
        agent           TEXT NOT NULL,
        seq             INTEGER NOT NULL,
        tool            TEXT NOT NULL,
        args_json       TEXT NOT NULL DEFAULT '{}',
        ok              INTEGER NOT NULL,
        summary         TEXT NOT NULL DEFAULT '',
        result_json     TEXT NOT NULL DEFAULT '{}',
        started_at      TEXT NOT NULL,
        ended_at        TEXT NOT NULL
    );
    CREATE INDEX idx_agent_steps_task ON agent_steps(task_id, seq);
    CREATE TABLE memories (
        id         TEXT PRIMARY KEY,
        project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        kind       TEXT NOT NULL,
        content    TEXT NOT NULL,
        source     TEXT NOT NULL,
        evidence   TEXT NOT NULL DEFAULT '',
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE INDEX idx_memories_project ON memories(project_id, updated_at);
    CREATE TABLE attachments (
        id              TEXT PRIMARY KEY,
        project_id      TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        conversation_id TEXT,
        name            TEXT NOT NULL,
        kind            TEXT NOT NULL,
        mime            TEXT NOT NULL,
        size            INTEGER NOT NULL,
        scope           TEXT NOT NULL,
        status          TEXT NOT NULL,
        error           TEXT,
        text            TEXT NOT NULL DEFAULT '',
        data_b64        TEXT,
        meta_json       TEXT NOT NULL DEFAULT '{}',
        created_at      TEXT NOT NULL
    );
    CREATE INDEX idx_attachments_project ON attachments(project_id, created_at);
    CREATE TABLE integrations (
        id         TEXT PRIMARY KEY,
        meta_json  TEXT NOT NULL DEFAULT '{}',
        updated_at TEXT NOT NULL
    );
    "#,
];

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: String,
    pub kind: String,
    pub spec: Option<Value>,
    pub notes: String,
    pub status: Value,
    pub created_at: String,
    pub updated_at: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRow {
    pub id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// chat | answer | proposal | report | error | note
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub message_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentStepRow {
    pub id: String,
    pub project_id: String,
    pub task_id: String,
    pub conversation_id: Option<String>,
    pub agent: String,
    pub seq: i64,
    pub tool: String,
    pub args: Value,
    pub ok: bool,
    pub summary: String,
    pub result: Value,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRow {
    pub id: String,
    pub project_id: String,
    /// decision | convention | fix | constraint | problem | integration | dependency
    pub kind: String,
    pub content: String,
    /// user (stated/approved by the user) | repo (verified from repository evidence) | agent (model inference)
    pub source: String,
    pub evidence: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    pub id: String,
    pub project_id: String,
    pub op: String,
    pub command: String,
    pub cwd: String,
    pub reason: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub output_tail: String,
    pub started_at: String,
    pub ended_at: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        migrate(&mut conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn().query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    // ---------- settings ----------

    pub fn get_setting(&self, key: &str) -> Result<Option<Value>> {
        let v: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?;
        Ok(match v {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        })
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value.to_string()],
        )?;
        Ok(())
    }

    pub fn all_settings(&self) -> Result<serde_json::Map<String, Value>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut map = serde_json::Map::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, serde_json::from_str(&v).unwrap_or(Value::Null));
        }
        Ok(map)
    }

    // ---------- projects ----------

    pub fn insert_project(&self, p: &ProjectRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO projects(id,name,description,path,kind,spec_json,notes,status_json,created_at,updated_at,last_opened_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                p.id,
                p.name,
                p.description,
                p.path,
                p.kind,
                p.spec.as_ref().map(|s| s.to_string()),
                p.notes,
                p.status.to_string(),
                p.created_at,
                p.updated_at,
                p.last_opened_at
            ],
        )?;
        Ok(())
    }

    fn row_to_project(r: &rusqlite::Row) -> rusqlite::Result<ProjectRow> {
        let spec: Option<String> = r.get("spec_json")?;
        let status: String = r.get("status_json")?;
        Ok(ProjectRow {
            id: r.get("id")?,
            name: r.get("name")?,
            description: r.get("description")?,
            path: r.get("path")?,
            kind: r.get("kind")?,
            spec: spec.and_then(|s| serde_json::from_str(&s).ok()),
            notes: r.get("notes")?,
            status: serde_json::from_str(&status).unwrap_or_else(|_| Value::Object(Default::default())),
            created_at: r.get("created_at")?,
            updated_at: r.get("updated_at")?,
            last_opened_at: r.get("last_opened_at")?,
        })
    }

    pub fn get_project(&self, id: &str) -> Result<ProjectRow> {
        self.conn()
            .query_row("SELECT * FROM projects WHERE id = ?1", [id], Self::row_to_project)
            .optional()?
            .ok_or_else(|| GeniusError::NotFound(format!("project {id}")))
    }

    pub fn find_project_by_path(&self, path: &str) -> Result<Option<ProjectRow>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM projects WHERE path = ?1", [path], Self::row_to_project)
            .optional()?)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM projects WHERE kind != 'chat' ORDER BY COALESCE(last_opened_at, updated_at) DESC", // 'chat' = the hidden general workspace
        )?;
        let rows = stmt.query_map([], Self::row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn update_project_spec(&self, id: &str, spec: &Value) -> Result<()> {
        self.conn().execute(
            "UPDATE projects SET spec_json = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, spec.to_string(), now()],
        )?;
        Ok(())
    }

    pub fn update_project_notes(&self, id: &str, notes: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE projects SET notes = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, notes, now()],
        )?;
        Ok(())
    }

    pub fn rename_project(&self, id: &str, name: &str, description: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE projects SET name = ?2, description = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, name, description, now()],
        )?;
        Ok(())
    }

    /// Merge `patch` into the project's status object (e.g. `{ "build": {...} }`).
    pub fn patch_project_status(&self, id: &str, patch: &Value) -> Result<Value> {
        let conn = self.conn();
        let current: String = conn
            .query_row("SELECT status_json FROM projects WHERE id = ?1", [id], |r| r.get(0))
            .optional()?
            .ok_or_else(|| GeniusError::NotFound(format!("project {id}")))?;
        let mut status: Value = serde_json::from_str(&current).unwrap_or_else(|_| Value::Object(Default::default()));
        if let (Some(obj), Some(p)) = (status.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        conn.execute(
            "UPDATE projects SET status_json = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.to_string(), now()],
        )?;
        Ok(status)
    }

    pub fn touch_project_opened(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("UPDATE projects SET last_opened_at = ?2 WHERE id = ?1", params![id, now()])?;
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM activity WHERE project_id = ?1", [id])?;
        conn.execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    // ---------- messages ----------

    /// Add a message to the project's most recent conversation (created if none exists).
    pub fn add_message(&self, project_id: &str, task_id: Option<&str>, role: &str, content: &str) -> Result<MessageRow> {
        let conv = self.current_conversation(project_id)?;
        self.add_message_in(&conv.id, project_id, task_id, role, "chat", content, &Value::Object(Default::default()))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add_message_in(&self, conversation_id: &str, project_id: &str, task_id: Option<&str>, role: &str, kind: &str, content: &str, meta: &Value) -> Result<MessageRow> {
        let m = MessageRow {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            task_id: task_id.map(String::from),
            role: role.into(),
            content: content.into(),
            created_at: now(),
            conversation_id: Some(conversation_id.into()),
            kind: kind.into(),
            meta: meta.clone(),
        };
        let conn = self.conn();
        conn.execute(
            "INSERT INTO messages(id,project_id,task_id,role,content,created_at,conversation_id,kind,meta_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![m.id, m.project_id, m.task_id, m.role, m.content, m.created_at, m.conversation_id, m.kind, m.meta.to_string()],
        )?;
        conn.execute("UPDATE conversations SET updated_at = ?2 WHERE id = ?1", params![conversation_id, m.created_at])?;
        Ok(m)
    }

    pub fn update_message_meta(&self, id: &str, meta: &Value) -> Result<()> {
        self.conn().execute("UPDATE messages SET meta_json = ?2 WHERE id = ?1", params![id, meta.to_string()])?;
        Ok(())
    }

    fn row_to_message(r: &rusqlite::Row) -> rusqlite::Result<MessageRow> {
        let meta: String = r.get("meta_json")?;
        Ok(MessageRow {
            id: r.get("id")?,
            project_id: r.get("project_id")?,
            task_id: r.get("task_id")?,
            role: r.get("role")?,
            content: r.get("content")?,
            created_at: r.get("created_at")?,
            conversation_id: r.get("conversation_id")?,
            kind: r.get("kind")?,
            meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        })
    }

    pub fn list_conversation_messages(&self, conversation_id: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM (SELECT *, rowid AS seq_ FROM messages WHERE conversation_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)
             ORDER BY created_at ASC, seq_ ASC",
        )?;
        let rows = stmt.query_map(params![conversation_id, limit as i64], Self::row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- conversations ----------

    pub fn create_conversation(&self, project_id: &str, title: &str) -> Result<ConversationRow> {
        let t = now();
        let c = ConversationRow { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), title: title.into(), created_at: t.clone(), updated_at: t, message_count: 0 };
        self.conn().execute(
            "INSERT INTO conversations(id,project_id,title,created_at,updated_at) VALUES(?1,?2,?3,?4,?5)",
            params![c.id, c.project_id, c.title, c.created_at, c.updated_at],
        )?;
        Ok(c)
    }

    pub fn list_conversations(&self, project_id: &str) -> Result<Vec<ConversationRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT c.*, (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id) AS n
             FROM conversations c WHERE c.project_id = ?1 ORDER BY c.updated_at DESC, c.rowid DESC",
        )?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok(ConversationRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                title: r.get("title")?,
                created_at: r.get("created_at")?,
                updated_at: r.get("updated_at")?,
                message_count: r.get("n")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_conversation(&self, id: &str) -> Result<ConversationRow> {
        self.list_conversations_where("c.id = ?1", id)?.into_iter().next().ok_or_else(|| GeniusError::NotFound(format!("conversation {id}")))
    }

    fn list_conversations_where(&self, cond: &str, arg: &str) -> Result<Vec<ConversationRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT c.*, (SELECT COUNT(*) FROM messages m WHERE m.conversation_id = c.id) AS n FROM conversations c WHERE {cond}"
        ))?;
        let rows = stmt.query_map(params![arg], |r| {
            Ok(ConversationRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                title: r.get("title")?,
                created_at: r.get("created_at")?,
                updated_at: r.get("updated_at")?,
                message_count: r.get("n")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Most recently active conversation, creating "Project conversation" if the project has none.
    pub fn current_conversation(&self, project_id: &str) -> Result<ConversationRow> {
        match self.list_conversations(project_id)?.into_iter().next() {
            Some(c) => Ok(c),
            None => self.create_conversation(project_id, "Project conversation"),
        }
    }

    pub fn rename_conversation(&self, id: &str, title: &str) -> Result<()> {
        self.conn().execute("UPDATE conversations SET title = ?2 WHERE id = ?1", params![id, title])?;
        Ok(())
    }

    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM messages WHERE conversation_id = ?1", params![id])?;
        conn.execute("DELETE FROM conversations WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Delete the given messages of a conversation (used when rewinding).
    pub fn delete_messages(&self, conversation_id: &str, ids: &[String]) -> Result<usize> {
        let conn = self.conn();
        let mut n = 0;
        for id in ids {
            n += conn.execute("DELETE FROM messages WHERE id = ?1 AND conversation_id = ?2", params![id, conversation_id])?;
        }
        Ok(n)
    }

    // ---------- agent steps (provenance of every tool call) ----------

    pub fn insert_agent_step(&self, s: &AgentStepRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO agent_steps(id,project_id,task_id,conversation_id,agent,seq,tool,args_json,ok,summary,result_json,started_at,ended_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![s.id, s.project_id, s.task_id, s.conversation_id, s.agent, s.seq, s.tool, s.args.to_string(), s.ok as i64, s.summary, s.result.to_string(), s.started_at, s.ended_at],
        )?;
        Ok(())
    }

    pub fn list_agent_steps(&self, task_id: &str) -> Result<Vec<AgentStepRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM agent_steps WHERE task_id = ?1 ORDER BY seq ASC, rowid ASC")?;
        let rows = stmt.query_map(params![task_id], |r| {
            let args: String = r.get("args_json")?;
            let result: String = r.get("result_json")?;
            Ok(AgentStepRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                task_id: r.get("task_id")?,
                conversation_id: r.get("conversation_id")?,
                agent: r.get("agent")?,
                seq: r.get("seq")?,
                tool: r.get("tool")?,
                args: serde_json::from_str(&args).unwrap_or(Value::Null),
                ok: r.get::<_, i64>("ok")? != 0,
                summary: r.get("summary")?,
                result: serde_json::from_str(&result).unwrap_or(Value::Null),
                started_at: r.get("started_at")?,
                ended_at: r.get("ended_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- project memory ----------

    pub fn add_memory(&self, project_id: &str, kind: &str, content: &str, source: &str, evidence: &str) -> Result<MemoryRow> {
        let t = now();
        let m = MemoryRow { id: uuid::Uuid::new_v4().to_string(), project_id: project_id.into(), kind: kind.into(), content: content.into(), source: source.into(), evidence: evidence.into(), created_at: t.clone(), updated_at: t };
        self.conn().execute(
            "INSERT INTO memories(id,project_id,kind,content,source,evidence,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![m.id, m.project_id, m.kind, m.content, m.source, m.evidence, m.created_at, m.updated_at],
        )?;
        Ok(m)
    }

    pub fn list_memories(&self, project_id: &str) -> Result<Vec<MemoryRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM memories WHERE project_id = ?1 ORDER BY updated_at DESC, rowid DESC")?;
        let rows = stmt.query_map(params![project_id], |r| {
            Ok(MemoryRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                kind: r.get("kind")?,
                content: r.get("content")?,
                source: r.get("source")?,
                evidence: r.get("evidence")?,
                created_at: r.get("created_at")?,
                updated_at: r.get("updated_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn update_memory(&self, id: &str, content: &str, source: &str, evidence: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE memories SET content = ?2, source = ?3, evidence = ?4, updated_at = ?5 WHERE id = ?1",
            params![id, content, source, evidence, now()],
        )?;
        Ok(())
    }

    pub fn delete_memory(&self, project_id: &str, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM memories WHERE id = ?1 AND project_id = ?2", params![id, project_id])?;
        Ok(())
    }

    // ---------- integrations (non-secret metadata only; secrets live in the OS store) ----------

    pub fn get_integration(&self, id: &str) -> Result<Option<Value>> {
        let conn = self.conn();
        let v: Option<String> = conn.query_row("SELECT meta_json FROM integrations WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        Ok(v.and_then(|s| serde_json::from_str(&s).ok()))
    }

    pub fn set_integration(&self, id: &str, meta: &Value) -> Result<()> {
        self.conn().execute(
            "INSERT INTO integrations(id, meta_json, updated_at) VALUES(?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET meta_json = excluded.meta_json, updated_at = excluded.updated_at",
            params![id, meta.to_string(), now()],
        )?;
        Ok(())
    }

    pub fn delete_integration(&self, id: &str) -> Result<()> {
        self.conn().execute("DELETE FROM integrations WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn list_messages(&self, project_id: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM (SELECT * FROM messages WHERE project_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![project_id, limit as i64], Self::row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---------- runs ----------

    pub fn insert_run(&self, r: &RunRow) -> Result<()> {
        self.conn().execute(
            "INSERT INTO runs(id,project_id,op,command,cwd,reason,status,exit_code,output_tail,started_at,ended_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![r.id, r.project_id, r.op, r.command, r.cwd, r.reason, r.status, r.exit_code, r.output_tail, r.started_at, r.ended_at],
        )?;
        Ok(())
    }

    pub fn finish_run(&self, id: &str, status: &str, exit_code: Option<i32>, output_tail: &str) -> Result<()> {
        self.conn().execute(
            "UPDATE runs SET status = ?2, exit_code = ?3, output_tail = ?4, ended_at = ?5 WHERE id = ?1",
            params![id, status, exit_code, output_tail, now()],
        )?;
        Ok(())
    }

    pub fn list_runs(&self, project_id: &str, limit: usize) -> Result<Vec<RunRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT * FROM runs WHERE project_id = ?1 ORDER BY started_at DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![project_id, limit as i64], |r| {
            Ok(RunRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                op: r.get("op")?,
                command: r.get("command")?,
                cwd: r.get("cwd")?,
                reason: r.get("reason")?,
                status: r.get("status")?,
                exit_code: r.get("exit_code")?,
                output_tail: r.get("output_tail")?,
                started_at: r.get("started_at")?,
                ended_at: r.get("ended_at")?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Runs left in `running` state by a crash or forced exit are marked as interrupted on startup,
    /// so the UI never shows a stale "running" status.
    pub fn mark_orphaned_runs(&self) -> Result<usize> {
        Ok(self.conn().execute(
            "UPDATE runs SET status = 'interrupted', ended_at = ?1 WHERE status = 'running'",
            [now()],
        )?)
    }

    // ---------- model cache ----------

    pub fn put_model_cache(&self, base_url: &str, json: &Value) -> Result<String> {
        let ts = now();
        self.conn().execute(
            "INSERT INTO model_cache(base_url, fetched_at, json) VALUES(?1,?2,?3)
             ON CONFLICT(base_url) DO UPDATE SET fetched_at = excluded.fetched_at, json = excluded.json",
            params![base_url, ts, json.to_string()],
        )?;
        Ok(ts)
    }

    pub fn get_model_cache(&self, base_url: &str) -> Result<Option<(String, Value)>> {
        let row: Option<(String, String)> = self
            .conn()
            .query_row(
                "SELECT fetched_at, json FROM model_cache WHERE base_url = ?1",
                [base_url],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(match row {
            Some((ts, j)) => Some((ts, serde_json::from_str(&j)?)),
            None => None,
        })
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version as usize > MIGRATIONS.len() {
        return Err(GeniusError::Internal(format!(
            "database schema v{version} is newer than this version of Genius supports (v{}). Refusing to modify it.",
            MIGRATIONS.len()
        )));
    }
    for (idx, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (idx + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, path: &str) -> ProjectRow {
        ProjectRow {
            id: id.into(),
            name: "Demo".into(),
            description: "d".into(),
            path: path.into(),
            kind: "vite-react-ts".into(),
            spec: None,
            notes: String::new(),
            status: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
            last_opened_at: None,
        }
    }

    #[test]
    fn migrates_fresh_db_to_latest() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
    }

    #[test]
    fn reopening_file_db_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.db");
        {
            let db = Db::open(&path).unwrap();
            db.insert_project(&sample("p1", "/tmp/x")).unwrap();
            db.set_setting("theme", &serde_json::json!("dark")).unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.get_project("p1").unwrap().name, "Demo");
        assert_eq!(db.get_setting("theme").unwrap(), Some(serde_json::json!("dark")));
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.db");
        {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "user_version", 999).unwrap();
        }
        assert!(Db::open(&path).is_err());
    }

    #[test]
    fn status_patch_merges() {
        let db = Db::open_in_memory().unwrap();
        db.insert_project(&sample("p1", "/a")).unwrap();
        db.patch_project_status("p1", &serde_json::json!({"build": {"ok": true}})).unwrap();
        let s = db.patch_project_status("p1", &serde_json::json!({"preview": "stopped"})).unwrap();
        assert_eq!(s["build"]["ok"], true);
        assert_eq!(s["preview"], "stopped");
    }

    #[test]
    fn delete_cascades() {
        let db = Db::open_in_memory().unwrap();
        db.insert_project(&sample("p1", "/a")).unwrap();
        db.add_message("p1", None, "user", "hello").unwrap();
        db.delete_project("p1").unwrap();
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn v1_database_upgrades_keeping_messages_in_a_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("g.db");
        {
            let mut conn = Connection::open(&path).unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute_batch(MIGRATIONS[0]).unwrap();
            tx.pragma_update(None, "user_version", 1i64).unwrap();
            tx.execute("INSERT INTO projects(id,name,path,created_at,updated_at) VALUES('p','P','/x','2026-01-01','2026-01-01')", []).unwrap();
            tx.execute("INSERT INTO messages(id,project_id,role,content,created_at) VALUES('m1','p','user','Add a footer','2026-01-02')", []).unwrap();
            tx.commit().unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let convs = db.list_conversations("p").unwrap();
        assert_eq!((convs.len(), convs[0].title.as_str(), convs[0].message_count), (1, "Earlier messages", 1));
        let msgs = db.list_conversation_messages(&convs[0].id, 10).unwrap();
        assert_eq!((msgs[0].content.as_str(), msgs[0].kind.as_str()), ("Add a footer", "chat"));
        // New messages without an explicit conversation go to the most recent one.
        db.add_message("p", None, "assistant", "Done").unwrap();
        assert_eq!(db.list_conversation_messages(&convs[0].id, 10).unwrap().len(), 2);
    }
}
