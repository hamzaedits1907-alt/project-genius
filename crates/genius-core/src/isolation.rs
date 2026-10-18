//! Isolated copies of a project for mission tasks.
//!
//! A task that may edit files never runs in the live project directory. It gets a copy of the
//! project tree (without `node_modules`, `.git`, `dist`, … and without secret files; `node_modules`
//! is shared read-only through a symlink so type checks work without a reinstall). The copy
//! remembers the content hash of every file at copy time (the task's *base snapshot*), so when the
//! task is done its changes can be computed exactly and checked against the live project: a file
//! that changed in the live project since the snapshot is a conflict and is never overwritten.
//!
//! Why not `git worktree`: a worktree checks out HEAD, so uncommitted work of the user and
//! untracked files would be missing from the task's base; it also writes administrative files into
//! the user's `.git`. A plain copy is identical for git and non-git projects and cannot touch the
//! repository.

use crate::changes::FileChange;
use crate::error::{GeniusError, Result};
use crate::workspace::{is_secret_name, Workspace, IGNORED_DIRS};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// path (slash separated, project relative) → sha256 of the file bytes
pub type Manifest = BTreeMap<String, String>;

const MAX_FILES: usize = 20_000;
const MAX_TOTAL_BYTES: u64 = 400 * 1024 * 1024;

pub fn hash_bytes(b: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b);
    h.finalize().iter().map(|x| format!("{x:02x}")).collect()
}

fn skip_dir(name: &str, keep_genius: bool) -> bool {
    IGNORED_DIRS.contains(&name) && !(keep_genius && name == ".genius")
}

/// Regular, non-secret files of a project tree (symlinks are never followed or copied).
pub fn walk_files(root: &Path, keep_genius: bool) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root).follow_links(false).sort_by_file_name().into_iter().filter_entry(|e| !(e.file_type().is_dir() && e.depth() > 0 && skip_dir(&e.file_name().to_string_lossy(), keep_genius)));
    for entry in walker {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue };
        let rel = crate::workspace::to_slash(rel);
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        if is_secret_name(name) || rel.contains(".genius-tmp-") {
            continue;
        }
        out.push((rel, entry.path().to_path_buf()));
        if out.len() > MAX_FILES {
            return Err(GeniusError::Validation(format!("The project has more than {MAX_FILES} files; it is too large to give an agent an isolated copy.")));
        }
    }
    Ok(out)
}

/// Content hashes of the project's files (as `copy_project` would copy them, `.genius` excluded).
pub fn compute_manifest(root: &Path) -> Result<Manifest> {
    let mut m = Manifest::new();
    for (rel, abs) in walk_files(root, false)? {
        if let Ok(b) = std::fs::read(&abs) {
            m.insert(rel, hash_bytes(&b));
        }
    }
    Ok(m)
}

/// Copy the project into `dst` and return its base manifest. With `link_modules`, the live
/// `node_modules` (if any) is linked into the copy.
pub fn copy_project(src: &Path, dst: &Path, link_modules: bool) -> Result<Manifest> {
    std::fs::create_dir_all(dst)?;
    let mut manifest = Manifest::new();
    let mut total = 0u64;
    for (rel, abs) in walk_files(src, true)? {
        let bytes = std::fs::read(&abs)?;
        total += bytes.len() as u64;
        if total > MAX_TOTAL_BYTES {
            return Err(GeniusError::Validation("The project is too large (over 400 MB of source files) to give an agent an isolated copy.".into()));
        }
        let target = dst.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &bytes)?;
        if let Ok(meta) = std::fs::metadata(&abs) {
            let _ = std::fs::set_permissions(&target, meta.permissions());
            // keep modification times: "package.json newer than node_modules" must not trigger a reinstall
            if let Ok(t) = meta.modified() {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = f.set_modified(t);
                }
            }
        }
        if !rel.starts_with(".genius/") {
            manifest.insert(rel, hash_bytes(&bytes));
        }
    }
    if link_modules {
        let nm = src.join("node_modules");
        if nm.is_dir() {
            link_dir(&nm, &dst.join("node_modules"));
        }
    }
    Ok(manifest)
}

#[cfg(unix)]
fn link_dir(target: &Path, link: &Path) {
    let _ = std::os::unix::fs::symlink(target, link);
}

#[cfg(windows)]
fn link_dir(target: &Path, link: &Path) {
    // may need privileges; without the link the copy simply reinstalls when it must build
    let _ = std::os::windows::fs::symlink_dir(target, link);
}

#[cfg(not(any(unix, windows)))]
fn link_dir(_target: &Path, _link: &Path) {}

