//! Sandboxed access to a project's directory.
//!
//! All paths coming from the UI or the model are *relative* project paths and are
//! validated here: no absolute paths, no `..`, no drive prefixes, no NUL bytes, no
//! writes into dependency/VCS folders, and no symlink that resolves outside the
//! workspace root. Size limits protect the UI and the model context.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

use crate::error::{GeniusError, Result};

pub const MAX_TEXT_FILE: u64 = 512 * 1024;
pub const MAX_WRITE: usize = 512 * 1024;

/// Directories never walked, shown, sent to the model, or written by the AI.
pub const IGNORED_DIRS: &[&str] = &["node_modules", ".git", "dist", "build", ".vite", ".next", "target", ".turbo", "coverage", ".cache", ".genius"];

#[derive(Debug, Clone)]
pub struct Workspace {
    /// Root as handed to child processes and shown to the user. On Windows this is a plain
    /// `C:\\…` path, never a `\\?\\` verbatim path: `cmd.exe` (which runs `npm.cmd`) refuses a
    /// verbatim current directory and silently falls back to `C:\\Windows`.
    root: PathBuf,
    /// Fully canonical root used only for the symlink-escape comparison.
    canonical: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub is_text: bool,
    /// Excluded from AI context by default (secrets, lockfiles, binaries, too large).
    pub sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

impl Workspace {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let canonical = root.canonicalize()?;
        let root = display_path(&canonical);
        Ok(Self { root, canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Validate a relative path and return its absolute location inside the workspace.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        let rel_norm = normalize_rel(rel)?;
        let abs = self.root.join(&rel_norm);
        // Walk existing ancestors and make sure none of them escapes via symlink.
        let mut probe = abs.clone();
        loop {
            if probe.exists() || probe.symlink_metadata().is_ok() {
                let real = probe.canonicalize().map_err(|_| GeniusError::PathRejected(format!("'{rel}' points through a broken or unreadable link")))?;
                if !real.starts_with(&self.canonical) {
                    return Err(GeniusError::PathRejected(format!("'{rel}' resolves outside the project workspace")));
                }
                break;
            }
            if !probe.pop() || probe == self.root {
                break;
            }
        }
        Ok(abs)
    }

    /// Validate a path the AI wants to write.
    pub fn check_writable(&self, rel: &str) -> Result<PathBuf> {
        let norm = normalize_rel(rel)?;
        let first = norm.components().next().and_then(|c| c.as_os_str().to_str()).unwrap_or("");
        if IGNORED_DIRS.contains(&first) {
            return Err(GeniusError::PathRejected(format!("writing into '{first}/' is not allowed")));
        }
        let name = norm.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if is_secret_name(name) {
            return Err(GeniusError::PathRejected(format!("'{rel}' looks like a secrets file; Genius will not write it automatically")));
        }
        self.resolve(rel)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.resolve(rel).map(|p| p.is_file()).unwrap_or(false)
    }

    pub fn read_text(&self, rel: &str) -> Result<String> {
        let abs = self.resolve(rel)?;
        let meta = std::fs::metadata(&abs).map_err(|_| GeniusError::NotFound(rel.to_string()))?;
        if !meta.is_file() {
            return Err(GeniusError::NotFound(format!("{rel} is not a file")));
        }
        if meta.len() > MAX_TEXT_FILE {
            return Err(GeniusError::Validation(format!("{rel} is too large to open as text ({} KB)", meta.len() / 1024)));
        }
        let bytes = std::fs::read(&abs)?;
        if looks_binary(&bytes) {
            return Err(GeniusError::Validation(format!("{rel} is a binary file")));
        }
        String::from_utf8(bytes).map_err(|_| GeniusError::Validation(format!("{rel} is not valid UTF-8 text")))
    }

    pub fn read_text_opt(&self, rel: &str) -> Result<Option<String>> {
        if !self.exists(rel) {
            return Ok(None);
        }
        self.read_text(rel).map(Some)
    }

    /// Atomic write: temp file in the same directory, then rename.
    pub fn write_text(&self, rel: &str, content: &str) -> Result<()> {
        if content.len() > MAX_WRITE {
            return Err(GeniusError::Validation(format!("refusing to write {rel}: content exceeds {} KB", MAX_WRITE / 1024)));
        }
        let abs = self.resolve(rel)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
            // re-validate now that intermediate directories exist
            self.resolve(rel)?;
        }
        let tmp = abs.with_extension(format!("genius-tmp-{}", std::process::id()));
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &abs).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        Ok(())
    }

