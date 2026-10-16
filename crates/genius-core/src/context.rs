//! Context selection: decide which project files are sent to the model for a request.
//!
//! The model never receives the whole repository. Files are ranked by explicit user
//! selection, references in the request/diagnostics, keyword overlap, and structural
//! importance, then packed into a character budget. Sensitive files (secrets, lockfiles,
//! binaries, large files) and user-excluded paths are never included. The selection is
//! returned to the UI *before* sending so the user can see and adjust what is shared.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::error::Result;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextFile {
    pub path: String,
    pub chars: usize,
    pub reason: String,
    pub included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPlan {
    pub files: Vec<ContextFile>,
    pub tree: Vec<String>,
    pub budget_chars: usize,
    pub used_chars: usize,
    pub excluded_sensitive: Vec<String>,
}

pub struct ContextRequest<'a> {
    pub query: &'a str,
    pub pinned: &'a [String],
    pub excluded: &'a [String],
    pub must: &'a [String],
    pub budget_chars: usize,
}

const ALWAYS: &[(&str, i32)] = &[("package.json", 40), ("src/App.tsx", 60), ("src/main.tsx", 25), ("src/index.css", 15), ("src/App.css", 15)];

fn words(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect()
}

fn is_excluded(path: &str, excluded: &[String]) -> bool {
    excluded.iter().any(|e| {
        let e = e.trim().trim_end_matches('/');
        !e.is_empty() && (path == e || path.starts_with(&format!("{e}/")) || (e.starts_with("*.") && path.ends_with(&e[1..])))
    })
}

pub fn plan(ws: &Workspace, req: &ContextRequest) -> Result<ContextPlan> {
    let entries = ws.list_files()?;
    let qwords = words(req.query);
    let qlower = req.query.to_lowercase();
    let mut tree = Vec::new();
    let mut excluded_sensitive = Vec::new();
    let mut scored: Vec<(i32, String, String, usize, String)> = Vec::new(); // score, path, reason, chars, content
    let always: HashMap<&str, i32> = ALWAYS.iter().cloned().collect();

    for e in &entries {
        tree.push(e.path.clone());
        if e.sensitive {
            excluded_sensitive.push(e.path.clone());
            continue;
        }
        if is_excluded(&e.path, req.excluded) {
            continue;
        }
        let Ok(content) = ws.read_text(&e.path) else { continue };
        let mut score = 0;
        let mut reasons = Vec::new();
        if req.must.contains(&e.path) {
            score += 1000;
            reasons.push("referenced by diagnostics");
        }
        if req.pinned.contains(&e.path) {
            score += 500;
            reasons.push("selected by you");
        }
        let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_lowercase();
        let stem = name.split('.').next().unwrap_or("").to_string();
        if stem.len() >= 3 && qlower.contains(&stem) {
            score += 120;
            reasons.push("named in request");
        }
        if let Some(s) = always.get(e.path.as_str()) {
            score += s;
            reasons.push("project entry point");
        }
        let cw = words(&content);
        let overlap = qwords.iter().filter(|w| cw.contains(*w)).count() as i32;
        let pw = words(&e.path);
        let path_overlap = qwords.iter().filter(|w| pw.contains(*w)).count() as i32;
        if overlap + path_overlap > 0 {
            score += overlap * 6 + path_overlap * 30;
            reasons.push("matches request keywords");
        }
        if e.path.starts_with("src/") {
            score += 5;
        }
        if e.path.contains(".test.") || e.path.contains(".spec.") {
            score -= 10;
        }
        let reason = if reasons.is_empty() { "project source".to_string() } else { reasons.join(", ") };
        scored.push((score, e.path.clone(), reason, content.len(), content));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut used = 0;
    let mut files = Vec::new();
    for (score, path, reason, chars, _) in scored {
        let forced = score >= 500;
        let include = (forced || score > 0) && (used + chars <= req.budget_chars || (forced && used + chars <= req.budget_chars * 2));
        if include {
            used += chars;
        }
        files.push(ContextFile { path, chars, reason, included: include });
    }
    Ok(ContextPlan { files, tree, budget_chars: req.budget_chars, used_chars: used, excluded_sensitive })
}

/// Render the included files for the prompt.
pub fn render(ws: &Workspace, plan: &ContextPlan) -> String {
    let mut s = String::new();
    s.push_str("PROJECT FILE TREE:\n");
    for p in &plan.tree {
        s.push_str("  ");
        s.push_str(p);
        s.push('\n');
    }
    s.push_str("\nRELEVANT FILES (full current contents):\n");
    for f in plan.files.iter().filter(|f| f.included) {
        if let Ok(c) = ws.read_text(&f.path) {
            s.push_str(&format!("\n<<<CURRENT: {}>>>\n{}", f.path, c));
            if !c.ends_with('\n') {
                s.push('\n');
            }
            s.push_str("<<<END CURRENT>>>\n");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        let w = Workspace::new(d.path().join("p")).unwrap();
        w.write_text("package.json", "{}").unwrap();
        w.write_text("src/App.tsx", "import Sidebar from './components/Sidebar';").unwrap();
        w.write_text("src/components/Sidebar.tsx", "export default function Sidebar() { return <nav>menu</nav> }").unwrap();
        w.write_text("src/components/Footer.tsx", "export default function Footer() { return <footer/> }").unwrap();
        w.write_text("src/big.ts", &"x".repeat(5000)).unwrap();
        std::fs::write(w.root().join(".env"), "SECRET=1").unwrap();
        (d, w)
    }

    #[test]
    fn ranks_relevant_files_and_excludes_secrets() {
        let (_d, w) = ws();
        let p = plan(&w, &ContextRequest { query: "make the sidebar menu collapsible", pinned: &[], excluded: &[], must: &[], budget_chars: 2000 }).unwrap();
        let inc: Vec<&str> = p.files.iter().filter(|f| f.included).map(|f| f.path.as_str()).collect();
        assert_eq!(inc[0], "src/components/Sidebar.tsx");
        assert!(inc.contains(&"src/App.tsx"));
        assert!(!inc.contains(&"src/big.ts"), "over budget");
        assert_eq!(p.excluded_sensitive, vec![".env"]);
        assert!(!render(&w, &p).contains("SECRET=1"));
    }

    #[test]
    fn user_exclusions_and_pins() {
        let (_d, w) = ws();
        let p = plan(&w, &ContextRequest {
            query: "change footer",
            pinned: &["src/big.ts".into()],
            excluded: &["src/components".into()],
            must: &[],
            budget_chars: 20000,
        })
        .unwrap();
        assert!(p.files.iter().all(|f| !f.path.starts_with("src/components")));
        assert!(p.files.iter().any(|f| f.path == "src/big.ts" && f.included && f.reason.contains("selected")));
    }
}
