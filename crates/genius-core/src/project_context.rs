//! Project understanding: a compact, evidence-based summary of a project built from real
//! file reads (no model involved). The agent receives this summary instead of the whole
//! repository and retrieves individual files with tools when it needs them.
//!
//! Everything here is a *verified repository fact* (it was read from disk just now); the
//! summary says so, and lists what could not be determined rather than guessing.

use crate::error::Result;
use crate::workspace::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFacts {
    pub file_count: usize,
    pub frameworks: Vec<String>,
    pub dependencies: Vec<String>,
    pub dev_dependencies: Vec<String>,
    pub scripts: Vec<(String, String)>,
    pub entry_points: Vec<String>,
    pub routes: Vec<String>,
    pub components: Vec<String>,
    pub tests: Vec<String>,
    pub config_files: Vec<String>,
    /// Names only — contents of env files are never read into context.
    pub env_files: Vec<String>,
    pub tree: Vec<String>,
    pub unknown: Vec<String>,
}

const KNOWN_FRAMEWORKS: &[(&str, &str)] = &[
    ("react", "React"),
    ("vite", "Vite"),
    ("typescript", "TypeScript"),
    ("next", "Next.js"),
    ("vue", "Vue"),
    ("svelte", "Svelte"),
    ("@angular/core", "Angular"),
    ("react-router-dom", "React Router"),
    ("react-router", "React Router"),
    ("tailwindcss", "Tailwind CSS"),
    ("vitest", "Vitest"),
    ("jest", "Jest"),
    ("@supabase/supabase-js", "Supabase client"),
    ("express", "Express"),
    ("@tauri-apps/api", "Tauri"),
];

const CONFIG_NAMES: &[&str] = &[
    "package.json",
    "tsconfig.json",
    "vite.config.ts",
    "vite.config.js",
    "next.config.js",
    "tailwind.config.js",
    "tailwind.config.ts",
    "eslint.config.js",
    ".eslintrc.json",
    "index.html",
    "supabase/config.toml",
];

pub fn gather(ws: &Workspace) -> Result<ProjectFacts> {
    let files = ws.list_files()?;
    let mut f = ProjectFacts { file_count: files.len(), ..Default::default() };
    match ws.read_text_opt("package.json")?.and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
        Some(pkg) => {
            let names = |k: &str| -> Vec<String> { pkg.get(k).and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default() };
            f.dependencies = names("dependencies");
            f.dev_dependencies = names("devDependencies");
            if let Some(s) = pkg.get("scripts").and_then(|s| s.as_object()) {
                f.scripts = s.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect();
            }
            for (dep, name) in KNOWN_FRAMEWORKS {
                if f.dependencies.iter().chain(f.dev_dependencies.iter()).any(|d| d == dep) && !f.frameworks.iter().any(|x| x == name) {
                    f.frameworks.push((*name).to_string());
                }
            }
        }
        None => f.unknown.push("package.json missing or not valid JSON: dependencies and scripts unknown".into()),
    }
    for e in &files {
        let p = e.path.as_str();
        let name = p.rsplit('/').next().unwrap_or(p);
        if CONFIG_NAMES.contains(&p) {
            f.config_files.push(p.into());
        }
        if name.starts_with(".env") {
            f.env_files.push(p.into());
            continue;
        }
        if matches!(p, "src/main.tsx" | "src/main.ts" | "src/main.jsx" | "src/index.tsx" | "src/index.js" | "src/App.tsx" | "src/App.jsx" | "index.html" | "app/page.tsx" | "pages/index.tsx") {
            f.entry_points.push(p.into());
        }
        if name.contains(".test.") || name.contains(".spec.") || p.starts_with("tests/") {
            f.tests.push(p.into());
        } else if (p.starts_with("src/components/") || p.starts_with("src/pages/") || p.starts_with("src/views/"))
            && (p.ends_with(".tsx") || p.ends_with(".jsx") || p.ends_with(".vue") || p.ends_with(".svelte"))
        {
            f.components.push(p.into());
        }
        if e.is_text && !e.sensitive && (p.ends_with(".tsx") || p.ends_with(".jsx")) && e.size < 200_000 {
            if let Ok(text) = ws.read_text(p) {
                for cap in route_regex().captures_iter(&text) {
                    let r = format!("{} ({p})", &cap[1]);
                    if !f.routes.contains(&r) && f.routes.len() < 50 {
                        f.routes.push(r);
                    }
                }
            }
        }
    }
    if f.routes.is_empty() {
        f.unknown.push("no router routes detected (single-page app or an unrecognised router)".into());
    }
    f.tree = files.iter().filter(|e| !e.sensitive || e.path.rsplit('/').next().is_some_and(|n| n.starts_with(".env"))).map(|e| e.path.clone()).take(300).collect();
    Ok(f)
}

