//! Desktop side of account sync: this computer's Chat and Research conversations as a replica of the
//! account's conversations in Supabase (the authoritative copy; see supabase/migrations/…_sync.sql).
//!
//! The network part lives in the interface (it holds the signed-in session); this module only does the
//! local database work, so it can be tested without a network:
//!
//! * **Outbox.** SQLite triggers record every change to a general-workspace conversation or message in
//!   `sync_outbox`, and every deletion in `sync_tombstones`. Nothing in the rest of the app has to know.
//! * **Collect / ack.** `sync_collect` turns the outbox into a push batch; `sync_ack` removes exactly what was
//!   sent (a change made while the push was in flight stays queued).
//! * **Apply.** `sync_apply` merges what the server returned: new rows are created with their original ids
//!   and timestamps, tombstones delete, the newest title wins, and a message that is waiting to be uploaded
//!   is never overwritten by an older server copy. Applied rows are removed from the outbox, so a pull never
//!   echoes back as a push.
//! * Attachments and local project files are not synced; Code projects stay on this computer.

use crate::app::Genius;
use crate::db::{now, Db};
use crate::error::Result;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const GENERAL: &str = "(SELECT id FROM projects WHERE kind = 'chat' LIMIT 1)";
const STAMP: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";
/// Longest message the server accepts (characters).
const MAX_CONTENT: usize = 399_000;

pub fn ensure_schema(db: &Db) -> Result<()> {
    let conn = db.conn();
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS sync_outbox (kind TEXT NOT NULL, id TEXT NOT NULL, ts TEXT NOT NULL, PRIMARY KEY (kind, id));
        CREATE TABLE IF NOT EXISTS sync_tombstones (kind TEXT NOT NULL, id TEXT NOT NULL, conversation_id TEXT, deleted_at TEXT NOT NULL, PRIMARY KEY (kind, id));
        CREATE TABLE IF NOT EXISTS sync_state (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        "#,
    )?;
    let has_col: bool = conn.prepare("SELECT 1 FROM pragma_table_info('conversations') WHERE name = 'title_updated_at'")?.exists([])?;
    if !has_col {
        conn.execute_batch("ALTER TABLE conversations ADD COLUMN title_updated_at TEXT;")?;
    }
    conn.execute_batch(&format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS sync_conv_ins AFTER INSERT ON conversations WHEN NEW.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_outbox(kind, id, ts) VALUES ('conv', NEW.id, {STAMP});
        END;
        CREATE TRIGGER IF NOT EXISTS sync_conv_upd AFTER UPDATE ON conversations WHEN NEW.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_outbox(kind, id, ts) VALUES ('conv', NEW.id, {STAMP});
        END;
        CREATE TRIGGER IF NOT EXISTS sync_conv_title AFTER UPDATE OF title ON conversations WHEN NEW.project_id = {GENERAL} BEGIN
            UPDATE conversations SET title_updated_at = {STAMP} WHERE id = NEW.id;
        END;
        CREATE TRIGGER IF NOT EXISTS sync_conv_del AFTER DELETE ON conversations WHEN OLD.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_tombstones(kind, id, conversation_id, deleted_at) VALUES ('conv', OLD.id, NULL, {STAMP});
            DELETE FROM sync_outbox WHERE id = OLD.id AND kind = 'conv';
        END;
        CREATE TRIGGER IF NOT EXISTS sync_msg_ins AFTER INSERT ON messages WHEN NEW.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_outbox(kind, id, ts) VALUES ('msg', NEW.id, {STAMP});
        END;
        CREATE TRIGGER IF NOT EXISTS sync_msg_upd AFTER UPDATE ON messages WHEN NEW.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_outbox(kind, id, ts) VALUES ('msg', NEW.id, {STAMP});
        END;
        CREATE TRIGGER IF NOT EXISTS sync_msg_del AFTER DELETE ON messages WHEN OLD.project_id = {GENERAL} BEGIN
            INSERT OR REPLACE INTO sync_tombstones(kind, id, conversation_id, deleted_at) VALUES ('msg', OLD.id, OLD.conversation_id, {STAMP});
            DELETE FROM sync_outbox WHERE id = OLD.id AND kind = 'msg';
        END;
        "#
    ))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncConv {
    pub id: String,
    pub mode: String,
    pub title: String,
    #[serde(default)]
    pub origin: Value,
    pub created_at: String,
    pub updated_at: String,
    pub meta_updated_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncMsg {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub meta: Value,
    #[serde(default)]
    pub task_id: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub seq: i64,
}

/// One queued change, remembered so that exactly it can be acknowledged after the upload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutKey {
    pub kind: String,
    pub id: String,
    pub ts: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Collected {
    pub conversations: Vec<SyncConv>,
    pub messages: Vec<SyncMsg>,
    pub keys: Vec<OutKey>,
    /// Changes still queued after this batch.
    pub remaining: i64,
    /// What was left out and why (unsupported role, …).
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RemoteBatch {
    #[serde(default)]
    pub conversations: Vec<SyncConv>,
    #[serde(default)]
    pub messages: Vec<SyncMsg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub conversations: usize,
    pub messages: usize,
    pub deleted: usize,
    pub skipped: usize,
}

/// Server timestamps (`2026-10-19T20:20:14.805942+00:00`) in the local format (`…14.805Z`), so ordering by text stays right.
fn local_ts(s: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)).unwrap_or_else(|_| s.to_string())
}

