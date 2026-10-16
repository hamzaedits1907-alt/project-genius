//! Secret redaction for anything that is displayed, logged or persisted.
//!
//! Two layers: pattern-based (well-known token formats) and value-based
//! (exact secrets registered at runtime, e.g. the stored NVIDIA key).

use regex::Regex;
use std::sync::{LazyLock, RwLock};

static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"nvapi-[A-Za-z0-9_\-]{8,}",
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        r"github_pat_[A-Za-z0-9_]{20,}",
        r"sk-[A-Za-z0-9_\-]{20,}",
        r"AKIA[0-9A-Z]{16}",
        r"xox[baprs]-[A-Za-z0-9\-]{10,}",
        r"(?i)bearer\s+[A-Za-z0-9._\-~+/]{8,}=*",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        // credentials embedded in URLs: https://user:token@host
        r"(?i)(https?://)[^/\s:@]+:[^/\s@]+@",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("valid redaction regex"))
    .collect()
});

static KNOWN: LazyLock<RwLock<Vec<String>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Register an exact secret value so it is scrubbed even if it does not match a pattern.
pub fn register_secret(value: &str) {
    if value.len() < 6 {
        return;
    }
    let mut known = KNOWN.write().unwrap_or_else(|e| e.into_inner());
    if !known.iter().any(|k| k == value) {
        known.push(value.to_string());
    }
}

pub fn unregister_secret(value: &str) {
    let mut known = KNOWN.write().unwrap_or_else(|e| e.into_inner());
    known.retain(|k| k != value);
}

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    {
        let known = KNOWN.read().unwrap_or_else(|e| e.into_inner());
        for k in known.iter() {
            if out.contains(k.as_str()) {
                out = out.replace(k.as_str(), "[REDACTED]");
            }
        }
    }
    for (i, re) in PATTERNS.iter().enumerate() {
        if i == PATTERNS.len() - 1 {
            out = re.replace_all(&out, "${1}[REDACTED]@").into_owned();
        } else {
            out = re.replace_all(&out, "[REDACTED]").into_owned();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_known_formats() {
        let s = "key nvapi-abcdefghijklmnop123 and ghp_ABCDEFGHIJKLMNOPQRSTUV12 and Authorization: Bearer abc.def-ghi123";
        let r = redact(s);
        assert!(!r.contains("nvapi-abcdef"));
        assert!(!r.contains("ghp_ABC"));
        assert!(!r.contains("abc.def-ghi123"));
        assert!(r.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_url_credentials() {
        let r = redact("fatal: https://bob:s3cretT0ken@github.com/x/y.git not found");
        assert_eq!(r, "fatal: https://[REDACTED]@github.com/x/y.git not found");
    }

    #[test]
    fn redacts_registered_values() {
        register_secret("plain-secret-value-42");
        assert_eq!(redact("x plain-secret-value-42 y"), "x [REDACTED] y");
        unregister_secret("plain-secret-value-42");
        assert_eq!(redact("x plain-secret-value-42 y"), "x plain-secret-value-42 y");
    }

    #[test]
    fn leaves_normal_text() {
        let s = "src/App.tsx(12,4): error TS2304: Cannot find name 'foo'.";
        assert_eq!(redact(s), s);
    }
}
