//! Persistent log of UI crashes caught by the frontend error boundary.
//!
//! A crash in the webview (for example a React effect cleanup that is not a function) is
//! otherwise visible only in devtools, which ordinary users never open. The boundary sends the
//! message, JS stack and React component stack here; they are redacted and appended as JSON
//! lines to `<data dir>/logs/ui-errors.log` (rotated at 512 KB) so the failing component can be
//! identified after the fact, on any OS.

use crate::error::Result;
use crate::redact::redact;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 512 * 1024;
const MAX_FIELD: usize = 8 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UiErrorReport {
    pub view: String,
    pub message: String,
    #[serde(default)]
    pub stack: String,
    #[serde(default)]
    pub component_stack: String,
}

fn clip(s: &str) -> String {
    let r = redact(s);
    if r.len() <= MAX_FIELD {
        return r;
    }
    let mut end = MAX_FIELD;
    while !r.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated]", &r[..end])
}

pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("logs").join("ui-errors.log")
}

/// Append a redacted report; returns the log file path.
pub fn record(data_dir: &Path, report: &UiErrorReport, at: &str) -> Result<PathBuf> {
    let path = log_path(data_dir);
    std::fs::create_dir_all(path.parent().expect("log path has a parent"))?;
    if std::fs::metadata(&path).map(|m| m.len() > MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let line = serde_json::json!({
        "at": at,
        "view": clip(&report.view),
        "message": clip(&report.message),
        "stack": clip(&report.stack),
        "componentStack": clip(&report.component_stack),
    });
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(f, "{line}")?;
    Ok(path)
}

/// Most recent reports, newest first.
pub fn recent(data_dir: &Path, limit: usize) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(log_path(data_dir)) else { return vec![] };
    text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_redacted_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        crate::redact::register_secret("nvapi-supersecretvalue123456");
        let r = UiErrorReport { view: "project".into(), message: "destroy is not a function".into(), stack: "at X (key nvapi-supersecretvalue123456)".into(), component_stack: "x".repeat(20_000) };
        let p = record(dir.path(), &r, "2026-01-01T00:00:00Z").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("supersecretvalue"));
        assert!(text.contains("destroy is not a function"));
        let got = recent(dir.path(), 5);
        assert_eq!(got.len(), 1);
        assert!(got[0]["componentStack"].as_str().unwrap().len() < 9_000);
    }
}
