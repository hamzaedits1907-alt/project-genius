//! User-provided files: import, parse, store, search — and safe ZIP project import.
//!
//! Imported content is **untrusted data**: it is labelled as such whenever it reaches a model,
//! never executed, and never written into a project except through a reviewable changeset.
//!
//! Supported: plain text, Markdown, CSV/TSV, JSON (validated), source code, PDF (text layer),
//! DOCX (document text), images (stored for models whose image input was verified), and ZIP
//! archives (listed and text files extracted as reference; or imported as a new project with
//! traversal, symlink, size and zip-bomb protection; `.git` and `node_modules` are skipped).

use crate::app::Genius;
use crate::db::now;
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use base64::Engine;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_IMPORT_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 2 * 1024 * 1024;
const ZIP_MAX_ENTRIES: usize = 5000;
const ZIP_MAX_TOTAL: u64 = 300 * 1024 * 1024;
const ZIP_MAX_ENTRY: u64 = 30 * 1024 * 1024;
const ZIP_MAX_RATIO: u64 = 200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub project_id: String,
    pub conversation_id: Option<String>,
    pub name: String,
    /// text | markdown | csv | json | code | pdf | docx | image | zip
    pub kind: String,
    pub mime: String,
    pub size: u64,
    /// message (one message only) | reference (indexed project reference material)
    pub scope: String,
    /// ready | failed
    pub status: String,
    pub error: Option<String>,
    #[serde(skip_serializing)]
    pub text: String,
    #[serde(skip_serializing)]
    pub data_b64: Option<String>,
    pub chars: usize,
    pub meta: Value,
    pub created_at: String,
}

const CODE_EXT: &[&str] = &[
    "ts", "tsx", "js", "jsx", "mjs", "cjs", "css", "scss", "html", "htm", "vue", "svelte", "py", "rs", "go", "java", "kt", "swift", "c", "h", "cpp", "cs", "rb", "php", "sql", "sh", "ps1", "yml",
    "yaml", "toml", "xml", "svg", "ini", "graphql",
];

/// Classify by extension (content is validated by the parser).
pub fn classify(name: &str) -> Option<(&'static str, &'static str)> {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    Some(match ext.as_str() {
        "txt" | "log" => ("text", "text/plain"),
        "md" | "markdown" => ("markdown", "text/markdown"),
        "csv" => ("csv", "text/csv"),
        "tsv" => ("csv", "text/tab-separated-values"),
        "json" => ("json", "application/json"),
        "pdf" => ("pdf", "application/pdf"),
        "docx" => ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "zip" => ("zip", "application/zip"),
        e if CODE_EXT.contains(&e) => ("code", "text/plain"),
        _ => return None,
    })
}

pub fn supported_extensions() -> Vec<String> {
    let mut v: Vec<String> = ["txt", "log", "md", "markdown", "csv", "tsv", "json", "pdf", "docx", "png", "jpg", "jpeg", "gif", "webp", "zip"].iter().map(|s| s.to_string()).collect();
    v.extend(CODE_EXT.iter().map(|s| s.to_string()));
    v
}

fn utf8(bytes: &[u8], name: &str) -> Result<String> {
    let s = String::from_utf8(bytes.to_vec()).map_err(|_| GeniusError::Validation(format!("{name} is not valid UTF-8 text")))?;
    Ok(s.trim_start_matches('\u{feff}').to_string())
}

fn cap_text(mut s: String) -> (String, bool) {
    if s.len() <= MAX_TEXT_CHARS {
        return (s, false);
    }
    let mut end = MAX_TEXT_CHARS;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    (s, true)
}

