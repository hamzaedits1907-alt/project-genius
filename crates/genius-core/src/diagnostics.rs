//! Extract structured diagnostics from real build/test output (TypeScript, Vite/Rollup,
//! esbuild, Vitest). Used by the Build Doctor to pick the files and error lines that are
//! relevant, so the model gets focused context instead of the whole log.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub code: Option<String>,
    pub message: String,
    pub source: String,
}

static TSC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<f>[^\s(:][^(:]*?)\((?P<l>\d+),(?P<c>\d+)\):\s*error\s+(?P<code>TS\d+):\s*(?P<m>.+)$").unwrap());
static TSC_PRETTY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<f>[^\s:][^:]*?):(?P<l>\d+):(?P<c>\d+)\s+-\s+error\s+(?P<code>TS\d+):\s*(?P<m>.+)$").unwrap());
static ESBUILD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:✘\s*)?\[ERROR\]\s*(?P<m>.+)$").unwrap());
static LOC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?P<f>(?:\./)?(?:src|tests?|public)/[\w./\-\[\]@]+\.(?:tsx?|jsx?|css|json|html|mjs|cjs))(?::(?P<l>\d+)(?::(?P<c>\d+))?)?").unwrap());
static GENERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(error|.*\berror\b:|.*failed to resolve|.*could not resolve|.*is not exported|.*unexpected token|.*cannot find module|.*syntaxerror|.*referenceerror|.*typeerror|\s*FAIL\s|.*AssertionError|.*expected .* to)").unwrap()
});

pub fn parse(output: &str) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim_end();
        if let Some(c) = TSC.captures(line).or_else(|| TSC_PRETTY.captures(line)) {
            out.push(Diagnostic {
                file: Some(norm(&c["f"])),
                line: c["l"].parse().ok(),
                column: c["c"].parse().ok(),
                code: Some(c["code"].to_string()),
                message: c["m"].trim().to_string(),
                source: "typescript".into(),
            });
            continue;
        }
        if let Some(c) = ESBUILD.captures(line) {
            let loc = lines.iter().skip(i + 1).take(4).find_map(|l| LOC.captures(l));
            out.push(Diagnostic {
                file: loc.as_ref().map(|l| norm(&l["f"])),
                line: loc.as_ref().and_then(|l| l.name("l")).and_then(|m| m.as_str().parse().ok()),
                column: loc.as_ref().and_then(|l| l.name("c")).and_then(|m| m.as_str().parse().ok()),
                code: None,
                message: c["m"].trim().to_string(),
                source: "esbuild".into(),
            });
            continue;
        }
        if GENERIC.is_match(line) {
            let loc = LOC.captures(line).or_else(|| lines.iter().skip(i + 1).take(3).find_map(|l| LOC.captures(l)));
            out.push(Diagnostic {
                file: loc.as_ref().map(|l| norm(&l["f"])),
                line: loc.as_ref().and_then(|l| l.name("l")).and_then(|m| m.as_str().parse().ok()),
                column: loc.as_ref().and_then(|l| l.name("c")).and_then(|m| m.as_str().parse().ok()),
                code: None,
                message: line.trim().chars().take(400).collect(),
                source: "build".into(),
            });
        }
        if out.len() >= 40 {
            break;
        }
    }
    out.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    out
}

fn norm(f: &str) -> String {
    let f = f.trim().replace('\\', "/");
    f.trim_start_matches("./").to_string()
}

/// Files referenced by diagnostics (and stack traces), deduplicated, in order of appearance.
pub fn referenced_files(output: &str, diags: &[Diagnostic]) -> Vec<String> {
    let mut files: Vec<String> = diags.iter().filter_map(|d| d.file.clone()).collect();
    for c in LOC.captures_iter(output) {
        files.push(norm(&c["f"]));
    }
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| !f.contains("node_modules") && seen.insert(f.clone()));
    files.truncate(8);
    files
}

/// Keep the most informative part of a long log: first error region + tail, bounded.
pub fn focus_log(output: &str, max_chars: usize) -> String {
    if output.len() <= max_chars {
        return output.to_string();
    }
    let lines: Vec<&str> = output.lines().collect();
    let first_err = lines.iter().position(|l| GENERIC.is_match(l) || TSC.is_match(l) || ESBUILD.is_match(l)).unwrap_or(0);
    let start = first_err.saturating_sub(5);
    let mut head = String::new();
    for l in &lines[start..] {
        if head.len() + l.len() > max_chars / 2 {
            break;
        }
        head.push_str(l);
        head.push('\n');
    }
    let mut tail_start = output.len().saturating_sub(max_chars / 2);
    while !output.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!("{head}\n… [log shortened] …\n{}", &output[tail_start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tsc_errors() {
        let log =
            "> tsc --noEmit && vite build\n\nsrc/App.tsx(12,7): error TS2304: Cannot find name 'useStat'.\nsrc/components/List.tsx:4:10 - error TS2305: Module '\"./x\"' has no exported member 'Y'.\n";
        let d = parse(log);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].file.as_deref(), Some("src/App.tsx"));
        assert_eq!(d[0].line, Some(12));
        assert_eq!(d[0].code.as_deref(), Some("TS2304"));
        assert_eq!(d[1].file.as_deref(), Some("src/components/List.tsx"));
        assert_eq!(referenced_files(log, &d), vec!["src/App.tsx", "src/components/List.tsx"]);
    }

    #[test]
    fn parses_vite_resolution_error() {
        let log = "vite v7.1.0 building for production...\nerror during build:\n[vite]: Rollup failed to resolve import \"lodash\" from \"/home/u/p/src/utils/sort.ts\".\n";
        let d = parse(log);
        assert!(d.iter().any(|x| x.message.contains("failed to resolve")));
        assert!(referenced_files(log, &d).contains(&"src/utils/sort.ts".to_string()));
    }

    #[test]
    fn parses_esbuild_block() {
        let log = "✘ [ERROR] Expected \";\" but found \"}\"\n\n    src/main.tsx:8:2:\n      8 │   }\n";
        let d = parse(log);
        assert_eq!(d[0].source, "esbuild");
        assert_eq!(d[0].file.as_deref(), Some("src/main.tsx"));
        assert_eq!(d[0].line, Some(8));
    }

    #[test]
    fn focus_bounds_length() {
        let big = format!("{}\nsrc/a.ts(1,1): error TS1: bad\n{}", "noise\n".repeat(5000), "tail\n".repeat(5000));
        let f = focus_log(&big, 4000);
        assert!(f.len() < 4200);
        assert!(f.contains("error TS1"));
    }
}
