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
            "SELECT * FROM projects ORDER BY COALESCE(last_opened_at, updated_at) DESC",
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

    pub fn add_message(&self, project_id: &str, task_id: Option<&str>, role: &str, content: &str) -> Result<MessageRow> {
        let m = MessageRow {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            task_id: task_id.map(String::from),
            role: role.into(),
            content: content.into(),
            created_at: now(),
        };
        self.conn().execute(
            "INSERT INTO messages(id,project_id,task_id,role,content,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
            params![m.id, m.project_id, m.task_id, m.role, m.content, m.created_at],
        )?;
        Ok(m)
    }

    pub fn list_messages(&self, project_id: &str, limit: usize) -> Result<Vec<MessageRow>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT * FROM (SELECT * FROM messages WHERE project_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map(params![project_id, limit as i64], |r| {
            Ok(MessageRow {
                id: r.get("id")?,
                project_id: r.get("project_id")?,
                task_id: r.get("task_id")?,
                role: r.get("role")?,
                content: r.get("content")?,
                created_at: r.get("created_at")?,
            })
        })?;
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
}
