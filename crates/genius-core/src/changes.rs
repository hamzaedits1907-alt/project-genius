//! Changesets: every modification of project files (AI generation, AI edits, Build Doctor
//! repairs, manual saves, restores) is recorded with the exact before/after content.
//!
//! Lifecycle: `proposed` → `applied` | `rejected`; an applied changeset can later be
//! `reverted`. Applying verifies that each file still has the content the proposal was
//! computed against, so a proposal never silently overwrites work done in the meantime.
//! This is Genius's internal recovery mechanism; it works with or without Git.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use similar::{ChangeTag, TextDiff};
use std::collections::BTreeMap;

use crate::db::{now, Db};
use crate::error::{GeniusError, Result};
use crate::workspace::{normalize_rel, to_slash, Workspace};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// "create" | "modify" | "delete"
    pub op: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChangeStat {
    pub path: String,
    pub op: String,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changeset {
    pub id: String,
    pub project_id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub status: String,
    pub created_at: String,
    pub applied_at: Option<String>,
    pub files: Vec<FileChangeStat>,
}

pub fn line_stats(before: Option<&str>, after: Option<&str>) -> (usize, usize) {
    let diff = TextDiff::from_lines(before.unwrap_or(""), after.unwrap_or(""));
    let mut a = 0;
    let mut r = 0;
    for c in diff.iter_all_changes() {
        match c.tag() {
            ChangeTag::Insert => a += 1,
            ChangeTag::Delete => r += 1,
            ChangeTag::Equal => {}
        }
    }
    (a, r)
}

pub fn unified_diff(path: &str, before: Option<&str>, after: Option<&str>) -> String {
    TextDiff::from_lines(before.unwrap_or(""), after.unwrap_or(""))
        .unified_diff()
        .context_radius(3)
        .header(
            &if before.is_some() { format!("a/{path}") } else { "/dev/null".into() },
            &if after.is_some() { format!("b/{path}") } else { "/dev/null".into() },
        )
        .to_string()
}

/// Compute file changes against the current workspace state. No-op writes are dropped;
/// every path is validated as writable.
pub fn compute_changes(ws: &Workspace, writes: &[(String, String)], deletes: &[String]) -> Result<Vec<FileChange>> {
    let mut out: BTreeMap<String, FileChange> = BTreeMap::new();
    for (path, content) in writes {
        let norm = to_slash(&normalize_rel(path)?);
        ws.check_writable(&norm)?;
        let before = ws.read_text_opt(&norm)?;
        if before.as_deref() == Some(content.as_str()) {
            continue;
        }
        let op = if before.is_some() { "modify" } else { "create" };
        out.insert(norm.clone(), FileChange { path: norm, op: op.into(), before, after: Some(content.clone()) });
    }
    for path in deletes {
        let norm = to_slash(&normalize_rel(path)?);
        ws.check_writable(&norm)?;
        if let Some(before) = ws.read_text_opt(&norm)? {
            out.insert(norm.clone(), FileChange { path: norm, op: "delete".into(), before: Some(before), after: None });
        }
    }
    Ok(out.into_values().collect())
}

pub struct ChangeStore<'a> {
    pub db: &'a Db,
}

impl<'a> ChangeStore<'a> {
    pub fn new(db: &'a Db) -> Self {
        Self { db }
    }