    pub fn delete_file(&self, rel: &str) -> Result<()> {
        let abs = self.resolve(rel)?;
        if abs.is_file() {
            std::fs::remove_file(&abs)?;
            // tidy now-empty parent dirs inside the workspace
            let mut dir = abs.parent().map(Path::to_path_buf);
            while let Some(d) = dir {
                if d == self.root || std::fs::remove_dir(&d).is_err() {
                    break;
                }
                dir = d.parent().map(Path::to_path_buf);
            }
        }
        Ok(())
    }

    pub fn list_files(&self) -> Result<Vec<FileEntry>> {
        let mut out = Vec::new();
        let walker = walkdir::WalkDir::new(&self.root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|e| !(e.file_type().is_dir() && e.depth() > 0 && IGNORED_DIRS.contains(&e.file_name().to_string_lossy().as_ref())));
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&self.root) else { continue };
            let rel = to_slash(rel);
            if rel.contains(".genius-tmp-") {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let is_text = size <= MAX_TEXT_FILE && is_text_path(&rel, entry.path());
            out.push(FileEntry { sensitive: is_sensitive(&rel, size, is_text), path: rel, size, is_text });
            if out.len() >= 5000 {
                break;
            }
        }
        Ok(out)
    }

    pub fn search(&self, query: &str, max_hits: usize) -> Result<Vec<SearchHit>> {
        let q = query.to_lowercase();
        if q.trim().is_empty() {
            return Ok(vec![]);
        }
        let mut hits = Vec::new();
        for f in self.list_files()? {
            if !f.is_text || f.sensitive {
                continue;
            }
            let Ok(text) = self.read_text(&f.path) else { continue };
            for (i, line) in text.lines().enumerate() {
                if line.to_lowercase().contains(&q) {
                    hits.push(SearchHit { path: f.path.clone(), line: i + 1, text: line.trim().chars().take(200).collect() });
                    if hits.len() >= max_hits {
                        return Ok(hits);
                    }
                }
            }
        }
        Ok(hits)
    }
}

/// A canonical path in the form ordinary programs accept. Strips the Windows verbatim prefix
/// (`\\?\\C:\\x` → `C:\\x`) when that is lossless; a no-op on other platforms.
pub fn display_path(p: &Path) -> PathBuf {
    dunce::simplified(p).to_path_buf()
}

/// `canonicalize` that never returns a Windows verbatim path when a plain one is equivalent.
pub fn canonical_dir(p: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(p)
}

pub fn to_slash(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Normalize a user/model-supplied relative path. Rejects anything that is not a plain
/// relative path made of normal components.
pub fn normalize_rel(rel: &str) -> Result<PathBuf> {
    let trimmed = rel.trim().trim_start_matches("./");
    if trimmed.is_empty() {
        return Err(GeniusError::PathRejected("empty path".into()));
    }
    if trimmed.contains('\0') {
        return Err(GeniusError::PathRejected("path contains a NUL byte".into()));
    }
    if trimmed.len() > 260 {
        return Err(GeniusError::PathRejected("path is too long".into()));
    }
    let unified = trimmed.replace('\\', "/");
    if unified.starts_with('/') || unified.contains(':') {
        return Err(GeniusError::PathRejected(format!("'{rel}' must be a relative path inside the project")));
    }
    let mut out = PathBuf::new();
    for part in unified.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(GeniusError::PathRejected(format!("'{rel}' tries to leave the project directory")));
        }
        // Windows reserved device names and trailing dots/spaces
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "LPT1", "LPT2", "LPT3"].contains(&stem.as_str()) || part.ends_with('.') || part.ends_with(' ') {
            return Err(GeniusError::PathRejected(format!("'{rel}' contains a reserved or invalid name")));
        }
        out.push(part);
    }
    if out.components().any(|c| !matches!(c, Component::Normal(_))) || out.as_os_str().is_empty() {
        return Err(GeniusError::PathRejected(format!("'{rel}' is not a valid project path")));
    }
    Ok(out)
}

pub fn is_secret_name(name: &str) -> bool {
    let l = name.to_lowercase();
    (l == ".env" || l.starts_with(".env.")) && l != ".env.example" && l != ".env.sample"
        || l.ends_with(".pem")
        || l.ends_with(".key")
        || l.ends_with(".p12")
        || l.ends_with(".pfx")
        || l == "id_rsa"
        || l == "id_ed25519"
        || l == ".npmrc"
        || l == ".netrc"
        || l.contains("credentials")
        || l.contains("secret")
}

