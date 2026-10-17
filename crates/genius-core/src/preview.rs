//! Live-preview state and readiness verification.
//!
//! A preview is *ready* only when all of these were observed for real:
//!
//! 1. the dev server process is running and accepts TCP connections on its port,
//! 2. `GET /` answers 200 with HTML **and carries this run's identity token** (the Genius
//!    wrapper config adds an `x-genius-preview` header with a per-run random value), so a
//!    different program — or another project's server — on that port is never mistaken for it,
//! 3. the application's module graph compiles: every local module reachable from the HTML entry
//!    scripts is requested and must transform without error (Vite answers 500 with the error,
//!    e.g. `Failed to resolve import "uuid"`).
//!
//! Type errors are not part of this check (Vite does not type-check in development); they are
//! reported by the separate production build, which is never conflated with the preview.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, VecDeque};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

pub const TOKEN_HEADER: &str = "x-genius-preview";
pub const TOKEN_ENV: &str = "GENIUS_PREVIEW_TOKEN";
const MAX_MODULES: usize = 400;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStatus {
    #[default]
    Idle,
    Checking,
    Installing,
    Starting,
    Compiling,
    Ready,
    Failed,
    Stopping,
}

impl PreviewStatus {
    pub fn busy(&self) -> bool {
        matches!(self, PreviewStatus::Checking | PreviewStatus::Installing | PreviewStatus::Starting | PreviewStatus::Compiling | PreviewStatus::Stopping)
    }
}