/// Extract text from DOCX (word/document.xml): text runs, tabs and paragraph breaks.
pub fn docx_text(bytes: &[u8]) -> Result<String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| GeniusError::Validation(format!("not a valid DOCX file: {e}")))?;
    let mut xml = String::new();
    let entry = zip.by_name("word/document.xml").map_err(|_| GeniusError::Validation("DOCX has no word/document.xml".into()))?;
    if entry.size() > ZIP_MAX_ENTRY {
        return Err(GeniusError::Validation("DOCX document is too large".into()));
    }
    entry.take(ZIP_MAX_ENTRY).read_to_string(&mut xml)?;
    let re = regex::Regex::new(r"(?s)<w:t(?:\s[^>]*)?>(.*?)</w:t>|<w:tab/>|</w:p>|<w:br/>").expect("valid regex");
    let mut out = String::new();
    for c in re.captures_iter(&xml) {
        match c.get(1) {
            Some(t) => out.push_str(&unescape_xml(t.as_str())),
            None => match &c[0] {
                "<w:tab/>" => out.push('\t'),
                _ => out.push('\n'),
            },
        }
    }
    Ok(out.trim().to_string())
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Validate a ZIP's central directory against bomb/traversal limits; returns safe entries
/// (relative path, uncompressed size) excluding directories, `.git/` and `node_modules/`.
pub fn zip_safe_entries(bytes: &[u8]) -> Result<Vec<(usize, String, u64)>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| GeniusError::Validation(format!("not a valid ZIP archive: {e}")))?;
    if zip.len() > ZIP_MAX_ENTRIES {
        return Err(GeniusError::Validation(format!("archive has {} entries (limit {ZIP_MAX_ENTRIES})", zip.len())));
    }
    let mut total = 0u64;
    let mut out = Vec::new();
    for i in 0..zip.len() {
        let e = zip.by_index_raw(i).map_err(|e| GeniusError::Validation(format!("corrupt archive entry: {e}")))?;
        if e.is_dir() {
            continue;
        }
        if e.is_symlink() {
            return Err(GeniusError::Validation(format!("archive contains a symbolic link ({}); refusing to extract", e.name())));
        }
        let Some(rel) = e.enclosed_name() else {
            return Err(GeniusError::Validation(format!("archive entry escapes the target folder: {}", e.name())));
        };
        let rel = crate::workspace::to_slash(&rel);
        crate::workspace::normalize_rel(&rel).map_err(|_| GeniusError::Validation(format!("unsafe path in archive: {rel}")))?;
        if rel.split('/').any(|seg| seg == ".git" || seg == "node_modules") {
            continue;
        }
        if e.size() > ZIP_MAX_ENTRY {
            return Err(GeniusError::Validation(format!("{rel} is {} MB uncompressed (limit {} MB)", e.size() / 1_048_576, ZIP_MAX_ENTRY / 1_048_576)));
        }
        if e.compressed_size() > 0 && e.size() / e.compressed_size().max(1) > ZIP_MAX_RATIO && e.size() > 1_048_576 {
            return Err(GeniusError::Validation(format!("{rel} has a suspicious compression ratio (possible zip bomb)")));
        }
        total += e.size();
        if total > ZIP_MAX_TOTAL {
            return Err(GeniusError::Validation(format!("archive expands to more than {} MB", ZIP_MAX_TOTAL / 1_048_576)));
        }
        out.push((i, rel, e.size()));
    }
    Ok(out)
}

/// Extract validated entries into `dest` (which must not exist yet). Never executes anything.
pub fn extract_zip(bytes: &[u8], dest: &Path) -> Result<usize> {
    let entries = zip_safe_entries(bytes)?;
    if dest.exists() {
        return Err(GeniusError::Conflict(format!("{} already exists", dest.display())));
    }
    std::fs::create_dir_all(dest)?;
    // Strip a single common top-level folder ("project-main/...") so the project root is the content.
    let strip = common_root(&entries.iter().map(|e| e.1.as_str()).collect::<Vec<_>>());
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| GeniusError::Validation(e.to_string()))?;
    let mut n = 0;
    for (i, rel, size) in &entries {
        let rel = strip.as_ref().map(|s| rel.strip_prefix(s.as_str()).unwrap_or(rel)).unwrap_or(rel);
        let rel = rel.trim_start_matches('/');
        if rel.is_empty() {
            continue;
        }
        let target = dest.join(crate::workspace::normalize_rel(rel)?);
        if !target.starts_with(dest) {
            return Err(GeniusError::Validation(format!("unsafe path in archive: {rel}")));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = zip.by_index(*i).map_err(|e| GeniusError::Validation(e.to_string()))?;
        let mut out = std::fs::File::create(&target)?;
        // Enforce the declared size while streaming, in case the header lies.
        let copied = std::io::copy(&mut (&mut f).take(*size + 1), &mut out)?;
        if copied > *size {
            return Err(GeniusError::Validation(format!("{rel} is larger than its header declares")));
        }
        n += 1;
    }
    Ok(n)
}