impl Genius {
    pub fn sync_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.db.conn().query_row("SELECT value FROM sync_state WHERE key = ?1", params![key], |r| r.get(0)).optional()?)
    }

    pub fn sync_set(&self, key: &str, value: &str) -> Result<()> {
        self.db.conn().execute("INSERT INTO sync_state(key, value) VALUES(?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value", params![key, value])?;
        Ok(())
    }

    /// Queue every existing conversation and message (first sync of this computer with an account).
    pub fn sync_enqueue_all(&self) -> Result<usize> {
        let conn = self.db.conn();
        let c = conn.execute(&format!("INSERT OR REPLACE INTO sync_outbox(kind, id, ts) SELECT 'conv', id, {STAMP} FROM conversations WHERE project_id = {GENERAL}"), [])?;
        let m = conn.execute(&format!("INSERT OR REPLACE INTO sync_outbox(kind, id, ts) SELECT 'msg', id, {STAMP} FROM messages WHERE project_id = {GENERAL}"), [])?;
        Ok(c + m)
    }

    pub fn sync_pending(&self) -> Result<i64> {
        let conn = self.db.conn();
        let a: i64 = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))?;
        let b: i64 = conn.query_row("SELECT COUNT(*) FROM sync_tombstones", [], |r| r.get(0))?;
        Ok(a + b)
    }

    /// The next push batch (at most `limit` changed rows).
    pub fn sync_collect(&self, limit: usize) -> Result<Collected> {
        let limit = limit.clamp(1, 400);
        let conn = self.db.conn();
        let mut out = Collected::default();
        let rows: Vec<(String, String, String)> = {
            let mut stmt = conn.prepare("SELECT kind, id, ts FROM sync_outbox ORDER BY ts, kind DESC, id LIMIT ?1")?;
            let it = stmt.query_map(params![limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            it.collect::<rusqlite::Result<_>>()?
        };
        let mut conv_ids: Vec<String> = Vec::new();
        let mut msg_ids: Vec<String> = Vec::new();
        for (kind, id, ts) in &rows {
            out.keys.push(OutKey { kind: kind.clone(), id: id.clone(), ts: ts.clone() });
            if kind == "conv" {
                conv_ids.push(id.clone());
            } else {
                msg_ids.push(id.clone());
            }
        }
        // messages first need their conversation on the server
        for id in &msg_ids {
            if let Some(cid) = conn.query_row("SELECT conversation_id FROM messages WHERE id = ?1", params![id], |r| r.get::<_, Option<String>>(0)).optional()?.flatten() {
                if !conv_ids.contains(&cid) {
                    conv_ids.push(cid);
                }
            }
        }
        for id in &conv_ids {
            let row = conn
                .query_row(
                    "SELECT c.id, cc.mode, c.title, cc.origin_json, c.created_at, c.updated_at, COALESCE(c.title_updated_at, c.created_at) FROM conversations c JOIN chat_conversations cc ON cc.conversation_id = c.id WHERE c.id = ?1",
                    params![id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?)),
                )
                .optional()?;
            if let Some((id, mode, title, origin, created, updated, meta)) = row {
                let origin: Value = serde_json::from_str(&origin).unwrap_or(Value::Null);
                // a Code-flavoured session never leaves the device that made it
                out.conversations.push(SyncConv { id, mode, title, origin: if origin.is_object() && origin.as_object().is_some_and(|o| o.is_empty()) { Value::Null } else { origin }, created_at: created, updated_at: updated, meta_updated_at: meta, deleted_at: None });
            }
            // a conversation whose mode row does not exist yet stays queued and is retried on the next run
        }
        for id in &msg_ids {
            let row = conn
                .query_row("SELECT id, conversation_id, role, kind, content, meta_json, task_id, created_at FROM messages WHERE id = ?1", params![id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, Option<String>>(6)?, r.get::<_, String>(7)?))
                })
                .optional()?;
            let Some((id, cid, role, kind, content, meta, task, created)) = row else { continue };
            let Some(cid) = cid else { continue };
            if !matches!(role.as_str(), "user" | "assistant" | "system") {
                out.skipped.push(format!("message {id}: role '{role}' is not synced"));
                continue;
            }
            let content: String = if content.chars().count() > MAX_CONTENT { content.chars().take(MAX_CONTENT).collect() } else { content };
            out.messages.push(SyncMsg { id, conversation_id: cid, role, kind, content, meta: serde_json::from_str(&meta).unwrap_or(json!({})), task_id: task, created_at: created.clone(), updated_at: created, deleted_at: None, seq: 0 });
        }
        // deletions
        let tombs: Vec<(String, String, Option<String>, String)> = {
            let mut stmt = conn.prepare("SELECT kind, id, conversation_id, deleted_at FROM sync_tombstones ORDER BY deleted_at LIMIT ?1")?;
            let it = stmt.query_map(params![limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
            it.collect::<rusqlite::Result<_>>()?
        };
        let gone_convs: Vec<&String> = tombs.iter().filter(|t| t.0 == "conv").map(|t| &t.1).collect();
        for (kind, id, cid, at) in &tombs {
            out.keys.push(OutKey { kind: format!("tomb-{kind}"), id: id.clone(), ts: at.clone() });
            if kind == "conv" {
                out.conversations.push(SyncConv { id: id.clone(), mode: "chat".into(), title: String::new(), origin: Value::Null, created_at: at.clone(), updated_at: at.clone(), meta_updated_at: at.clone(), deleted_at: Some(at.clone()) });
            } else if let Some(cid) = cid {
                if gone_convs.contains(&cid) {
                    continue; // the whole conversation is deleted: no need to tombstone each message
                }
                out.messages.push(SyncMsg { id: id.clone(), conversation_id: cid.clone(), role: "user".into(), kind: "chat".into(), content: String::new(), meta: json!({}), task_id: None, created_at: at.clone(), updated_at: at.clone(), deleted_at: Some(at.clone()), seq: 0 });
            }
        }
        out.remaining = {
            let a: i64 = conn.query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))?;
            let b: i64 = conn.query_row("SELECT COUNT(*) FROM sync_tombstones", [], |r| r.get(0))?;
            a + b - out.keys.len() as i64
        };
        Ok(out)
    }

    /// Forget what was uploaded. A row changed again after it was collected has a newer timestamp and stays queued.
    pub fn sync_ack(&self, keys: &[OutKey]) -> Result<()> {
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        for k in keys {
            match k.kind.as_str() {
                "conv" | "msg" => tx.execute("DELETE FROM sync_outbox WHERE kind = ?1 AND id = ?2 AND ts <= ?3", params![k.kind, k.id, k.ts])?,
                "tomb-conv" => tx.execute("DELETE FROM sync_tombstones WHERE kind = 'conv' AND id = ?1 AND deleted_at <= ?2", params![k.id, k.ts])?,
                "tomb-msg" => tx.execute("DELETE FROM sync_tombstones WHERE kind = 'msg' AND id = ?1 AND deleted_at <= ?2", params![k.id, k.ts])?,
                _ => 0,
            };
        }
        tx.commit()?;
        Ok(())
    }

    /// Merge what the server sent into the local database.
    pub fn sync_apply(&self, batch: &RemoteBatch) -> Result<ApplyReport> {
        let ws = self.general_workspace()?;
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        let mut rep = ApplyReport::default();
        let mut touched_conv: Vec<String> = Vec::new();
        let mut touched_msg: Vec<String> = Vec::new();
        let mut dead_convs: Vec<String> = Vec::new();

        for c in &batch.conversations {
            if !matches!(c.mode.as_str(), "chat" | "research") {
                rep.skipped += 1;
                continue;
            }
            if c.deleted_at.is_some() {
                let n = tx.execute("DELETE FROM conversations WHERE id = ?1 AND project_id = ?2", params![c.id, ws.id])?;
                tx.execute("DELETE FROM messages WHERE conversation_id = ?1 AND project_id = ?2", params![c.id, ws.id])?;
                dead_convs.push(c.id.clone());
                rep.deleted += n;
                continue;
            }
            let local: Option<(String, String)> = tx.query_row("SELECT title, COALESCE(title_updated_at, created_at) FROM conversations WHERE id = ?1 AND project_id = ?2", params![c.id, ws.id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let created = local_ts(&c.created_at);
            let updated = local_ts(&c.updated_at);
            let meta = local_ts(&c.meta_updated_at);
            match local {
                None => {
                    let exists_elsewhere: bool = tx.query_row("SELECT COUNT(*) FROM conversations WHERE id = ?1", params![c.id], |r| r.get::<_, i64>(0))? > 0;
                    if exists_elsewhere {
                        rep.skipped += 1;
                        continue;
                    }
                    tx.execute("INSERT INTO conversations(id, project_id, title, created_at, updated_at) VALUES(?1, ?2, ?3, ?4, ?5)", params![c.id, ws.id, c.title, created, updated])?;
                    tx.execute("INSERT OR REPLACE INTO chat_conversations(conversation_id, mode, origin_json) VALUES(?1, ?2, ?3)", params![c.id, c.mode, if c.origin.is_null() { "{}".to_string() } else { c.origin.to_string() }])?;
                    tx.execute("UPDATE conversations SET title_updated_at = ?2 WHERE id = ?1", params![c.id, meta])?;
                    rep.conversations += 1;
                }
                Some((title, title_at)) => {
                    if meta > title_at && title != c.title {
                        tx.execute("UPDATE conversations SET title = ?2 WHERE id = ?1", params![c.id, c.title])?;
                        // the title trigger stamped "now"; the title's real age is the server's
                        tx.execute("UPDATE conversations SET title_updated_at = ?2 WHERE id = ?1", params![c.id, meta])?;
                    }
                    tx.execute("UPDATE conversations SET updated_at = ?2 WHERE id = ?1 AND updated_at < ?2", params![c.id, updated])?;
                    rep.conversations += 1;
                }
            }
            touched_conv.push(c.id.clone());
        }

        // oldest first, then by the server's sequence, so rows with the same millisecond keep their order
        let mut msgs: Vec<&SyncMsg> = batch.messages.iter().collect();
        msgs.sort_by(|a, b| local_ts(&a.created_at).cmp(&local_ts(&b.created_at)).then(a.seq.cmp(&b.seq)));
        for m in msgs {
            if dead_convs.contains(&m.conversation_id) {
                continue;
            }
            let known: bool = tx.query_row("SELECT COUNT(*) FROM conversations WHERE id = ?1 AND project_id = ?2", params![m.conversation_id, ws.id], |r| r.get::<_, i64>(0))? > 0;
            if !known {
                rep.skipped += 1;
                continue;
            }
            if m.deleted_at.is_some() {
                let n = tx.execute("DELETE FROM messages WHERE id = ?1 AND project_id = ?2", params![m.id, ws.id])?;
                rep.deleted += n;
                touched_msg.push(m.id.clone());
                continue;
            }
            if !matches!(m.role.as_str(), "user" | "assistant" | "system") {
                rep.skipped += 1;
                continue;
            }
            let existing: Option<(String, String)> = tx.query_row("SELECT content, created_at FROM messages WHERE id = ?1", params![m.id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let created = local_ts(&m.created_at);
            match existing {
                None => {
                    tx.execute(
                        "INSERT INTO messages(id, project_id, task_id, role, content, created_at, conversation_id, kind, meta_json) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        params![m.id, ws.id, m.task_id, m.role, m.content, created, m.conversation_id, if m.kind.is_empty() { "chat" } else { &m.kind }, if m.meta.is_null() { "{}".to_string() } else { m.meta.to_string() }],
                    )?;
                    rep.messages += 1;
                }
                Some((content, _)) => {
                    let queued: bool = tx.query_row("SELECT COUNT(*) FROM sync_outbox WHERE kind = 'msg' AND id = ?1", params![m.id], |r| r.get::<_, i64>(0))? > 0;
                    if !queued && content != m.content && local_ts(&m.updated_at) > created {
                        tx.execute("UPDATE messages SET content = ?2, meta_json = ?3 WHERE id = ?1", params![m.id, m.content, if m.meta.is_null() { "{}".to_string() } else { m.meta.to_string() }])?;
                        rep.messages += 1;
                    }
                }
            }
            touched_msg.push(m.id.clone());
        }
        // what came from the server must not be pushed back
        for id in &touched_conv {
            tx.execute("DELETE FROM sync_outbox WHERE kind = 'conv' AND id = ?1", params![id])?;
        }
        for id in &touched_msg {
            tx.execute("DELETE FROM sync_outbox WHERE kind = 'msg' AND id = ?1", params![id])?;
            tx.execute("DELETE FROM sync_tombstones WHERE kind = 'msg' AND id = ?1", params![id])?;
        }
        for id in &dead_convs {
            tx.execute("DELETE FROM sync_outbox WHERE id = ?1", params![id])?;
            tx.execute("DELETE FROM sync_tombstones WHERE id = ?1 OR conversation_id = ?1", params![id])?;
        }
        tx.commit()?;
        let _ = now();
        Ok(rep)
    }
}
