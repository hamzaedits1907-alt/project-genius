//! Heuristic secret detection for files about to be committed, published or sent to the model.
//! Findings never include the secret itself — only its location and type.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

use crate::workspace::{is_secret_name, Workspace};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SecretFinding {
    pub path: String,
    pub line: Option<usize>,
    pub kind: String,
}

static RULES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        ("NVIDIA API key", r"nvapi-[A-Za-z0-9_\-]{20,}"),
        ("GitHub token", r"gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,}"),
        ("OpenAI-style API key", r"\bsk-[A-Za-z0-9_\-]{24,}"),
        ("AWS access key", r"\bAKIA[0-9A-Z]{16}\b"),
        ("Slack token", r"xox[baprs]-[A-Za-z0-9\-]{10,}"),
        ("Private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        ("Google API key", r"\bAIza[0-9A-Za-z_\-]{35}\b"),
        ("Stripe secret key", r"\b(sk|rk)_live_[0-9A-Za-z]{20,}"),
        (
            "Hard-coded credential",
            r#"(?i)\b(api[_-]?key|secret|passw(or)?d|access[_-]?token|auth[_-]?token)\b\s*[:=]\s*["'][^"'\s]{12,}["']"#,
        ),
    ]
    .into_iter()
    .map(|(k, p)| (k, Regex::new(p).expect("valid secret regex")))
    .collect()
});

pub fn scan_text(path: &str, text: &str) -> Vec<SecretFinding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (kind, re) in RULES.iter() {
            if re.is_match(line) {
                out.push(SecretFinding { path: path.into(), line: Some(i + 1), kind: (*kind).into() });
                break;
            }
        }
        if out.len() > 50 {
            break;
        }
    }
    out
}

pub fn scan_path_name(path: &str) -> Option<SecretFinding> {
    let name = path.rsplit('/').next().unwrap_or(path);
    is_secret_name(name).then(|| SecretFinding { path: path.into(), line: None, kind: "Sensitive file name".into() })
}

/// Scan the given workspace files (names and contents).
pub fn scan_files(ws: &Workspace, paths: &[String]) -> Vec<SecretFinding> {
    let mut out = Vec::new();
    for p in paths {
        if let Some(f) = scan_path_name(p) {
            out.push(f);
        }
        if let Ok(text) = ws.read_text(p) {
            out.extend(scan_text(p, &text));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_secrets_without_leaking() {
        let f = scan_text("src/api.ts", "const k = 'nvapi-abcdefghijklmnopqrstuvwxyz012345';\nconst ok = 1;\nconst password = \"hunter2hunter2!!\";");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].line, Some(1));
        assert_eq!(f[0].kind, "NVIDIA API key");
        assert_eq!(f[1].kind, "Hard-coded credential");
        assert!(!serde_json::to_string(&f).unwrap().contains("abcdefghij"));
    }

    #[test]
    fn ignores_normal_code() {
        assert!(scan_text("a.ts", "const apiKey = import.meta.env.VITE_KEY;\nconst secret = '';").is_empty());
    }

    #[test]
    fn flags_sensitive_names() {
        assert!(scan_path_name("config/.env.production").is_some());
        assert!(scan_path_name("src/env.ts").is_none());
    }
}