fn common_root(paths: &[&str]) -> Option<String> {
    let first = paths.first()?.split_once('/')?.0;
    if paths.iter().all(|p| p.split_once('/').is_some_and(|(a, _)| a == first)) {
        Some(format!("{first}/"))
    } else {
        None
    }
}

/// Parse bytes into (kind, mime, text, image_b64, meta).
pub fn parse(name: &str, bytes: &[u8]) -> Result<(String, String, String, Option<String>, Value)> {
    let (kind, mime) = classify(name).ok_or_else(|| GeniusError::Validation(format!("unsupported file type: {name}. Supported: {}", supported_extensions().join(", "))))?;
    let mut meta = json!({});
    let text = match kind {
        "text" | "markdown" | "code" => utf8(bytes, name)?,
        "csv" => {
            let t = utf8(bytes, name)?;
            meta["rows"] = json!(t.lines().count());
            meta["header"] = json!(t.lines().next().unwrap_or("").chars().take(300).collect::<String>());
            t
        }
        "json" => {
            let t = utf8(bytes, name)?;
            serde_json::from_str::<Value>(&t).map_err(|e| GeniusError::Validation(format!("{name} is not valid JSON: {e}")))?;
            t
        }
        "pdf" => {
            let t = pdf_extract::extract_text_from_mem(bytes).map_err(|e| GeniusError::Validation(format!("could not read text from the PDF: {e}")))?;
            if t.trim().is_empty() {
                return Err(GeniusError::Validation("the PDF has no extractable text layer (scanned image PDFs are not supported)".into()));
            }
            t
        }
        "docx" => docx_text(bytes)?,
        "image" => {
            if bytes.len() as u64 > MAX_IMAGE_BYTES {
                return Err(GeniusError::Validation(format!("images are limited to {} MB", MAX_IMAGE_BYTES / 1_048_576)));
            }
            return Ok((kind.into(), mime.into(), String::new(), Some(base64::engine::general_purpose::STANDARD.encode(bytes)), meta));
        }
        "zip" => {
            let entries = zip_safe_entries(bytes)?;
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| GeniusError::Validation(e.to_string()))?;
            let mut s = format!("ZIP ARCHIVE {name}: {} files\n", entries.len());
            for (_, rel, size) in &entries {
                s.push_str(&format!("- {rel} ({size} B)\n"));
            }
            let mut included = 0;
            for (i, rel, size) in &entries {
                if s.len() > MAX_TEXT_CHARS / 2 || *size > 400_000 {
                    continue;
                }
                let Some((k, _)) = classify(rel) else { continue };
                if !matches!(k, "text" | "markdown" | "code" | "csv" | "json") || crate::workspace::is_secret_name(rel.rsplit('/').next().unwrap_or(rel)) {
                    continue;
                }
                let mut buf = Vec::new();
                zip.by_index(*i).map_err(|e| GeniusError::Validation(e.to_string()))?.take(*size + 1).read_to_end(&mut buf)?;
                if let Ok(t) = String::from_utf8(buf) {
                    s.push_str(&format!("\n===== {rel} =====\n{t}\n"));
                    included += 1;
                }
            }
            meta["files"] = json!(entries.len());
            meta["textFilesIncluded"] = json!(included);
            s
        }
        _ => unreachable!(),
    };
    let (text, truncated) = cap_text(text);
    meta["truncated"] = json!(truncated);
    Ok((kind.into(), mime.into(), text, None, meta))
}

