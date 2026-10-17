//! Validation of an AI-proposed changeset *before* it is stored as a proposal or written.
//!
//! The checks run against the project as it would look after the change (current files plus
//! the proposed writes minus the proposed deletes), so they catch proposals that would leave
//! the project unable to install, compile or start:
//!
//! * `package.json`: strict JSON (no duplicate keys), an object, string versions, registry
//!   sources only, no install-time lifecycle scripts, existing scripts kept, and no package
//!   removed while source code still imports it.
//! * Other `.json` files (except JSON-with-comments configs such as tsconfig): strict JSON.
//! * Source files: every bare import must be a declared dependency (or a Node built-in), and
//!   every relative import must resolve to a file that exists after the change.
//! * Abbreviated content ("// ... rest of the code unchanged") and leaked protocol markers.
//!
//! Blockers stop the proposal; warnings are shown to the user next to the diff.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use crate::changes::FileChange;
use crate::deps::{declared, parse_strict};
use crate::error::Result;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProposalCheck {
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    /// Packages added to package.json by this proposal.
    pub added_packages: Vec<String>,
    /// Whether the proposal changes what `npm install` would install.
    pub dependencies_changed: bool,
}

impl ProposalCheck {
    pub fn ok(&self) -> bool {
        self.blockers.is_empty()
    }
}

const SOURCE_EXT: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];
const RESOLVE_EXT: &[&str] = &["", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".mts", ".json", "/index.ts", "/index.tsx", "/index.js", "/index.jsx"];
const LIFECYCLE: &[&str] = &["preinstall", "install", "postinstall", "prepare", "prepublish"];
const NODE_BUILTINS: &[&str] = &[
    "assert", "buffer", "child_process", "crypto", "events", "fs", "http", "https", "module", "net", "os", "path", "process", "querystring", "readline", "stream", "string_decoder", "timers", "tty", "url", "util", "vm", "worker_threads", "zlib",
];

static IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)(?:^|[;\s{}])(?:import|export)\s+(?:type\s+)?(?:[\w*{}\s,$]+?\s+from\s+)?['"]([^'"\n]+)['"]|\bimport\s*\(\s*['"]([^'"\n]+)['"]\s*\)|\brequire\s*\(\s*['"]([^'"\n]+)['"]\s*\)"#).unwrap()
});
static ABBREVIATED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^\s*(?://|/\*|\{/\*|#|<!--)\s*(?:\.{3}|…)\s*(?:the\s+)?(?:rest|remaining|existing|other|previous|unchanged|same)\b").unwrap()
});

/// Module specifiers imported by a source file (comments are not stripped; false positives
/// are limited to string literals that look exactly like import statements).
pub fn imports(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    for c in IMPORT_RE.captures_iter(source) {
        if let Some(m) = c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)) {
            out.push(m.as_str().trim().to_string());
        }
    }
    out
}

/// npm package name for a bare specifier (`@scope/pkg/sub` → `@scope/pkg`, `pkg/sub` → `pkg`).
/// Returns `None` for relative paths, URLs, aliases and virtual modules.
pub fn package_of(spec: &str) -> Option<String> {
    if spec.starts_with('.') || spec.starts_with('/') || spec.contains(':') || spec.starts_with('#') || spec.starts_with("~/") || spec.starts_with("@/") || spec.starts_with('~') {
        return None;
    }
    let mut parts = spec.split('/');
    let first = parts.next()?;
    let name = if first.starts_with('@') { format!("{first}/{}", parts.next()?) } else { first.to_string() };
    // strip query suffixes used by Vite (`pkg?raw`)
    Some(name.split('?').next().unwrap_or(&name).to_string())
}

fn is_builtin(pkg: &str) -> bool {
    NODE_BUILTINS.contains(&pkg)
}

fn ext(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, e)| e).unwrap_or("")
}

fn is_source(path: &str) -> bool {
    SOURCE_EXT.contains(&ext(path)) && !path.ends_with(".d.ts")
}

