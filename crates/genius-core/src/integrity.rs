//! Cheap, deterministic checks that generated output hangs together *before* anything is
//! built, so a half-written project is never reported as a finished one.
//!
//! A model that runs out of output budget typically stops after writing the leaf files and
//! before the file that wires them together (`src/App.tsx` is written last). The build then
//! "succeeds" on the untouched starter template. These checks find that situation without a
//! model call: relative imports that point at files nobody wrote, and the starter
//! placeholder still being the application.

use std::collections::HashSet;
use std::sync::LazyLock;

/// A relative import whose target exists neither in the generated files nor on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Unresolved {
    pub importer: String,
    pub specifier: String,
}

static IMPORT_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?m)(?:\bimport\s+(?:[^'";]*?\sfrom\s+)?|\bexport\s+[^'";]*?\sfrom\s+|\bimport\s*\(\s*)['"](\.{1,2}/[^'"\n]+)['"]"#).unwrap()
});

const CODE_EXT: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs"];
const TRY_EXT: &[&str] = &[".ts", ".tsx", ".js", ".jsx", ".mjs", ".json", ".css"];

fn is_code(path: &str) -> bool {
    path.rsplit('.').next().is_some_and(|e| CODE_EXT.contains(&e))
}

/// `dir` + relative specifier → normalised project-relative path (no leading `./`).
fn join(dir: &str, spec: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
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

fn exists(candidate: &str, generated: &HashSet<&str>, on_disk: &dyn Fn(&str) -> bool) -> bool {
    generated.contains(candidate) || on_disk(candidate)
}

fn resolves(base: &str, generated: &HashSet<&str>, on_disk: &dyn Fn(&str) -> bool) -> bool {
    if exists(base, generated, on_disk) && base.rsplit('/').next().is_some_and(|n| n.contains('.')) {
        return true; // `./x.css`, `./logo.svg`, `./x.tsx`
    }
    TRY_EXT.iter().any(|e| exists(&format!("{base}{e}"), generated, on_disk)) || TRY_EXT.iter().any(|e| exists(&format!("{base}/index{e}"), generated, on_disk))
}

/// Relative imports of the generated code files that cannot be resolved.
pub fn unresolved_relative_imports(files: &[(String, String)], on_disk: &dyn Fn(&str) -> bool) -> Vec<Unresolved> {
    let generated: HashSet<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
    let mut out: Vec<Unresolved> = Vec::new();
    for (path, content) in files {
        if !is_code(path) {
            continue;
        }
        let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        for cap in IMPORT_RE.captures_iter(content) {
            let spec = &cap[1];
            let Some(base) = join(dir, spec) else {
                out.push(Unresolved { importer: path.clone(), specifier: spec.to_string() });
                continue;
            };
            if !resolves(&base, &generated, on_disk) {
                let u = Unresolved { importer: path.clone(), specifier: spec.to_string() };
                if !out.contains(&u) {
                    out.push(u);
                }
            }
        }
    }
    out
}

/// Human-readable lines for the model: which imports point nowhere.
pub fn render_unresolved(items: &[Unresolved]) -> String {
    items.iter().take(30).map(|u| format!("- {} imports \"{}\", but that file does not exist", u.importer, u.specifier)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, c: &str) -> (String, String) {
        (p.into(), c.into())
    }

    #[test]
    fn finds_imports_nobody_wrote() {
        let files = vec![
            f("src/App.tsx", "import Dashboard from './pages/Dashboard';\nimport { useStore } from './hooks/useStore';\nimport './App.css';\nimport React from 'react';\n"),
            f("src/hooks/useStore.ts", "export const useStore = () => 1;\n"),
        ];
        let none = |_: &str| false;
        let u = unresolved_relative_imports(&files, &none);
        let specs: Vec<&str> = u.iter().map(|x| x.specifier.as_str()).collect();
        assert_eq!(specs, vec!["./pages/Dashboard", "./App.css"], "bare package imports are ignored, written files resolve");
    }

    #[test]
    fn resolves_extensions_index_files_parent_dirs_and_disk() {
        let files = vec![
            f("src/pages/Home.tsx", "import Card from '../components/Card';\nimport { x } from '../lib';\nimport type { T } from '../types';\nexport * from './parts';\nconst L = () => import('./Lazy');\n"),
            f("src/components/Card.tsx", ""),
            f("src/lib/index.ts", ""),
            f("src/pages/parts/index.tsx", ""),
        ];
        let disk = |p: &str| p == "src/types.ts" || p == "src/pages/Lazy.tsx";
        assert!(unresolved_relative_imports(&files, &disk).is_empty());
    }

    #[test]
    fn escaping_the_project_root_is_unresolved_and_non_code_is_skipped() {
        let files = vec![f("a.ts", "import x from '../../outside';\n"), f("README.md", "import y from './nope';\n")];
        let u = unresolved_relative_imports(&files, &|_| false);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].importer, "a.ts");
    }

    #[test]
    fn multiline_named_imports() {
        let files = vec![f("src/App.tsx", "import {\n  A,\n  B,\n} from './missing';\n")];
        assert_eq!(unresolved_relative_imports(&files, &|_| false).len(), 1);
    }
}