/// What the task changed relative to its base snapshot, as changeset entries against the live
/// project (`before` = the live content). Files that cannot be represented as text (binary, too
/// large) are returned in the second list, not changed.
pub fn collect_changes(shadow: &Path, base: &Manifest, live: &Path) -> Result<(Vec<FileChange>, Vec<String>)> {
    let live_ws = Workspace::new(live)?;
    let shadow_ws = Workspace::new(shadow)?;
    let mut changes = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (rel, abs) in walk_files(shadow, false)? {
        seen.insert(rel.clone());
        let Ok(bytes) = std::fs::read(&abs) else { continue };
        if base.get(&rel) == Some(&hash_bytes(&bytes)) {
            continue;
        }
        let Ok(after) = shadow_ws.read_text(&rel) else {
            skipped.push(rel);
            continue;
        };
        let before = live_ws.read_text_opt(&rel).unwrap_or(None);
        if before.as_deref() == Some(after.as_str()) {
            continue;
        }
        let op = if base.contains_key(&rel) || before.is_some() { "modify" } else { "create" };
        changes.push(FileChange { path: rel, op: op.into(), before, after: Some(after) });
    }
    for rel in base.keys() {
        if seen.contains(rel) {
            continue;
        }
        if let Ok(Some(before)) = live_ws.read_text_opt(rel) {
            changes.push(FileChange { path: rel.clone(), op: "delete".into(), before: Some(before), after: None });
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((changes, skipped))
}

/// Paths whose live content differs from the task's base snapshot (or that appeared in the live
/// project since): applying the task's change to them would overwrite someone else's work.
pub fn detect_conflicts(live: &Path, base: &Manifest, changes: &[FileChange]) -> Vec<String> {
    let mut out = Vec::new();
    for c in changes {
        let now_hash = std::fs::read(live.join(&c.path)).ok().map(|b| hash_bytes(&b));
        if now_hash.as_ref() != base.get(&c.path) {
            out.push(c.path.clone());
        }
    }
    out
}

/// Write the changes into a tree (used to build the "integrated result" copy for verification).
pub fn overlay(root: &Path, changes: &[FileChange]) -> Result<()> {
    let ws = Workspace::new(root)?;
    for c in changes {
        match &c.after {
            Some(content) => ws.write_text(&c.path, content)?,
            None => ws.delete_file(&c.path)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("src")).unwrap();
        std::fs::create_dir_all(r.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(r.join(".git")).unwrap();
        std::fs::write(r.join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::fs::write(r.join("src/b.ts"), "export const b = 1;\n").unwrap();
        std::fs::write(r.join(".env"), "TOKEN=SECRET").unwrap();
        std::fs::write(r.join("node_modules/x/i.js"), "x").unwrap();
        std::fs::write(r.join(".git/HEAD"), "ref").unwrap();
        d
    }

    #[test]
    fn copy_excludes_secrets_and_vcs_and_links_modules() {
        let live = project();
        let dst = tempfile::tempdir().unwrap();
        let base = copy_project(live.path(), &dst.path().join("ws"), true).unwrap();
        assert_eq!(base.keys().cloned().collect::<Vec<_>>(), vec!["src/a.ts".to_string(), "src/b.ts".to_string()]);
        assert!(!dst.path().join("ws/.env").exists() && !dst.path().join("ws/.git").exists());
        #[cfg(unix)]
        assert!(dst.path().join("ws/node_modules").symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(compute_manifest(live.path()).unwrap(), base);
    }

    #[test]
    fn changes_are_computed_against_the_base_and_conflicts_are_detected() {
        let live = project();
        let dst = tempfile::tempdir().unwrap();
        let ws_root = dst.path().join("ws");
        let base = copy_project(live.path(), &ws_root, false).unwrap();
        std::fs::write(ws_root.join("src/a.ts"), "export const a = 2;\n").unwrap();
        std::fs::write(ws_root.join("src/new.ts"), "export {};\n").unwrap();
        std::fs::remove_file(ws_root.join("src/b.ts")).unwrap();
        let (changes, skipped) = collect_changes(&ws_root, &base, live.path()).unwrap();
        assert!(skipped.is_empty());
        let ops: Vec<(&str, &str)> = changes.iter().map(|c| (c.path.as_str(), c.op.as_str())).collect();
        assert_eq!(ops, vec![("src/a.ts", "modify"), ("src/b.ts", "delete"), ("src/new.ts", "create")]);
        assert_eq!(changes[0].before.as_deref(), Some("export const a = 1;\n"));
        assert!(detect_conflicts(live.path(), &base, &changes).is_empty());
        // the live project moves on: a.ts changes, and someone creates new.ts
        std::fs::write(live.path().join("src/a.ts"), "export const a = 'live';\n").unwrap();
        std::fs::write(live.path().join("src/new.ts"), "export const n = 1;\n").unwrap();
        assert_eq!(detect_conflicts(live.path(), &base, &changes), vec!["src/a.ts".to_string(), "src/new.ts".to_string()]);
    }

    #[test]
    fn overlay_builds_the_integrated_tree() {
        let live = project();
        let dst = tempfile::tempdir().unwrap();
        let copy = dst.path().join("int");
        copy_project(live.path(), &copy, false).unwrap();
        let ch = vec![
            FileChange { path: "src/a.ts".into(), op: "modify".into(), before: None, after: Some("A2".into()) },
            FileChange { path: "src/b.ts".into(), op: "delete".into(), before: None, after: None },
        ];
        overlay(&copy, &ch).unwrap();
        assert_eq!(std::fs::read_to_string(copy.join("src/a.ts")).unwrap(), "A2");
        assert!(!copy.join("src/b.ts").exists());
        assert_eq!(std::fs::read_to_string(live.path().join("src/a.ts")).unwrap(), "export const a = 1;\n", "the live tree is untouched");
    }
}