const LOCKFILES: &[&str] = &["package-lock.json", "pnpm-lock.yaml", "yarn.lock", "bun.lockb", "Cargo.lock"];

pub fn is_sensitive(rel: &str, size: u64, is_text: bool) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    !is_text || size > 100 * 1024 || is_secret_name(name) || LOCKFILES.contains(&name)
}

fn is_text_path(rel: &str, abs: &Path) -> bool {
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    const BIN: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "webp", "ico", "bmp", "woff", "woff2", "ttf", "otf", "eot", "zip", "gz", "tar", "pdf", "mp4", "mp3", "wav", "exe", "dll", "so", "dylib", "wasm", "lockb", "db",
        "sqlite",
    ];
    if BIN.contains(&ext.as_str()) {
        return false;
    }
    use std::io::Read;
    let mut buf = [0u8; 1024];
    match std::fs::File::open(abs).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => !looks_binary(&buf[..n]),
        Err(_) => false,
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        let w = Workspace::new(d.path().join("proj")).unwrap();
        (d, w)
    }

    #[test]
    fn rejects_traversal_and_absolute_paths() {
        let (_d, w) = ws();
        for bad in ["../x", "a/../../x", "/etc/passwd", "C:\\Windows\\x", "..\\x", "", "a/\0b", "src/CON.ts", "foo./bar"] {
            assert!(w.resolve(bad).is_err(), "should reject {bad:?}");
        }
        assert!(w.resolve("src/App.tsx").is_ok());
        assert!(w.resolve("./src//App.tsx").is_ok());
    }

    #[test]
    fn rejects_protected_and_secret_targets() {
        let (_d, w) = ws();
        assert!(w.check_writable("node_modules/x/index.js").is_err());
        assert!(w.check_writable(".git/config").is_err());
        assert!(w.check_writable(".env").is_err());
        assert!(w.check_writable("config/.env.local").is_err());
        assert!(w.check_writable(".env.example").is_ok());
        assert!(w.check_writable("src/main.tsx").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        let (d, w) = ws();
        let outside = d.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, w.root().join("link")).unwrap();
        assert!(w.resolve("link/evil.txt").is_err());
        assert!(w.write_text("link/evil.txt", "x").is_err());
        assert!(!outside.join("evil.txt").exists());
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let (_d, w) = ws();
        w.write_text("src/components/Button.tsx", "export {}\n").unwrap();
        assert_eq!(w.read_text("src/components/Button.tsx").unwrap(), "export {}\n");
        let files = w.list_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/components/Button.tsx");
        w.delete_file("src/components/Button.tsx").unwrap();
        assert!(!w.root().join("src").exists(), "empty dirs cleaned");
    }

    #[test]
    fn size_limit_enforced() {
        let (_d, w) = ws();
        let big = "a".repeat(MAX_WRITE + 1);
        assert!(w.write_text("big.txt", &big).is_err());
    }

    #[test]
    fn listing_skips_ignored_and_flags_sensitive() {
        let (_d, w) = ws();
        std::fs::create_dir_all(w.root().join("node_modules/react")).unwrap();
        std::fs::write(w.root().join("node_modules/react/index.js"), "x").unwrap();
        std::fs::write(w.root().join(".env"), "API_KEY=1").unwrap();
        std::fs::write(w.root().join("logo.png"), [0u8, 1, 2]).unwrap();
        w.write_text("src/a.ts", "const a = 1;").unwrap();
        let files = w.list_files().unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec![".env", "logo.png", "src/a.ts"]);
        assert!(files[0].sensitive && files[1].sensitive && !files[2].sensitive);
        assert!(!files[1].is_text);
    }

    #[test]
    fn binary_read_is_rejected() {
        let (_d, w) = ws();
        std::fs::write(w.root().join("b.bin"), [0u8, 159, 146, 150]).unwrap();
        assert!(w.read_text("b.bin").is_err());
    }

    #[test]
    fn search_finds_lines_but_not_secrets() {
        let (_d, w) = ws();
        w.write_text("src/a.ts", "const title = 'Tasks';\nconst x = 2;").unwrap();
        std::fs::write(w.root().join(".env"), "TITLE_SECRET=Tasks").unwrap();
        let hits = w.search("tasks", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
    }
}
