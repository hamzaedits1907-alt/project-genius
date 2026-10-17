//! Runtime errors from the live preview ("Genius sees what the user sees").
//!
//! The injected preview script (see `inspector.rs`) reports uncaught exceptions, unhandled
//! rejections, console errors/warnings, failed resource loads, Vite's error overlay and
//! blank pages to the Genius window, which forwards them here. They are kept in memory only
//! (a small ring buffer per project), merged by identity, redacted, and cleared whenever the
//! preview restarts or a changeset is applied so stale errors never linger.
//!
//! Everything here comes from code running in the preview, i.e. untrusted input: fields are
//! clipped and validated, and the text is only ever shown to the user or the model as data.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

use crate::app::Genius;
use crate::error::Result;
use crate::events::{channels, EventKind, EventStatus};
use crate::redact::redact;

/// Distinct errors kept per project.
pub const MAX_ERRORS: usize = 50;
/// Errors accepted per report call (the client batches and rate-limits on its side too).
const MAX_PER_REPORT: usize = 50;
const KINDS: &[&str] = &["error", "rejection", "console", "resource", "blank", "vite"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRuntimeError {
    /// "error" | "rejection" | "console" | "resource" | "blank" | "vite"
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub column: Option<u32>,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub at: String,
    #[serde(default = "one")]
    pub count: u32,
}

fn one() -> u32 {
    1
}

pub(crate) type Store = Arc<Mutex<HashMap<String, VecDeque<PreviewRuntimeError>>>>;

static ORIGIN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://(?:127\.0\.0\.1|localhost|\[::1\])(?::\d+)?/").unwrap());
static CACHE_BUST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\?(?:t|v|import)(?:=[0-9A-Za-z]+)?(?:&(?:t|v|import)(?:=[0-9A-Za-z]+)?)*").unwrap());

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Project-relative paths instead of dev-server URLs (`http://127.0.0.1:5173/src/App.tsx?t=1`
/// → `src/App.tsx`), redacted and clipped.
fn tidy(s: &str, max: usize) -> String {
    let s = ORIGIN.replace_all(s, "");
    let s = CACHE_BUST.replace_all(&s, "");
    clip(&redact(&s), max)
}

fn sanitize(mut e: PreviewRuntimeError) -> PreviewRuntimeError {
    if !KINDS.contains(&e.kind.as_str()) {
        e.kind = "error".into();
    }
    e.message = tidy(e.message.trim(), 1000);
    if e.message.is_empty() {
        e.message = if e.kind == "blank" { "The page rendered no visible content".into() } else { "(no message)".into() };
    }
    e.source = e.source.map(|s| tidy(s.trim(), 300)).filter(|s| !s.is_empty());
    e.stack = e.stack.map(|s| tidy(s.trim(), 2000)).filter(|s| !s.is_empty());
    e.page = clip(e.page.trim(), 300);
    if e.page.is_empty() {
        e.page = "/".into();
    }
    if chrono::DateTime::parse_from_rfc3339(&e.at).is_err() {
        e.at = crate::db::now();
    }
    e.count = e.count.clamp(1, 1_000_000);
    e
}

fn same(a: &PreviewRuntimeError, b: &PreviewRuntimeError) -> bool {
    a.kind == b.kind && a.message == b.message && a.source == b.source && a.line == b.line
}

/// Merge `incoming` into `buf`; returns the errors that were not seen before.
pub(crate) fn merge_into(buf: &mut VecDeque<PreviewRuntimeError>, incoming: Vec<PreviewRuntimeError>) -> Vec<PreviewRuntimeError> {
    let mut fresh = Vec::new();
    for e in incoming.into_iter().take(MAX_PER_REPORT).map(sanitize) {
        if let Some(i) = buf.iter().position(|x| same(x, &e)) {
            let mut cur = buf.remove(i).expect("index from position");
            cur.count = cur.count.saturating_add(e.count);
            cur.at = e.at;
            cur.page = e.page;
            cur.column = e.column.or(cur.column);
            cur.stack = e.stack.or(cur.stack);
            buf.push_back(cur);
        } else {
            fresh.push(e.clone());
            buf.push_back(e);
            while buf.len() > MAX_ERRORS {
                buf.pop_front();
            }
        }
    }
    fresh
}

fn location(e: &PreviewRuntimeError) -> Option<String> {
    let src = e.source.as_deref().filter(|s| !s.starts_with("console."))?;
    Some(match (e.line, e.column) {
        (Some(l), Some(c)) => format!("{src}:{l}:{c}"),
        (Some(l), None) => format!("{src}:{l}"),
        _ => src.to_string(),
    })
}

fn label(e: &PreviewRuntimeError) -> &'static str {
    match e.kind.as_str() {
        "rejection" => "Unhandled promise rejection",
        "console" if e.source.as_deref() == Some("console.warn") => "console.warn",
        "console" => "console.error",
        "resource" => "Failed to load resource",
        "blank" => "Blank page",
        "vite" => "Vite error overlay",
        _ => "Uncaught error",
    }
}

impl Genius {
    /// Record errors reported by the preview. New distinct errors are announced in the
    /// activity feed; the full current list is pushed on `genius://preview-errors`.
    pub fn report_preview_errors(&self, project_id: &str, errors: Vec<PreviewRuntimeError>) -> Result<()> {
        let p = self.db.get_project(project_id)?;
        if errors.is_empty() {
            return Ok(());
        }
        // Vite's overlay shows absolute paths; keep them project-relative like everything else.
        // On Windows the project path may appear with `\\?\` (verbatim) or without it, with
        // either slash style, and Vite mixes `/` into the remainder — strip every spelling.
        let plain = p.path.trim_start_matches(r"\\?\").trim_start_matches("//?/");
        let mut prefixes: Vec<String> = Vec::new();
        for base in [plain.to_string(), format!(r"\\?\{plain}")] {
            for spelled in [base.replace('\\', "/"), base.replace('/', "\\")] {
                for sep in ["/", "\\"] {
                    let pre = format!("{spelled}{sep}");
                    if !prefixes.contains(&pre) {
                        prefixes.push(pre);
                    }
                }
            }
        }
        // Longest first, so the verbatim spelling is removed before its plain suffix.
        prefixes.sort_by_key(|p| std::cmp::Reverse(p.len()));
        let rel = |mut s: String| {
            for pre in &prefixes {
                s = s.replace(pre.as_str(), "");
            }
            s
        };
        let errors = errors.into_iter().map(|e| PreviewRuntimeError { message: rel(e.message), source: e.source.map(rel), stack: e.stack.map(rel), ..e }).collect();
        let (fresh, all) = {
            let mut map = self.runtime_errors.lock().unwrap();
            let buf = map.entry(project_id.to_string()).or_default();
            let fresh = merge_into(buf, errors);
            (fresh, buf.iter().cloned().collect::<Vec<_>>())
        };
        for e in &fresh {
            let title = if e.kind == "blank" { "Preview shows a blank page".to_string() } else { format!("Preview {}: {}", label(e).to_lowercase(), clip(e.message.lines().next().unwrap_or(""), 140)) };
            self.bus.record(Some(project_id), None, EventKind::Preview, EventStatus::Failed, title, json!({ "runtimeError": e }));
        }
        self.bus.push(channels::PREVIEW_ERRORS, json!({ "projectId": project_id, "errors": all }));
        Ok(())
    }

    /// Current runtime errors of the project's preview, oldest first.
    pub fn preview_errors(&self, project_id: &str) -> Vec<PreviewRuntimeError> {
        self.runtime_errors.lock().unwrap().get(project_id).map(|b| b.iter().cloned().collect()).unwrap_or_default()
    }

    pub fn clear_preview_errors(&self, project_id: &str) {
        let had = self.runtime_errors.lock().unwrap().remove(project_id).is_some_and(|b| !b.is_empty());
        if had {
            self.bus.push(channels::PREVIEW_ERRORS, json!({ "projectId": project_id, "errors": [] }));
        }
    }

    /// Compact plain text for the model: one block per error with count, location and the
    /// first stack lines.
    pub fn render_preview_errors(errors: &[PreviewRuntimeError]) -> String {
        let mut out = String::new();
        for (i, e) in errors.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!("[{}] {}", i + 1, label(e)));
            if e.count > 1 {
                out.push_str(&format!(" (×{})", e.count));
            }
            if let Some(loc) = location(e) {
                out.push_str(&format!(" at {loc}"));
            }
            out.push_str(&format!(" on page {}\n", e.page));
            for l in e.message.lines().take(20) {
                out.push_str(&format!("  {}\n", l.trim_end()));
            }
            if let Some(st) = &e.stack {
                let first = e.message.lines().next().unwrap_or("").trim();
                let lines: Vec<&str> = st.lines().map(str::trim_end).filter(|l| !l.trim().is_empty() && l.trim() != first).take(15).collect();
                if !lines.is_empty() {
                    out.push_str("  Stack:\n");
                    for l in lines {
                        out.push_str(&format!("    {}\n", l.trim()));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingEmitter;

    fn err(kind: &str, msg: &str, line: Option<u32>) -> PreviewRuntimeError {
        PreviewRuntimeError { kind: kind.into(), message: msg.into(), source: Some("http://127.0.0.1:5173/src/W.tsx?t=1712".into()), line, column: Some(3), stack: None, page: "/".into(), at: "2026-10-01T00:00:00Z".into(), count: 1 }
    }

    #[test]
    fn merges_by_identity_and_counts() {
        let mut buf = VecDeque::new();
        let fresh = merge_into(&mut buf, vec![err("error", "boom", Some(4)), err("error", "boom", Some(4)), err("error", "boom", Some(5))]);
        assert_eq!(fresh.len(), 2, "same kind/message/source/line merged");
        assert_eq!(buf.len(), 2);
        assert_eq!(buf[0].count, 2);
        assert_eq!(buf[0].source.as_deref(), Some("src/W.tsx"), "dev-server origin and cache-busting query stripped");
        let fresh = merge_into(&mut buf, vec![PreviewRuntimeError { count: 3, ..err("error", "boom", Some(4)) }]);
        assert!(fresh.is_empty());
        assert_eq!(buf.back().unwrap().count, 5, "merged entry moves to the back");
    }

    #[test]
    fn ring_buffer_is_bounded_and_input_sanitized() {
        let mut buf = VecDeque::new();
        for i in 0..(MAX_ERRORS + 15) {
            merge_into(&mut buf, vec![err("error", &format!("e{i}"), None)]);
        }
        assert_eq!(buf.len(), MAX_ERRORS);
        assert_eq!(buf[0].message, "e15", "oldest dropped first");
        let mut buf = VecDeque::new();
        let long = "x".repeat(10_000);
        merge_into(&mut buf, vec![PreviewRuntimeError { kind: "<script>".into(), stack: Some(long.clone()), at: "nonsense".into(), count: 0, page: String::new(), ..err("error", &long, None) }]);
        let e = &buf[0];
        assert_eq!(e.kind, "error");
        assert!(e.message.len() < 1100 && e.stack.as_ref().unwrap().len() < 2100);
        assert_eq!((e.count, e.page.as_str()), (1, "/"));
        assert!(chrono::DateTime::parse_from_rfc3339(&e.at).is_ok());
    }

    #[test]
    fn redacts_secrets() {
        let mut buf = VecDeque::new();
        merge_into(&mut buf, vec![PreviewRuntimeError { stack: Some("at fetch (Bearer abcdefghijklmnop123)".into()), ..err("rejection", "401 for key nvapi-abcdefghijklmnopqrstu", None) }]);
        assert!(!buf[0].message.contains("nvapi-abc") && buf[0].message.contains("[REDACTED]"));
        assert!(!buf[0].stack.as_ref().unwrap().contains("abcdefghijklmnop123"));
    }

    #[test]
    fn renders_compact_text() {
        let mut stack = String::from("TypeError: Cannot read properties of undefined (reading 'map')\n");
        for i in 0..30 {
            stack.push_str(&format!("    at f{i} (src/W.tsx:{i}:1)\n"));
        }
        let errors = vec![
            PreviewRuntimeError { count: 3, stack: Some(stack), source: Some("src/W.tsx".into()), line: Some(42), column: Some(13), ..err("error", "TypeError: Cannot read properties of undefined (reading 'map')", None) },
            PreviewRuntimeError { source: None, line: None, column: None, ..err("blank", "Nothing rendered in #root", None) },
            PreviewRuntimeError { source: Some("console.warn".into()), line: None, ..err("console", "Warning: Each child needs a key", None) },
        ];
        let t = Genius::render_preview_errors(&errors);
        assert!(t.starts_with("[1] Uncaught error (×3) at src/W.tsx:42:13 on page /\n  TypeError: Cannot read"), "{t}");
        assert!(t.contains("    at f14 (") && !t.contains("at f15 ("), "first 15 stack lines: {t}");
        assert_eq!(t.matches("TypeError").count(), 1, "stack header line not repeated");
        assert!(t.contains("[2] Blank page on page /\n  Nothing rendered"));
        assert!(t.contains("[3] console.warn on page /"), "{t}");
    }

    #[tokio::test]
    async fn report_list_clear_and_events() {
        let d = tempfile::tempdir().unwrap();
        let rec = Arc::new(RecordingEmitter::default());
        let g = Genius::new(d.path(), rec.clone(), Arc::new(crate::secrets::MemoryStore::default())).unwrap();
        let p = g.create_project("Dash", "").unwrap();
        assert!(g.report_preview_errors("nope", vec![err("error", "x", None)]).is_err(), "unknown project");
        g.report_preview_errors(&p.id, vec![err("error", "boom", Some(1))]).unwrap();
        g.report_preview_errors(&p.id, vec![err("error", "boom", Some(1)), err("blank", "", None)]).unwrap();
        let list = g.preview_errors(&p.id);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].count, 2);
        assert_eq!(list[1].message, "The page rendered no visible content");
        g.report_preview_errors(&p.id, vec![PreviewRuntimeError { source: Some(format!("{}/src/App.tsx", p.path)), ..err("vite", "Transform failed", Some(3)) }]).unwrap();
        assert_eq!(g.preview_errors(&p.id)[2].source.as_deref(), Some("src/App.tsx"), "absolute project paths made relative");
        let preview_events: Vec<_> = g.bus.list(Some(&p.id), 50).unwrap().into_iter().filter(|e| e.kind == EventKind::Preview).collect();
        assert_eq!(preview_events.len(), 3, "one activity event per new distinct error");
        assert_eq!(preview_events[1].title, "Preview shows a blank page");
        let pushed = rec.on(channels::PREVIEW_ERRORS);
        assert_eq!(pushed.last().unwrap()["errors"].as_array().unwrap().len(), 3, "full current list pushed");
        g.clear_preview_errors(&p.id);
        assert!(g.preview_errors(&p.id).is_empty());
        assert_eq!(rec.on(channels::PREVIEW_ERRORS).last().unwrap()["errors"], json!([]));
        // Windows spellings: verbatim prefix, backslashes, Vite's mixed separators.
        let win = format!(r"\\?\{}", p.path.replace('/', "\\"));
        g.report_preview_errors(&p.id, vec![PreviewRuntimeError { source: Some(format!("{win}/src/Main.tsx")), stack: Some(format!("at App ({}\\src\\App.tsx:3:5)", p.path.replace('/', "\\"))), ..err("vite", "Other failure", Some(4)) }]).unwrap();
        let last = g.preview_errors(&p.id).pop().unwrap();
        assert_eq!(last.source.as_deref(), Some("src/Main.tsx"));
        assert!(last.stack.as_deref().unwrap().contains(r"(src\App.tsx:3:5)"), "{:?}", last.stack);
    }
}