fn row_to_attachment(r: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    let meta: String = r.get("meta_json")?;
    let text: String = r.get("text")?;
    Ok(Attachment {
        id: r.get("id")?,
        project_id: r.get("project_id")?,
        conversation_id: r.get("conversation_id")?,
        name: r.get("name")?,
        kind: r.get("kind")?,
        mime: r.get("mime")?,
        size: r.get::<_, i64>("size")? as u64,
        scope: r.get("scope")?,
        status: r.get("status")?,
        error: r.get("error")?,
        chars: text.len(),
        text,
        data_b64: r.get("data_b64")?,
        meta: serde_json::from_str(&meta).unwrap_or(Value::Null),
        created_at: r.get("created_at")?,
    })
}

impl crate::db::Db {
    pub fn count_attachments(&self, project_id: &str) -> Result<i64> {
        Ok(self.conn().query_row("SELECT COUNT(*) FROM attachments WHERE project_id = ?1 AND scope = 'reference' AND status = 'ready'", params![project_id], |r| r.get(0))?)
    }
}

impl Genius {
    /// Import a file from disk (chosen by the user in a native dialog).
    pub fn import_attachment(&self, project_id: &str, path: &str, scope: &str, conversation_id: Option<&str>) -> Result<Attachment> {
        let p = PathBuf::from(path);
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).ok_or_else(|| GeniusError::Validation("not a file".into()))?;
        let meta = std::fs::metadata(&p)?;
        if !meta.is_file() {
            return Err(GeniusError::Validation(format!("{name} is not a regular file")));
        }
        if meta.len() > MAX_IMPORT_BYTES {
            return Err(GeniusError::Validation(format!("{name} is {} MB; the import limit is {} MB", meta.len() / 1_048_576, MAX_IMPORT_BYTES / 1_048_576)));
        }
        let bytes = std::fs::read(&p)?;
        self.import_attachment_bytes(project_id, &name, &bytes, scope, conversation_id)
    }

    /// Import bytes (drag-and-drop). Parse failures are stored as `failed` with the reason so
    /// the user sees exactly what happened.
    pub fn import_attachment_bytes(&self, project_id: &str, name: &str, bytes: &[u8], scope: &str, conversation_id: Option<&str>) -> Result<Attachment> {
        self.db.get_project(project_id)?;
        if !matches!(scope, "message" | "reference") {
            return Err(GeniusError::Validation("scope must be 'message' or 'reference'".into()));
        }
        if bytes.len() as u64 > MAX_IMPORT_BYTES {
            return Err(GeniusError::Validation(format!("{name} exceeds the {} MB import limit", MAX_IMPORT_BYTES / 1_048_576)));
        }
        let name: String = name.chars().filter(|c| !c.is_control() && *c != '/' && *c != '\\').take(200).collect();
        let mut ev = self.bus.start(Some(project_id), None, EventKind::FileRead, EventStatus::Running, format!("Importing {name}"), json!({ "name": name, "size": bytes.len(), "scope": scope }));
        let (kind, mime) = classify(&name).map(|(k, m)| (k.to_string(), m.to_string())).unwrap_or(("unsupported".into(), "application/octet-stream".into()));
        let (status, error, text, data, meta, kind, mime) = match parse(&name, bytes) {
            Ok((k, m, t, d, meta)) => ("ready", None, t, d, meta, k, m),
            Err(e) => ("failed", Some(e.to_string()), String::new(), None, json!({}), kind, mime),
        };
        let a = Attachment {
            id: uuid::Uuid::new_v4().to_string(),
            project_id: project_id.into(),
            conversation_id: conversation_id.map(String::from),
            name: name.clone(),
            kind,
            mime,
            size: bytes.len() as u64,
            scope: scope.into(),
            status: status.into(),
            error: error.clone(),
            chars: text.len(),
            text,
            data_b64: data,
            meta,
            created_at: now(),
        };
        self.db.conn().execute(
            "INSERT INTO attachments(id,project_id,conversation_id,name,kind,mime,size,scope,status,error,text,data_b64,meta_json,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![a.id, a.project_id, a.conversation_id, a.name, a.kind, a.mime, a.size as i64, a.scope, a.status, a.error, a.text, a.data_b64, a.meta.to_string(), a.created_at],
        )?;
        match &error {
            None => {
                ev.set_title(format!("Imported {name} ({})", a.kind));
                ev.update(EventStatus::Completed, json!({ "attachmentId": a.id, "chars": a.chars, "kind": a.kind }));
            }
            Some(e) => ev.update(EventStatus::Failed, json!({ "error": { "kind": "validation", "message": e } })),
        }
        Ok(a)
    }

    pub fn list_attachments(&self, project_id: &str) -> Result<Vec<Attachment>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare("SELECT * FROM attachments WHERE project_id = ?1 ORDER BY created_at DESC, rowid DESC")?;
        let rows = stmt.query_map(params![project_id], row_to_attachment)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn attachment(&self, project_id: &str, id: &str) -> Result<Attachment> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare("SELECT * FROM attachments WHERE id = ?1 AND project_id = ?2")?;
        let mut rows = stmt.query_map(params![id, project_id], row_to_attachment)?;
        rows.next().transpose()?.ok_or_else(|| GeniusError::NotFound(format!("imported file {id}")))
    }

    /// Text preview for the UI (first part of the extracted text).
    pub fn attachment_preview(&self, project_id: &str, id: &str) -> Result<Value> {
        let a = self.attachment(project_id, id)?;
        let preview: String = a.text.chars().take(4000).collect();
        Ok(json!({ "attachment": a, "preview": preview }))
    }

    pub fn set_attachment_scope(&self, project_id: &str, id: &str, scope: &str) -> Result<Attachment> {
        if !matches!(scope, "message" | "reference") {
            return Err(GeniusError::Validation("scope must be 'message' or 'reference'".into()));
        }
        self.db.conn().execute("UPDATE attachments SET scope = ?3 WHERE id = ?1 AND project_id = ?2", params![id, project_id, scope])?;
        self.attachment(project_id, id)
    }

    pub fn delete_attachment(&self, project_id: &str, id: &str) -> Result<()> {
        let n = self.db.conn().execute("DELETE FROM attachments WHERE id = ?1 AND project_id = ?2", params![id, project_id])?;
        if n == 0 {
            return Err(GeniusError::NotFound(format!("imported file {id}")));
        }
        self.bus.record(Some(project_id), None, EventKind::FileWrite, EventStatus::Completed, "Removed an imported file", json!({ "attachmentId": id }));
        Ok(())
    }

    /// Attachments sent with one message (any scope), validated to belong to the project.
    pub fn attachments_for_message(&self, project_id: &str, ids: &[String]) -> Result<Vec<Attachment>> {
        let mut out = Vec::new();
        for id in ids.iter().take(10) {
            let a = self.attachment(project_id, id)?;
            if a.status != "ready" {
                return Err(GeniusError::Validation(format!("{} could not be imported: {}", a.name, a.error.unwrap_or_default())));
            }
            if a.kind == "image" && self.settings()?.model.as_deref().and_then(|m| self.vision_probe(m)) != Some(true) {
                return Err(GeniusError::Validation(format!(
                    "{} is an image, but image input has not been verified for the selected model. Use Settings → AI provider → Test image input, or choose a vision model.",
                    a.name
                )));
            }
            out.push(a);
        }
        Ok(out)
    }

    /// Search reference-scoped attachments. Returns (id, name, snippet).
    pub fn search_attachments(&self, project_id: &str, query: &str, max: usize) -> Result<Vec<(String, String, String)>> {
        let words: Vec<String> = query.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2).map(String::from).collect();
        if words.is_empty() {
            return Err(GeniusError::Validation("query needs at least one word of 3+ characters".into()));
        }
        let mut scored = Vec::new();
        for a in self.list_attachments(project_id)?.into_iter().filter(|a| a.scope == "reference" && a.status == "ready" && !a.text.is_empty()) {
            let lower = a.text.to_lowercase();
            for (pos, _) in lower.match_indices(words[0].as_str()).take(50) {
                let start = floor_char(&a.text, pos.saturating_sub(160));
                let end = floor_char(&a.text, (pos + 240).min(a.text.len()));
                let snip = a.text[start..end].replace('\n', " ");
                let score = words.iter().filter(|w| snip.to_lowercase().contains(w.as_str())).count();
                scored.push((score, a.id.clone(), a.name.clone(), snip));
            }
        }
        scored.sort_by_key(|s| std::cmp::Reverse(s.0));
        Ok(scored.into_iter().take(max).map(|(_, i, n, s)| (i, n, s)).collect())
    }

    /// Import a ZIP archive of an existing project as a new Genius project (safe extraction).
    pub fn import_zip_project(&self, zip_path: &str) -> Result<crate::db::ProjectRow> {
        let p = PathBuf::from(zip_path);
        let meta = std::fs::metadata(&p)?;
        if meta.len() > MAX_IMPORT_BYTES * 4 {
            return Err(GeniusError::Validation("archive is larger than 100 MB".into()));
        }
        let bytes = std::fs::read(&p)?;
        let stem: String = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "imported".into())
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
            .collect();
        let root = self.workspace_dir()?;
        std::fs::create_dir_all(&root)?;
        let mut dest = root.join(&stem);
        let mut i = 2;
        while dest.exists() {
            dest = root.join(format!("{stem}-{i}"));
            i += 1;
        }
        let n = match extract_zip(&bytes, &dest) {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(e);
            }
        };
        let project = self.import_project(&dest.to_string_lossy(), Some(&stem))?;
        self.bus.record(
            Some(&project.id),
            None,
            EventKind::FileWrite,
            EventStatus::Completed,
            format!("Extracted {n} files from {}", p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()),
            json!({ "files": n, "note": "No scripts were run. Review package.json scripts before installing." }),
        );
        Ok(project)
    }

    // ---------------------------------------------------------------- vision verification

    pub fn vision_probe(&self, model: &str) -> Option<bool> {
        self.db.get_setting("vision_probes").ok().flatten().and_then(|v| v.get(model).and_then(|r| r.get("ok")).and_then(|b| b.as_bool()))
    }

    /// Send one tiny real image to the selected model. Success = the request was accepted and
    /// produced an answer; the answer itself is returned so the user can judge it.
    pub async fn probe_vision(&self, model: &str) -> Result<Value> {
        let (client, s) = self.client()?;
        // 8x8 solid red PNG.
        const RED_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC";
        let msg = crate::nvidia::ChatMessage {
            role: "user".into(),
            content: "What is the dominant colour of this image? Answer with one word.".into(),
            images: vec![format!("data:image/png;base64,{RED_PNG}")],
        };
        let opts = crate::nvidia::ChatOptions { temperature: 0.0, top_p: 1.0, max_tokens: 16, stream: false };
        let cancel = tokio_util::sync::CancellationToken::new();
        let res = client.chat(model, &[msg], &opts, &cancel, |_| {}).await;
        let (ok, answer, error) = match res {
            Ok(r) => (!r.content.trim().is_empty(), r.content.trim().to_string(), None),
            Err(e) => (false, String::new(), Some(e.to_string())),
        };
        let mut all = self.db.get_setting("vision_probes")?.unwrap_or_else(|| json!({}));
        all[model] = json!({ "ok": ok, "answer": answer, "error": error, "at": now(), "baseUrl": s.base_url });
        self.db.set_setting("vision_probes", &all)?;
        Ok(all[model].clone())
    }
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                w.write_all(data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn classifies_and_parses_text_formats() {
        assert_eq!(classify("Spec.MD").unwrap().0, "markdown");
        assert_eq!(classify("a.tsx").unwrap().0, "code");
        assert!(classify("a.exe").is_none());
        let (k, _, t, _, m) = parse("d.csv", b"a,b\n1,2\n").unwrap();
        assert_eq!((k.as_str(), t.as_str(), m["rows"].as_u64()), ("csv", "a,b\n1,2\n", Some(2)));
        assert!(parse("bad.json", b"{nope").unwrap_err().to_string().contains("not valid JSON"));
        assert!(parse("bin.txt", &[0xff, 0xfe, 0x00]).unwrap_err().to_string().contains("UTF-8"));
        assert!(parse("x.exe", b"MZ").unwrap_err().to_string().contains("unsupported"));
    }

    #[test]
    fn docx_text_is_extracted() {
        let xml = br#"<w:document><w:body><w:p><w:r><w:t>Login page</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve">must have &amp; validate </w:t></w:r><w:r><w:tab/><w:t>email</w:t></w:r></w:p></w:body></w:document>"#;
        let z = make_zip(&[("word/document.xml", xml)]);
        assert_eq!(docx_text(&z).unwrap(), "Login page\nmust have & validate \temail");
        let (k, _, t, _, _) = parse("req.docx", &z).unwrap();
        assert_eq!(k, "docx");
        assert!(t.contains("Login page"));
    }

    #[test]
    fn images_are_stored_as_base64_with_size_limit() {
        let (k, mime, t, d, _) = parse("shot.png", b"\x89PNG....").unwrap();
        assert_eq!((k.as_str(), mime.as_str(), t.as_str()), ("image", "image/png", ""));
        assert!(d.is_some());
        let big = vec![0u8; (MAX_IMAGE_BYTES + 1) as usize];
        assert!(parse("big.png", &big).is_err());
    }

    #[test]
    fn zip_rejects_traversal_and_skips_git_and_node_modules() {
        let evil = make_zip(&[("../evil.txt", b"x")]);
        assert!(zip_safe_entries(&evil).unwrap_err().to_string().contains("escapes"));
        let ok = make_zip(&[("proj/src/App.tsx", b"export {}"), ("proj/.git/hooks/pre-commit", b"rm -rf /"), ("proj/node_modules/x/index.js", b"x"), ("proj/package.json", b"{}")]);
        let names: Vec<String> = zip_safe_entries(&ok).unwrap().into_iter().map(|e| e.1).collect();
        assert_eq!(names, vec!["proj/src/App.tsx", "proj/package.json"]);
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out");
        assert_eq!(extract_zip(&ok, &dest).unwrap(), 2);
        assert!(dest.join("src/App.tsx").is_file());
        assert!(!dest.join(".git").exists());
        assert!(extract_zip(&ok, &dest).is_err(), "never extracts over an existing folder");
    }

    #[test]
    fn zip_bomb_is_rejected() {
        let zeros = vec![0u8; 20 * 1024 * 1024];
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            w.start_file("zeros.txt", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            w.write_all(&zeros).unwrap();
            w.finish().unwrap();
        }
        let err = zip_safe_entries(&buf.into_inner()).unwrap_err().to_string();
        assert!(err.contains("zip bomb"), "{err}");
    }

    #[test]
    fn zip_reference_lists_and_includes_text_but_not_secrets() {
        let z = make_zip(&[("a/README.md", b"# Hello spec"), ("a/.env", b"SECRET=1"), ("a/logo.png", b"\x89PNG")]);
        let (_, _, t, _, m) = parse("p.zip", &z).unwrap();
        assert!(t.contains("# Hello spec"));
        assert!(t.contains("a/.env"), "listed by name");
        assert!(!t.contains("SECRET=1"), "secret contents are never extracted");
        assert_eq!(m["textFilesIncluded"], 1);
    }
}