    pub fn create(
        &self,
        project_id: &str,
        task_id: Option<&str>,
        kind: &str,
        title: &str,
        summary: &str,
        files: &[FileChange],
    ) -> Result<Changeset> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.db.conn();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO changesets(id,project_id,task_id,kind,title,summary,status,created_at) VALUES(?1,?2,?3,?4,?5,?6,'proposed',?7)",
            params![id, project_id, task_id, kind, title, summary, now()],
        )?;
        for f in files {
            tx.execute(
                "INSERT INTO changeset_files(changeset_id,path,op,before_text,after_text) VALUES(?1,?2,?3,?4,?5)",
                params![id, f.path, f.op, f.before, f.after],
            )?;
        }
        tx.commit()?;
        drop(conn);
        self.get(&id)
    }

    pub fn get(&self, id: &str) -> Result<Changeset> {
        let conn = self.db.conn();
        let mut cs = conn
            .query_row("SELECT * FROM changesets WHERE id = ?1", [id], |r| {
                Ok(Changeset {
                    id: r.get("id")?,
                    project_id: r.get("project_id")?,
                    task_id: r.get("task_id")?,
                    kind: r.get("kind")?,
                    title: r.get("title")?,
                    summary: r.get("summary")?,
                    status: r.get("status")?,
                    created_at: r.get("created_at")?,
                    applied_at: r.get("applied_at")?,
                    files: vec![],
                })
            })
            .optional()?
            .ok_or_else(|| GeniusError::NotFound(format!("changeset {id}")))?;
        drop(conn);
        cs.files = self
            .files(id)?
            .into_iter()
            .map(|f| {
                let (added, removed) = line_stats(f.before.as_deref(), f.after.as_deref());
                FileChangeStat { path: f.path, op: f.op, added, removed }
            })
            .collect();
        Ok(cs)
    }

    pub fn files(&self, id: &str) -> Result<Vec<FileChange>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare("SELECT path, op, before_text, after_text FROM changeset_files WHERE changeset_id = ?1 ORDER BY path")?;
        let rows = stmt.query_map([id], |r| {
            Ok(FileChange { path: r.get(0)?, op: r.get(1)?, before: r.get(2)?, after: r.get(3)? })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list(&self, project_id: &str, limit: usize) -> Result<Vec<Changeset>> {
        let ids: Vec<String> = {
            let conn = self.db.conn();
            let mut stmt = conn.prepare("SELECT id FROM changesets WHERE project_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2")?;
            let rows = stmt.query_map(params![project_id, limit as i64], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        ids.iter().map(|id| self.get(id)).collect()
    }

    fn set_status(&self, id: &str, status: &str) -> Result<()> {
        let applied = if status == "applied" { Some(now()) } else { None };
        self.db.conn().execute(
            "UPDATE changesets SET status = ?2, applied_at = COALESCE(?3, applied_at) WHERE id = ?1",
            params![id, status, applied],
        )?;
        Ok(())
    }

    /// Apply a proposed changeset. All files are checked first; writes are rolled back if
    /// any write fails, so the workspace is never left half-applied.
    pub fn apply(&self, ws: &Workspace, id: &str) -> Result<Changeset> {
        let cs = self.get(id)?;
        if cs.status != "proposed" {
            return Err(GeniusError::Conflict(format!("changeset is already {}", cs.status)));
        }
        let files = self.files(id)?;
        apply_files(ws, &files)?;
        self.set_status(id, "applied")?;
        self.get(id)
    }

    pub fn reject(&self, id: &str) -> Result<Changeset> {
        let cs = self.get(id)?;
        if cs.status != "proposed" {
            return Err(GeniusError::Conflict(format!("changeset is already {}", cs.status)));
        }
        self.set_status(id, "rejected")?;
        self.get(id)
    }

    /// Record and immediately apply a change made directly by the user (editor save).
    pub fn record_manual(&self, ws: &Workspace, project_id: &str, path: &str, content: &str) -> Result<Option<Changeset>> {
        let changes = compute_changes(ws, &[(path.to_string(), content.to_string())], &[])?;
        if changes.is_empty() {
            return Ok(None);
        }
        let cs = self.create(project_id, None, "manual", &format!("Edited {path}"), "Saved from the editor", &changes)?;
        Ok(Some(self.apply(ws, &cs.id)?))
    }

    /// Propose a changeset that undoes an applied changeset. Fails with a conflict if a file
    /// has been changed since (unless `force`, in which case those later edits are included
    /// in the reviewable diff and overwritten only when the user applies it).
    pub fn propose_revert(&self, ws: &Workspace, id: &str, force: bool) -> Result<Changeset> {
        let cs = self.get(id)?;
        if cs.status != "applied" {
            return Err(GeniusError::Conflict(format!("only applied changesets can be reverted (this one is {})", cs.status)));
        }
        let mut changes = Vec::new();
        let mut conflicts = Vec::new();
        for f in self.files(id)? {
            let current = ws.read_text_opt(&f.path)?;
            if current != f.after {
                conflicts.push(f.path.clone());
            }
            if current == f.before {
                continue;
            }
            changes.push(FileChange {
                op: match (&current, &f.before) {
                    (None, Some(_)) => "create",
                    (Some(_), None) => "delete",
                    _ => "modify",
                }
                .into(),
                path: f.path,
                before: current,
                after: f.before,
            });
        }
        if !conflicts.is_empty() && !force {
            return Err(GeniusError::Conflict(format!(
                "these files changed after '{}' was applied: {}. Review and confirm to revert anyway.",
                cs.title,
                conflicts.join(", ")
            )));
        }
        if changes.is_empty() {
            return Err(GeniusError::Validation("nothing to revert: files already match the earlier state".into()));
        }
        self.create(&cs.project_id, None, "restore", &format!("Revert: {}", cs.title), &format!("reverts:{id}"), &changes)
    }

    /// Propose a changeset that returns every file touched after `id` to its state right after
    /// `id` was applied ("restore to this point").
    pub fn propose_restore_to(&self, ws: &Workspace, project_id: &str, id: &str) -> Result<Changeset> {
        let target = self.get(id)?;
        let later: Vec<String> = {
            let conn = self.db.conn();
            let mut stmt = conn.prepare(
                "SELECT id FROM changesets WHERE project_id = ?1 AND status IN ('applied','reverted')
                   AND (created_at > ?2 OR (created_at = ?2 AND rowid > (SELECT rowid FROM changesets WHERE id = ?3)))
                 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = stmt.query_map(params![project_id, target.created_at, id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        // earliest `before` per path among later changesets = state at the restore point
        let mut target_state: BTreeMap<String, Option<String>> = BTreeMap::new();
        for cid in &later {
            for f in self.files(cid)? {
                target_state.entry(f.path.clone()).or_insert(f.before);
            }
        }
        let mut changes = Vec::new();
        for (path, wanted) in target_state {
            let current = ws.read_text_opt(&path)?;
            if current == wanted {
                continue;
            }
            let op = match (&current, &wanted) {
                (None, _) => "create",
                (_, None) => "delete",
                _ => "modify",
            };
            changes.push(FileChange { path, op: op.into(), before: current, after: wanted });
        }
        if changes.is_empty() {
            return Err(GeniusError::Validation("the project already matches that point in history".into()));
        }
        self.create(project_id, None, "restore", &format!("Restore to: {}", target.title), &format!("restores-to:{id}"), &changes)
    }

    /// After applying a revert proposal, mark the original as reverted.
    pub fn mark_reverted_if_restore(&self, applied: &Changeset) -> Result<()> {
        if let Some(orig) = applied.summary.strip_prefix("reverts:") {
            self.set_status(orig, "reverted")?;
        }
        Ok(())
    }
}

pub fn apply_files(ws: &Workspace, files: &[FileChange]) -> Result<()> {
    // Pre-flight: every file must be exactly in the state the change was computed from.
    let mut stale = Vec::new();
    for f in files {
        ws.check_writable(&f.path)?;
        let current = ws.read_text_opt(&f.path)?;
        if current != f.before {
            stale.push(f.path.clone());
        }
    }
    if !stale.is_empty() {
        return Err(GeniusError::Conflict(format!(
            "these files changed since the proposal was created: {}. Regenerate or review the proposal again.",
            stale.join(", ")
        )));
    }
    let mut done: Vec<&FileChange> = Vec::new();
    for f in files {
        let res = match &f.after {
            Some(text) => ws.write_text(&f.path, text),
            None => ws.delete_file(&f.path),
        };
        if let Err(e) = res {
            for d in done.iter().rev() {
                let _ = match &d.before {
                    Some(t) => ws.write_text(&d.path, t),
                    None => ws.delete_file(&d.path),
                };
            }
            return Err(e);
        }
        done.push(f);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ProjectRow;

    fn setup() -> (tempfile::TempDir, Workspace, Db) {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::new(d.path().join("p")).unwrap();
        let db = Db::open_in_memory().unwrap();
        db.insert_project(&ProjectRow {
            id: "p".into(),
            name: "p".into(),
            description: String::new(),
            path: ws.root().to_string_lossy().into(),
            kind: "x".into(),
            spec: None,
            notes: String::new(),
            status: serde_json::json!({}),
            created_at: now(),
            updated_at: now(),
            last_opened_at: None,
        })
        .unwrap();
        (d, ws, db)
    }

    #[test]
    fn diff_shows_inserted_and_removed_lines() {
        let d = unified_diff("a.ts", Some("one\ntwo\n"), Some("one\nthree\n"));
        assert!(d.contains("--- a/a.ts") && d.contains("+++ b/a.ts"));
        assert!(d.contains("-two") && d.contains("+three"));
        assert_eq!(line_stats(Some("one\ntwo\n"), Some("one\nthree\nfour\n")), (2, 1));
    }

    #[test]
    fn propose_apply_and_revert() {
        let (_d, ws, db) = setup();
        ws.write_text("src/a.ts", "v1\n").unwrap();
        let store = ChangeStore::new(&db);
        let changes = compute_changes(&ws, &[("src/a.ts".into(), "v2\n".into()), ("src/b.ts".into(), "new\n".into())], &[]).unwrap();
        assert_eq!(changes.len(), 2);
        let cs = store.create("p", None, "change", "Edit", "", &changes).unwrap();
        assert_eq!(cs.status, "proposed");
        assert_eq!(ws.read_text("src/a.ts").unwrap(), "v1\n", "proposal does not touch disk");
        store.apply(&ws, &cs.id).unwrap();
        assert_eq!(ws.read_text("src/a.ts").unwrap(), "v2\n");
        assert!(store.apply(&ws, &cs.id).is_err(), "cannot apply twice");

        let rev = store.propose_revert(&ws, &cs.id, false).unwrap();
        let applied = store.apply(&ws, &rev.id).unwrap();
        store.mark_reverted_if_restore(&applied).unwrap();
        assert_eq!(ws.read_text("src/a.ts").unwrap(), "v1\n");
        assert!(!ws.exists("src/b.ts"));
        assert_eq!(store.get(&cs.id).unwrap().status, "reverted");
    }

    #[test]
    fn stale_proposal_is_not_applied() {
        let (_d, ws, db) = setup();
        ws.write_text("a.ts", "v1\n").unwrap();
        let store = ChangeStore::new(&db);
        let cs = store.create("p", None, "change", "E", "", &compute_changes(&ws, &[("a.ts".into(), "ai\n".into())], &[]).unwrap()).unwrap();
        ws.write_text("a.ts", "user edit\n").unwrap();
        let err = store.apply(&ws, &cs.id).unwrap_err();
        assert!(matches!(err, GeniusError::Conflict(_)));
        assert_eq!(ws.read_text("a.ts").unwrap(), "user edit\n", "user work preserved");
    }

    #[test]
    fn revert_detects_later_edits() {
        let (_d, ws, db) = setup();
        let store = ChangeStore::new(&db);
        let cs = store.create("p", None, "gen", "G", "", &compute_changes(&ws, &[("a.ts".into(), "gen\n".into())], &[]).unwrap()).unwrap();
        store.apply(&ws, &cs.id).unwrap();
        ws.write_text("a.ts", "later\n").unwrap();
        assert!(matches!(store.propose_revert(&ws, &cs.id, false), Err(GeniusError::Conflict(_))));
        let forced = store.propose_revert(&ws, &cs.id, true).unwrap();
        assert_eq!(store.files(&forced.id).unwrap()[0].before.as_deref(), Some("later\n"));
    }

    #[test]
    fn restore_to_point() {
        let (_d, ws, db) = setup();
        let store = ChangeStore::new(&db);
        let c1 = store.create("p", None, "gen", "one", "", &compute_changes(&ws, &[("a.ts".into(), "1\n".into())], &[]).unwrap()).unwrap();
        store.apply(&ws, &c1.id).unwrap();
        let c2 = store.create("p", None, "change", "two", "", &compute_changes(&ws, &[("a.ts".into(), "2\n".into()), ("b.ts".into(), "b\n".into())], &[]).unwrap()).unwrap();
        store.apply(&ws, &c2.id).unwrap();
        store.record_manual(&ws, "p", "a.ts", "3\n").unwrap();
        let restore = store.propose_restore_to(&ws, "p", &c1.id).unwrap();
        store.apply(&ws, &restore.id).unwrap();
        assert_eq!(ws.read_text("a.ts").unwrap(), "1\n");
        assert!(!ws.exists("b.ts"));
    }

    #[test]
    fn no_op_and_unsafe_changes() {
        let (_d, ws, _db) = setup();
        ws.write_text("a.ts", "same").unwrap();
        assert!(compute_changes(&ws, &[("a.ts".into(), "same".into())], &[]).unwrap().is_empty());
        assert!(compute_changes(&ws, &[("../x".into(), "x".into())], &[]).is_err());
        assert!(compute_changes(&ws, &[(".env".into(), "K=1".into())], &[]).is_err());
    }
}