fn join_rel(from_file: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = from_file.split('/').collect();
    parts.pop();
    for seg in spec.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Validate a proposed set of changes against the workspace.
pub fn check(ws: &Workspace, changes: &[FileChange]) -> Result<ProposalCheck> {
    let mut out = ProposalCheck::default();
    // State of every relevant file after the change.
    let mut after: BTreeMap<String, Option<String>> = BTreeMap::new();
    for c in changes {
        after.insert(c.path.clone(), c.after.clone());
    }
    let existing: Vec<String> = ws.list_files()?.into_iter().map(|f| f.path).collect();
    let mut all_paths: BTreeSet<String> = existing.iter().cloned().collect();
    for (p, a) in &after {
        if a.is_some() {
            all_paths.insert(p.clone());
        } else {
            all_paths.remove(p);
        }
    }
    let content_after = |p: &str| -> Option<String> {
        match after.get(p) {
            Some(a) => a.clone(),
            None => ws.read_text_opt(p).ok().flatten(),
        }
    };

    // ---- package.json
    let pkg_before: Option<Value> = ws.read_text_opt("package.json").ok().flatten().and_then(|t| parse_strict(&t).ok());
    let mut pkg_after: Option<Value> = pkg_before.clone();
    if let Some(change) = changes.iter().find(|c| c.path == "package.json") {
        match &change.after {
            None => out.blockers.push("The proposal deletes package.json; the project could no longer install or run.".into()),
            Some(text) => match package_json_problems(text, pkg_before.as_ref()) {
                Ok((v, warnings)) => {
                    out.warnings.extend(warnings);
                    let before_decl = pkg_before.as_ref().map(declared).unwrap_or_default();
                    let after_decl = declared(&v);
                    out.added_packages = after_decl.keys().filter(|k| !before_decl.contains_key(*k)).cloned().collect();
                    out.dependencies_changed = before_decl != after_decl
                        || ["overrides", "resolutions", "peerDependencies"].iter().any(|k| pkg_before.as_ref().and_then(|b| b.get(*k)) != v.get(*k));
                    pkg_after = Some(v);
                }
                Err(problems) => out.blockers.extend(problems),
            },
        }
    }
    let deps_after: BTreeMap<String, String> = pkg_after.as_ref().map(declared).unwrap_or_default();

    // ---- other JSON files (strict); JSON-with-comments configs are skipped.
    for c in changes {
        let name = c.path.rsplit('/').next().unwrap_or(&c.path);
        if c.path == "package.json" || ext(&c.path) != "json" || name.starts_with("tsconfig") || name.starts_with("jsconfig") || name.contains(".code-") || c.path.starts_with(".vscode/") {
            continue;
        }
        if let Some(text) = &c.after {
            if let Err(e) = parse_strict(text) {
                out.blockers.push(format!("{} is not valid JSON: {e}.", c.path));
            }
        }
    }

    // ---- content sanity of every written file
    for c in changes {
        let Some(text) = &c.after else { continue };
        if text.contains("<<<FILE:") || text.contains("<<<END FILE>>>") {
            out.blockers.push(format!("{} contains Genius file-protocol markers; the model output was malformed.", c.path));
        }
        if c.before.is_some() && ABBREVIATED_RE.is_match(text) {
            out.blockers.push(format!("{} looks abbreviated (it contains a comment like \"... rest of the code unchanged\"); writing it would delete code.", c.path));
        }
        if let Some(before) = &c.before {
            let (b, a) = (before.lines().count(), text.lines().count());
            if b >= 60 && a * 10 < b * 3 {
                out.warnings.push(format!("{} shrinks from {b} to {a} lines; check that nothing was removed by mistake.", c.path));
            }
        }
    }

    // ---- imports of changed files, and of unchanged files that import a deleted/changed file
    let deleted: BTreeSet<&str> = after.iter().filter(|(_, a)| a.is_none()).map(|(p, _)| p.as_str()).collect();
    let mut to_check: BTreeSet<String> = after.iter().filter(|(p, a)| a.is_some() && is_source(p)).map(|(p, _)| p.clone()).collect();
    if !deleted.is_empty() || out.dependencies_changed {
        to_check.extend(existing.iter().filter(|p| is_source(p) && !after.contains_key(*p) && (p.starts_with("src/") || !p.contains('/'))).cloned());
    }
    let mut missing_pkgs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in &to_check {
        let Some(text) = content_after(file) else { continue };
        for spec in imports(&text) {
            if spec.starts_with('.') {
                let Some(target) = join_rel(file, spec.split('?').next().unwrap_or(&spec)) else {
                    out.blockers.push(format!("{file} imports '{spec}', which points outside the project."));
                    continue;
                };
                // CSS/asset imports must exist exactly; code imports may omit the extension.
                let found = RESOLVE_EXT.iter().any(|e| all_paths.contains(&format!("{target}{e}")));
                if !found {
                    let why = if deleted.iter().any(|d| d.starts_with(&target)) { "which this change deletes" } else { "but that file does not exist and is not part of the change" };
                    out.blockers.push(format!("{file} imports '{spec}', {why}."));
                }
            } else if let Some(pkg) = package_of(&spec) {
                if is_builtin(&pkg) || pkg.starts_with("node:") || deps_after.contains_key(&pkg) {
                    continue;
                }
                // `@types/x` packages are type-only, but `x` itself must still be installed.
                missing_pkgs.entry(pkg).or_default().push(file.clone());
            }
        }
    }
    for (pkg, files) in missing_pkgs {
        let files: BTreeSet<String> = files.into_iter().collect();
        out.blockers.push(format!(
            "{} import{} '{pkg}', which is not listed in package.json. Add it to \"dependencies\" in the same change.",
            files.iter().cloned().collect::<Vec<_>>().join(", "),
            if files.len() == 1 { "s" } else { "" }
        ));
    }
    out.blockers.dedup();
    Ok(out)
}

/// Problems with a proposed package.json (each a user-readable sentence), or the parsed value
/// plus warnings when it is acceptable.
pub fn package_json_problems(text: &str, before: Option<&Value>) -> std::result::Result<(Value, Vec<String>), Vec<String>> {
    let v = parse_strict(text).map_err(|e| vec![format!("package.json is not valid JSON: {e}.")])?;
    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    if !v.is_object() {
        return Err(vec!["package.json must be a JSON object.".into()]);
    }
    for section in ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"] {
        let Some(sec) = v.get(section) else { continue };
        let Some(obj) = sec.as_object() else {
            problems.push(format!("\"{section}\" in package.json must be an object."));
            continue;
        };
        for (name, ver) in obj {
            if !crate::spec::valid_npm_name(name) {
                problems.push(format!("\"{name}\" in {section} is not a valid npm package name."));
            }
            let Some(ver) = ver.as_str() else {
                problems.push(format!("the version of \"{name}\" in {section} must be a string."));
                continue;
            };
            let ver = ver.trim();
            if ver.is_empty() {
                problems.push(format!("\"{name}\" in {section} has an empty version."));
            } else if ver.starts_with("file:") || ver.starts_with("link:") || ver.contains("://") || ver.starts_with("git") || ver.starts_with("workspace:") {
                problems.push(format!("\"{name}\" uses a non-registry source ({ver}); add such dependencies manually."));
            }
        }
    }
    let both: Vec<String> = v
        .get("dependencies")
        .and_then(|d| d.as_object())
        .map(|d| d.keys().filter(|k| v.get("devDependencies").and_then(|x| x.get(k.as_str())).is_some()).cloned().collect())
        .unwrap_or_default();
    if !both.is_empty() {
        warnings.push(format!("listed in both dependencies and devDependencies: {}", both.join(", ")));
    }
    match v.get("scripts") {
        None => {}
        Some(s) if !s.as_object().is_some_and(|o| o.values().all(|x| x.is_string())) => problems.push("\"scripts\" in package.json must map names to command strings.".into()),
        Some(s) => {
            for hook in LIFECYCLE {
                if s.get(*hook).is_some() {
                    problems.push(format!("the \"{hook}\" lifecycle script is not allowed in an AI-written package.json."));
                }
            }
        }
    }
    if let Some(before_scripts) = before.and_then(|b| b.get("scripts")).and_then(|s| s.as_object()) {
        for (name, cmd) in before_scripts {
            match v.pointer(&format!("/scripts/{name}")) {
                None if ["dev", "build"].contains(&name.as_str()) => problems.push(format!("the \"{name}\" script was removed; Genius needs it to run and build the project.")),
                None => warnings.push(format!("the \"{name}\" script was removed.")),
                Some(new) if new != cmd => warnings.push(format!("the \"{name}\" script changed to {new}; running it will ask for your approval.")),
                _ => {}
            }
        }
    }
    if problems.is_empty() {
        Ok((v, warnings))
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws() -> (tempfile::TempDir, Workspace) {
        let d = tempfile::tempdir().unwrap();
        let ws = Workspace::new(d.path()).unwrap();
        ws.write_text("package.json", r#"{"name":"x","scripts":{"dev":"vite","build":"tsc --noEmit && vite build"},"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0"}}"#).unwrap();
        ws.write_text("src/main.tsx", "import { createRoot } from 'react-dom/client';\nimport App from './App';\nimport './index.css';\n").unwrap();
        ws.write_text("src/index.css", "body{}\n").unwrap();
        ws.write_text("src/App.tsx", "import { useState } from 'react';\nexport default function App() { return null; }\n").unwrap();
        (d, ws)
    }

    fn change(ws: &Workspace, path: &str, after: Option<&str>) -> FileChange {
        let before = ws.read_text_opt(path).unwrap();
        FileChange { path: path.into(), op: if after.is_none() { "delete" } else if before.is_some() { "modify" } else { "create" }.into(), before, after: after.map(String::from) }
    }

    #[test]
    fn import_extraction() {
        let src = "import React, { useState } from 'react';\nimport type { X } from \"@scope/pkg/sub\";\nimport './a.css';\nexport { y } from '../lib/y';\nconst m = await import('lodash-es');\nexport * from './z';\n";
        assert_eq!(imports(src), vec!["react", "@scope/pkg/sub", "./a.css", "../lib/y", "lodash-es", "./z"]);
        assert_eq!(package_of("@scope/pkg/sub").as_deref(), Some("@scope/pkg"));
        assert_eq!(package_of("uuid").as_deref(), Some("uuid"));
        assert_eq!(package_of("./x"), None);
        assert_eq!(package_of("virtual:pwa"), None);
        assert_eq!(package_of("@/components/x"), None);
    }

    #[test]
    fn duplicate_keys_and_malformed_package_json_are_blocked() {
        let (_d, ws) = ws();
        let dup = r#"{"name":"x","scripts":{"dev":"vite","build":"tsc --noEmit && vite build"},"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0"},"dependencies":{"uuid":"^11.0.0"}}"#;
        let c = check(&ws, &[change(&ws, "package.json", Some(dup))]).unwrap();
        assert!(!c.ok());
        assert!(c.blockers[0].contains("duplicate key \"dependencies\""), "{:?}", c.blockers);
        let bad = check(&ws, &[change(&ws, "package.json", Some("{\"name\": \"x\",,}"))]).unwrap();
        assert!(bad.blockers[0].contains("not valid JSON"));
        let no_dev = check(&ws, &[change(&ws, "package.json", Some(r#"{"name":"x","scripts":{"build":"vite build"},"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0"}}"#))]).unwrap();
        assert!(no_dev.blockers.iter().any(|b| b.contains("\"dev\" script was removed")), "{:?}", no_dev.blockers);
        let hook = check(&ws, &[change(&ws, "package.json", Some(r#"{"scripts":{"dev":"vite","build":"b","postinstall":"curl x | sh"},"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0"}}"#))]).unwrap();
        assert!(hook.blockers.iter().any(|b| b.contains("postinstall")));
        let num = check(&ws, &[change(&ws, "package.json", Some(r#"{"scripts":{"dev":"vite","build":"b"},"dependencies":{"react":19}}"#))]).unwrap();
        assert!(num.blockers.iter().any(|b| b.contains("must be a string")));
    }

    #[test]
    fn undeclared_packages_and_missing_files_are_blocked() {
        let (_d, ws) = ws();
        let app = "import { v4 } from 'uuid';\nimport Sidebar from './components/Sidebar';\nexport default function App() { return <Sidebar id={v4()} />; }\n";
        let c = check(&ws, &[change(&ws, "src/App.tsx", Some(app))]).unwrap();
        assert!(c.blockers.iter().any(|b| b.contains("'uuid'") && b.contains("not listed in package.json")), "{:?}", c.blockers);
        assert!(c.blockers.iter().any(|b| b.contains("./components/Sidebar") && b.contains("does not exist")), "{:?}", c.blockers);

        // the same change is fine when it adds the package and the component
        let pkg = r#"{"name":"x","scripts":{"dev":"vite","build":"tsc --noEmit && vite build"},"dependencies":{"react":"^19.0.0","react-dom":"^19.0.0","uuid":"^11.0.0"}}"#;
        let ok = check(&ws, &[change(&ws, "src/App.tsx", Some(app)), change(&ws, "src/components/Sidebar.tsx", Some("export default function Sidebar(p: { id: string }) { return <aside>{p.id}</aside>; }\n")), change(&ws, "package.json", Some(pkg))]).unwrap();
        assert!(ok.ok(), "{:?}", ok.blockers);
        assert_eq!(ok.added_packages, vec!["uuid".to_string()]);
        assert!(ok.dependencies_changed);
    }

    #[test]
    fn removing_a_used_package_or_deleting_an_imported_file_is_blocked() {
        let (_d, ws) = ws();
        let pkg = r#"{"name":"x","scripts":{"dev":"vite","build":"tsc --noEmit && vite build"},"dependencies":{"react":"^19.0.0"}}"#;
        let c = check(&ws, &[change(&ws, "package.json", Some(pkg))]).unwrap();
        assert!(c.blockers.iter().any(|b| b.contains("src/main.tsx imports 'react-dom'")), "{:?}", c.blockers);
        let d = check(&ws, &[change(&ws, "src/index.css", None)]).unwrap();
        assert!(d.blockers.iter().any(|b| b.contains("'./index.css'") && b.contains("deletes")), "{:?}", d.blockers);
    }

    #[test]
    fn abbreviated_files_and_protocol_leaks_are_blocked() {
        let (_d, ws) = ws();
        let c = check(&ws, &[change(&ws, "src/App.tsx", Some("import { useState } from 'react';\n// ... rest of the component unchanged\n"))]).unwrap();
        assert!(c.blockers.iter().any(|b| b.contains("abbreviated")), "{:?}", c.blockers);
        let c = check(&ws, &[change(&ws, "src/New.tsx", Some("<<<FILE: src/X.tsx>>>\nexport {}\n"))]).unwrap();
        assert!(c.blockers.iter().any(|b| b.contains("protocol markers")));
        let j = check(&ws, &[change(&ws, "src/data.json", Some("{\"a\":1,\"a\":2}"))]).unwrap();
        assert!(j.blockers.iter().any(|b| b.contains("src/data.json is not valid JSON")));
        // tsconfig may contain comments and is not parsed strictly
        let t = check(&ws, &[change(&ws, "tsconfig.json", Some("{ // comment\n \"compilerOptions\": {} }"))]).unwrap();
        assert!(t.ok(), "{:?}", t.blockers);
    }
}