fn route_regex() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r#"(?:<Route[^>]*\spath=|path:\s*)["']([^"']+)["']"#).expect("valid regex"))
}

/// Render facts as compact text for the model.
pub fn render(f: &ProjectFacts) -> String {
    let list = |v: &[String]| if v.is_empty() { "none found".to_string() } else { v.join(", ") };
    let mut s = String::from("PROJECT FACTS (read from disk just now; verified):\n");
    s.push_str(&format!("- files: {}\n", f.file_count));
    s.push_str(&format!("- frameworks/tools: {}\n", list(&f.frameworks)));
    s.push_str(&format!("- dependencies: {}\n", list(&f.dependencies)));
    s.push_str(&format!("- devDependencies: {}\n", list(&f.dev_dependencies)));
    let scripts: Vec<String> = f.scripts.iter().map(|(k, v)| format!("{k}=`{v}`")).collect();
    s.push_str(&format!("- npm scripts: {}\n", list(&scripts)));
    s.push_str(&format!("- entry points: {}\n", list(&f.entry_points)));
    s.push_str(&format!("- routes: {}\n", list(&f.routes)));
    s.push_str(&format!("- components: {}\n", list(&f.components)));
    s.push_str(&format!("- tests: {}\n", list(&f.tests)));
    s.push_str(&format!("- config files: {}\n", list(&f.config_files)));
    s.push_str(&format!("- env files (names only, contents never shared): {}\n", list(&f.env_files)));
    if !f.unknown.is_empty() {
        s.push_str(&format!("- could not determine: {}\n", f.unknown.join("; ")));
    }
    s.push_str("FILE TREE:\n");
    for p in &f.tree {
        s.push_str(p);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gathers_real_facts_and_never_reads_env_contents() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        std::fs::create_dir_all(dir.path().join("src/components")).unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"scripts":{"build":"vite build"},"dependencies":{"react":"19","react-router-dom":"7"},"devDependencies":{"vitest":"3"}}"#).unwrap();
        std::fs::write(dir.path().join("src/App.tsx"), r#"<Routes><Route path="/" element={<Home/>}/><Route path="/settings" element={<S/>}/></Routes>"#).unwrap();
        std::fs::write(dir.path().join("src/components/Sidebar.tsx"), "export {}").unwrap();
        std::fs::write(dir.path().join("src/App.test.tsx"), "test").unwrap();
        std::fs::write(dir.path().join(".env.local"), "SECRET=abc").unwrap();
        let f = gather(&ws).unwrap();
        assert_eq!(f.frameworks, vec!["React", "React Router", "Vitest"]);
        assert_eq!(f.scripts, vec![("build".to_string(), "vite build".to_string())]);
        assert_eq!(f.routes, vec!["/ (src/App.tsx)", "/settings (src/App.tsx)"]);
        assert_eq!(f.components, vec!["src/components/Sidebar.tsx"]);
        assert_eq!(f.tests, vec!["src/App.test.tsx"]);
        assert_eq!(f.env_files, vec![".env.local"]);
        let text = render(&f);
        assert!(text.contains(".env.local"));
        assert!(!text.contains("SECRET=abc"));
    }

    #[test]
    fn reports_unknowns_instead_of_guessing() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        let f = gather(&ws).unwrap();
        assert!(f.unknown.iter().any(|u| u.contains("package.json")));
        assert!(render(&f).contains("could not determine"));
    }
}