/// A failure in plain language plus the technical evidence behind it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreviewProblem {
    /// "config" | "install" | "start" | "compile" | "identity" | "runtime"
    pub stage: String,
    /// One sentence for a beginner.
    pub summary: String,
    /// Exact error text (compiler message, process output tail).
    pub details: String,
    pub file: Option<String>,
    /// The npm run that produced the evidence, if any.
    pub run_id: Option<String>,
    /// Whether "Fix automatically" can work on it (false e.g. when npm is not installed).
    pub fixable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreviewState {
    pub project_id: String,
    pub status: PreviewStatus,
    /// What is happening now, in plain language.
    pub message: String,
    pub url: Option<String>,
    pub port: Option<u16>,
    /// The dev-server run this state belongs to. The UI keys the iframe on it.
    pub run_id: Option<String>,
    pub problem: Option<PreviewProblem>,
    /// Number of modules verified by the last compile check.
    pub modules_checked: usize,
    /// Increases on every start/restart so late events of an old run are ignored.
    pub generation: u64,
    pub inspector: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CheckFailure {
    /// Nothing usable answered (yet).
    NotReady(String),
    /// Something answered, but not this run's server.
    WrongServer(String),
    /// The server is ours but the app does not compile.
    Compile { file: Option<String>, message: String, details: String },
}

static SCRIPT_SRC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<script\b[^>]*\btype\s*=\s*["']module["'][^>]*\bsrc\s*=\s*["']([^"']+)["']|<script\b[^>]*\bsrc\s*=\s*["']([^"']+)["'][^>]*\btype\s*=\s*["']module["']"#).unwrap());
static JS_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?:\bfrom\s*|\bimport\s*\(?\s*)["'](/[^"'\s]+)["']"#).unwrap());
static VITE_ERROR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)const error = (\{.*?\})\s*\n").unwrap());

/// Local module URLs worth compiling: project sources, not Vite internals or pre-bundled deps.
fn crawlable(url: &str) -> bool {
    !(url.starts_with("/@") || url.contains("/node_modules/") || url.starts_with("/__") || url.starts_with("//"))
}

pub fn entry_scripts(html: &str) -> Vec<String> {
    SCRIPT_SRC.captures_iter(html).filter_map(|c| c.get(1).or_else(|| c.get(2)).map(|m| m.as_str().to_string())).filter(|s| s.starts_with('/') && crawlable(s)).collect()
}

pub fn module_imports(js: &str) -> Vec<String> {
    JS_IMPORT.captures_iter(js).filter_map(|c| c.get(1).map(|m| m.as_str().to_string())).filter(|s| crawlable(s)).collect()
}

/// Extract the structured error Vite embeds in its 500 page.
pub fn parse_vite_error(body: &str, project_root: Option<&str>) -> (Option<String>, String, String) {
    let strip = |s: &str| match project_root {
        Some(r) if !r.is_empty() => s.replace(&format!("{r}/"), "").replace(&format!("{}\\", r), "").replace(r, ""),
        _ => s.to_string(),
    };
    if let Some(c) = VITE_ERROR.captures(body) {
        if let Ok(v) = serde_json::from_str::<Value>(&c[1]) {
            let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("compile error").to_string();
            let file = v.pointer("/loc/file").or_else(|| v.get("id")).and_then(|f| f.as_str()).map(|f| strip(f).replace('\\', "/"));
            let frame = v.get("frame").and_then(|f| f.as_str()).unwrap_or("");
            let plugin = v.get("plugin").and_then(|f| f.as_str()).unwrap_or("");
            let details = format!("{}{}{}", strip(&message), if plugin.is_empty() { String::new() } else { format!("\nPlugin: {plugin}") }, if frame.is_empty() { String::new() } else { format!("\n{frame}") });
            return (file, strip(&message), details);
        }
    }
    let text: String = body.chars().take(2000).collect();
    (None, "the development server could not compile a module".into(), strip(&text))
}

pub fn http_client() -> reqwest::Client {
    // Never route loopback checks through a proxy configured in the environment.
    reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(30)).build().unwrap_or_default()
}

/// Verify that the server on `port` is this run's server and that the app compiles.
/// Returns the number of modules compiled.
pub async fn check_server(client: &reqwest::Client, port: u16, token: Option<&str>, project_root: Option<&str>, budget: Duration) -> std::result::Result<usize, CheckFailure> {
    let base = format!("http://127.0.0.1:{port}");
    let res = client.get(format!("{base}/")).header("accept", "text/html").send().await.map_err(|e| CheckFailure::NotReady(format!("no HTTP answer yet: {e}")))?;
    if let Some(t) = token {
        let got = res.headers().get(TOKEN_HEADER).and_then(|v| v.to_str().ok()).unwrap_or("");
        if got != t {
            return Err(CheckFailure::WrongServer(format!("the program answering on port {port} is not this project's preview server")));
        }
    }
    let status = res.status();
    let html = res.text().await.map_err(|e| CheckFailure::NotReady(e.to_string()))?;
    if status.as_u16() == 500 {
        let (file, message, details) = parse_vite_error(&html, project_root);
        return Err(CheckFailure::Compile { file, message, details });
    }
    if !status.is_success() {
        return Err(CheckFailure::Compile { file: Some("index.html".into()), message: format!("the start page answered HTTP {status}"), details: html.chars().take(2000).collect() });
    }
    let mut queue: VecDeque<String> = entry_scripts(&html).into();
    if queue.is_empty() && !html.to_lowercase().contains("<html") && !html.to_lowercase().contains("<!doctype") {
        return Err(CheckFailure::Compile { file: Some("index.html".into()), message: "the start page is not an HTML document".into(), details: html.chars().take(500).collect() });
    }
    let mut seen: BTreeSet<String> = queue.iter().cloned().collect();
    let deadline = Instant::now() + budget;
    let mut compiled = 0;
    while let Some(url) = queue.pop_front() {
        if compiled >= MAX_MODULES || Instant::now() > deadline {
            break;
        }
        let r = client.get(format!("{base}{url}")).send().await.map_err(|e| CheckFailure::NotReady(e.to_string()))?;
        let st = r.status();
        let body = r.text().await.unwrap_or_default();
        if st.as_u16() == 504 {
            // "Outdated Optimize Dep": Vite is re-optimizing dependencies; try again shortly.
            return Err(CheckFailure::NotReady("the development server is optimizing dependencies".into()));
        }
        if !st.is_success() {
            let (file, message, details) = parse_vite_error(&body, project_root);
            let file = file.or_else(|| Some(url.trim_start_matches('/').split('?').next().unwrap_or("").to_string()));
            return Err(CheckFailure::Compile { file, message, details });
        }
        compiled += 1;
        for m in module_imports(&body) {
            if seen.insert(m.clone()) {
                queue.push_back(m);
            }
        }
    }
    Ok(compiled)
}

/// Lines in dev-server output that mean the running app is currently broken.
pub fn is_error_line(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("internal server error") || l.contains("pre-transform error") || l.contains("failed to resolve import") || l.contains("[plugin:vite:")
}

/// Lines that mean Vite picked up a file change (a good moment to re-verify).
pub fn is_update_line(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    l.contains("hmr update") || l.contains("page reload") || l.contains("server restarted")
}

pub fn new_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_entry_scripts_and_imports() {
        let html = r#"<script type="module" src="/@vite/client"></script><body><script type="module" src="/src/main.tsx"></script><script src="/legacy.js" type="module"></script>"#;
        assert_eq!(entry_scripts(html), vec!["/src/main.tsx", "/legacy.js"]);
        let js = "import x from \"/node_modules/.vite/deps/react.js?v=1\";\nimport App from \"/src/App.tsx\";\nimport \"/src/index.css\";\nconst L = () => import(\"/src/Lazy.tsx\");\nimport * as R from \"/@react-refresh\";";
        assert_eq!(module_imports(js), vec!["/src/App.tsx", "/src/index.css", "/src/Lazy.tsx"]);
    }

    #[test]
    fn parses_the_real_vite_error_page() {
        // Shape copied from a real Vite 7 response for a missing package.
        let body = "<script type=\"module\">\n              const error = {\"message\":\"Failed to resolve import \\\"uuid\\\" from \\\"src/App.tsx\\\". Does the file exist?\",\"id\":\"/home/me/app/src/App.tsx\",\"frame\":\"2  |  import { v4 } from \\\"uuid\\\";\",\"plugin\":\"vite:import-analysis\",\"loc\":{\"file\":\"/home/me/app/src/App.tsx\",\"line\":1,\"column\":19}}\n              try {";
        let (file, msg, details) = parse_vite_error(body, Some("/home/me/app"));
        assert_eq!(file.as_deref(), Some("src/App.tsx"));
        assert!(msg.contains("Failed to resolve import \"uuid\""), "{msg}");
        assert!(details.contains("vite:import-analysis") && details.contains("import { v4 }"));
        assert!(!details.contains("/home/me"), "absolute paths are shortened");
    }

    #[test]
    fn classifies_output_lines() {
        assert!(is_error_line("10:57:38 PM [vite] Internal server error: Failed to resolve import \"uuid\""));
        assert!(is_error_line("[vite] (client) Pre-transform error: Transform failed"));
        assert!(!is_error_line("  VITE v7.3.7  ready in 298 ms"));
        assert!(is_update_line("10:58:01 PM [vite] (client) hmr update /src/App.tsx"));
    }
}
