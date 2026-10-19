//! Project Brain: a persistent, incrementally updated understanding of one authorised project.
//!
//! What it holds, and how far each part can be trusted:
//!
//! * **Verified source facts** — files, languages, entry points, build/test commands, symbols with
//!   line numbers, the import graph. Read from disk by a *pattern-based scan* (regular
//!   expressions per language), not a compiler: it finds ordinary declarations and imports, it
//!   does not resolve types, macros, re-exports through barrels or dynamic paths. Every answer
//!   carries the time of the scan.
//! * **Recorded knowledge** — project memory, earlier changes and Git history. Useful context, but
//!   it describes the past: it is returned in its own section and flagged *stale* when it mentions
//!   files that no longer exist or is an old agent inference.
//!
//! Retrieval returns a handful of ranked hits with short excerpts instead of whole files, so a
//! model gets focused context. The index is keyed by project: nothing from one project is ever
//! offered while working on another. Re-indexing only re-reads files whose size or modification
//! time changed.

use crate::workspace::Workspace;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::LazyLock;

pub const INDEX_VERSION: u32 = 2;
const MAX_SOURCE_FILES: usize = 3000;
const MAX_FILE_BYTES: u64 = 400_000;
const MAX_SYMBOLS_PER_FILE: usize = 60;

pub const PROVENANCE: &str = "Facts come from a pattern-based scan of the files on disk (not a compiler analysis): declarations and imports are found, types, macros and dynamic imports are not resolved.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub lang: String,
    pub size: u64,
    /// `size:mtime_ms` — changes when the file changes; unchanged files are not re-read.
    pub fp: String,
    pub lines: usize,
    pub is_test: bool,
    pub symbols: Vec<Symbol>,
    /// Project files this file imports (resolved).
    pub imports: Vec<String>,
    /// Packages it imports from outside the project.
    pub external: Vec<String>,
    /// Relative imports that matched no project file.
    pub unresolved: Vec<String>,
    /// Names the file calls or instantiates (call-like patterns; see [`callers`]).
    #[serde(default)]
    pub calls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub path: String,
    pub kind: String,
    pub name: Option<String>,
    pub scripts: Vec<(String, String)>,
    pub dependencies: Vec<String>,
    pub main: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrainIndex {
    pub version: u32,
    pub indexed_at: String,
    pub files: BTreeMap<String, FileNode>,
    pub manifests: Vec<Manifest>,
    /// Files that exist but were not scanned (too many, too large, binary, secrets).
    pub skipped: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub scanned: usize,
    pub reused: usize,
    pub removed: usize,
}

// ------------------------------------------------------------------------------------ parsing

static TS_IMPORT_FROM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*(?:import|export)\b[^'";]*?\bfrom\s*['"]([^'"]+)['"]"#).unwrap());
static TS_IMPORT_BARE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?m)^\s*import\s*['"]([^'"]+)['"]"#).unwrap());
static TS_DYNAMIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\b(?:require|import)\(\s*['"]([^'"]+)['"]\s*\)"#).unwrap());
static TS_FUNCTION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)").unwrap());
static TS_CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)").unwrap());
static TS_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:export\s+)?(?:declare\s+)?(interface|type|enum)\s+([A-Za-z_$][\w$]*)").unwrap());
static TS_EXPORT_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*export\s+(?:default\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)").unwrap());
static TS_TOP_COMPONENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:const|let)\s+([A-Z][\w$]*)\s*[:=]").unwrap());
static PY_IMPORT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*import\s+([\w.]+)").unwrap());
static PY_FROM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*from\s+(\.*[\w.]*)\s+import\b").unwrap());
static PY_DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:async\s+)?def\s+(\w+)").unwrap());
static PY_CLASS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^class\s+(\w+)").unwrap());
static RS_USE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+(crate|super|self)::([\w:]+)").unwrap());
static RS_USE_EXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([a-z_][\w]*)::").unwrap());
static RS_MOD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;").unwrap());
static RS_FN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)").unwrap());
static RS_TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(struct|enum|trait)\s+(\w+)").unwrap());
static RS_IMPL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*impl(?:<[^>]*>)?\s+(?:[\w:<>, ]+\s+for\s+)?([A-Z]\w*)").unwrap());

pub fn lang_of(path: &str) -> Option<&'static str> {
    match path.rsplit('.').next()? {
        "ts" | "tsx" | "mts" | "cts" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("javascript"),
        "py" => Some("python"),
        "rs" => Some("rust"),
        _ => None,
    }
}

pub fn is_test_path(path: &str) -> bool {
    let p = path.to_lowercase();
    let name = p.rsplit('/').next().unwrap_or("");
    p.contains("/tests/") || p.starts_with("tests/") || p.contains("/__tests__/") || p.contains("/test/") || name.contains(".test.") || name.contains(".spec.") || name.ends_with("_test.py") || name.ends_with("_test.rs") || name.starts_with("test_")
}

/// Package name of a bare import specifier (`@scope/pkg/sub` → `@scope/pkg`, `react/jsx-runtime` → `react`).
fn package_of(spec: &str) -> String {
    let mut it = spec.split('/');
    let first = it.next().unwrap_or("");
    if first.starts_with('@') { format!("{first}/{}", it.next().unwrap_or("")) } else { first.to_string() }
}

struct Parsed {
    symbols: Vec<Symbol>,
    /// Raw import specifiers to resolve.
    specs: Vec<String>,
    lines: usize,
    calls: Vec<String>,
}

fn parse_source(lang: &str, path: &str, text: &str) -> Parsed {
    let mut symbols = Vec::new();
    let mut push = |name: &str, kind: &str, line: usize| {
        if symbols.len() < MAX_SYMBOLS_PER_FILE && !name.is_empty() && !symbols.iter().any(|s: &Symbol| s.name == name && s.kind == kind) {
            symbols.push(Symbol { name: name.to_string(), kind: kind.to_string(), line });
        }
    };
    let mut specs: Vec<String> = Vec::new();
    match lang {
        "typescript" | "javascript" => {
            for (i, l) in text.lines().enumerate() {
                let n = i + 1;
                if let Some(c) = TS_FUNCTION.captures(l) {
                    push(&c[1], "function", n);
                } else if let Some(c) = TS_CLASS.captures(l) {
                    push(&c[1], "class", n);
                } else if let Some(c) = TS_TYPE.captures(l) {
                    push(&c[2], &c[1], n);
                } else if let Some(c) = TS_EXPORT_VAR.captures(l) {
                    push(&c[1], "const", n);
                } else if let Some(c) = TS_TOP_COMPONENT.captures(l) {
                    push(&c[1], "component", n);
                }
            }
            for re in [&*TS_IMPORT_FROM, &*TS_IMPORT_BARE, &*TS_DYNAMIC] {
                specs.extend(re.captures_iter(text).map(|c| c[1].to_string()));
            }
        }
        "python" => {
            for (i, l) in text.lines().enumerate() {
                if let Some(c) = PY_DEF.captures(l) {
                    push(&c[1], "function", i + 1);
                } else if let Some(c) = PY_CLASS.captures(l) {
                    push(&c[1], "class", i + 1);
                }
            }
            specs.extend(PY_IMPORT.captures_iter(text).map(|c| c[1].to_string()));
            specs.extend(PY_FROM.captures_iter(text).map(|c| c[1].to_string()));
        }
        "rust" => {
            for (i, l) in text.lines().enumerate() {
                if let Some(c) = RS_FN.captures(l) {
                    push(&c[1], "function", i + 1);
                } else if let Some(c) = RS_TYPE.captures(l) {
                    push(&c[2], &c[1], i + 1);
                } else if let Some(c) = RS_IMPL.captures(l) {
                    push(&c[1], "impl", i + 1);
                }
            }
            specs.extend(RS_MOD.captures_iter(text).map(|c| format!("mod:{}", &c[1])));
            specs.extend(RS_USE.captures_iter(text).map(|c| format!("{}::{}", &c[1], &c[2])));
            specs.extend(RS_USE_EXT.captures_iter(text).filter(|c| !matches!(&c[1], "crate" | "super" | "self" | "std" | "core" | "alloc")).map(|c| format!("ext:{}", &c[1])));
        }
        _ => {}
    }
    let _ = path;
    specs.sort();
    specs.dedup();
    Parsed { symbols, specs, lines: text.lines().count(), calls: call_names(lang, text) }
}

/// Normalise `a/b/../c` → `a/c`; `None` when the path climbs out of the project.
fn join_normalised(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
    for seg in rel.split('/') {
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

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

/// Where a specifier points: a project file, an external package, or nowhere we can tell.
enum Target {
    File(String),
    External(String),
    Unresolved(String),
}

fn resolve(lang: &str, from: &str, spec: &str, all: &HashSet<&str>) -> Target {
    let exists = |p: &str| all.contains(p).then(|| p.to_string());
    match lang {
        "typescript" | "javascript" => {
            let base = if spec.starts_with('.') {
                join_normalised(dir_of(from), spec)
            } else if let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) {
                Some(format!("src/{rest}"))
            } else {
                return Target::External(package_of(spec));
            };
            let Some(base) = base else { return Target::Unresolved(spec.to_string()) };
            let exts = ["", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".json", ".css"];
            // `./x.js` may point at `x.ts`: TypeScript writes the output extension.
            let stem = base.rsplit_once('.').filter(|(_, e)| matches!(*e, "js" | "jsx" | "mjs")).map(|(s, _)| s.to_string());
            let found = exts.iter().find_map(|e| exists(&format!("{base}{e}"))).or_else(|| stem.and_then(|s| [".ts", ".tsx"].iter().find_map(|e| exists(&format!("{s}{e}"))))).or_else(|| ["index.ts", "index.tsx", "index.js", "index.jsx"].iter().find_map(|i| exists(&format!("{base}/{i}"))));
            match found {
                Some(f) => Target::File(f),
                None if spec.starts_with('.') || spec.starts_with("@/") || spec.starts_with("~/") => Target::Unresolved(spec.to_string()),
                None => Target::External(package_of(spec)),
            }
        }
        "python" => {
            let dots = spec.chars().take_while(|c| *c == '.').count();
            let module = spec.trim_start_matches('.').replace('.', "/");
            let roots: Vec<String> = if dots > 0 {
                let mut d = dir_of(from).to_string();
                for _ in 1..dots {
                    d = dir_of(&d).to_string();
                }
                vec![d]
            } else {
                vec![String::new(), "src".into(), dir_of(from).to_string()]
            };
            for r in roots {
                let base = if r.is_empty() { module.clone() } else if module.is_empty() { r.clone() } else { format!("{r}/{module}") };
                if let Some(f) = exists(&format!("{base}.py")).or_else(|| exists(&format!("{base}/__init__.py"))) {
                    return Target::File(f);
                }
            }
            if dots > 0 { Target::Unresolved(spec.to_string()) } else { Target::External(spec.split('.').next().unwrap_or(spec).to_string()) }
        }
        "rust" => {
            if let Some(ext) = spec.strip_prefix("ext:") {
                return Target::External(ext.to_string());
            }
            let file_name = from.rsplit('/').next().unwrap_or("");
            // Directory holding the submodules of this file.
            let own_dir = if matches!(file_name, "main.rs" | "lib.rs" | "mod.rs") { dir_of(from).to_string() } else { from.trim_end_matches(".rs").to_string() };
            if let Some(m) = spec.strip_prefix("mod:") {
                let base = if own_dir.is_empty() { m.to_string() } else { format!("{own_dir}/{m}") };
                return match exists(&format!("{base}.rs")).or_else(|| exists(&format!("{base}/mod.rs"))) {
                    Some(f) => Target::File(f),
                    None => Target::Unresolved(spec.to_string()),
                };
            }
            // `crate::a::b` → the crate's `src` directory (nearest ancestor named src).
            let crate_src = from.find("/src/").map(|i| &from[..i + 4]).or_else(|| from.starts_with("src/").then_some("src")).unwrap_or("src");
            let mut segs: Vec<&str> = spec.split("::").collect();
            let head = segs.remove(0);
            let root = match head {
                "crate" => crate_src.to_string(),
                "super" => dir_of(&own_dir).to_string(),
                _ => own_dir.clone(),
            };
            // The longest prefix of the path that is a file wins (`crate::a::Type` → a.rs).
            for take in (1..=segs.len()).rev() {
                let joined = segs[..take].join("/");
                let base = if root.is_empty() { joined } else { format!("{root}/{joined}") };
                if let Some(f) = exists(&format!("{base}.rs")).or_else(|| exists(&format!("{base}/mod.rs"))) {
                    return Target::File(f);
                }
            }
            Target::Unresolved(spec.to_string())
        }
        _ => Target::Unresolved(spec.to_string()),
    }
}

fn parse_manifest(path: &str, text: &str) -> Option<Manifest> {
    let file = path.rsplit('/').next()?;
    match file {
        "package.json" | "tauri.conf.json" => {
            let v: Value = serde_json::from_str(text).ok()?;
            let scripts = v.get("scripts").and_then(|s| s.as_object()).map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect()).unwrap_or_default();
            let mut deps: Vec<String> = ["dependencies", "devDependencies"].iter().filter_map(|k| v.get(*k).and_then(|d| d.as_object())).flat_map(|o| o.keys().cloned()).collect();
            deps.sort();
            let name = v.get("name").or_else(|| v.get("productName")).and_then(|n| n.as_str()).map(String::from);
            Some(Manifest { path: path.into(), kind: if file == "package.json" { "npm" } else { "tauri" }.into(), name, scripts, dependencies: deps, main: v.get("main").or_else(|| v.get("module")).and_then(|m| m.as_str()).map(String::from) })
        }
        "Cargo.toml" => {
            let name = Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]+)""#).ok()?.captures(text).map(|c| c[1].to_string());
            let in_deps = text.split("\n[").filter(|s| s.starts_with("dependencies") || s.starts_with("workspace.dependencies") || s.starts_with("dev-dependencies")).flat_map(|s| s.lines().skip(1).take_while(|l| !l.starts_with('[')).filter_map(|l| l.split('=').next().map(|k| k.trim().to_string())).filter(|k| !k.is_empty() && !k.starts_with('#'))).collect::<Vec<_>>();
            Some(Manifest { path: path.into(), kind: "cargo".into(), name, scripts: vec![], dependencies: in_deps, main: None })
        }
        "pyproject.toml" | "requirements.txt" => {
            let name = Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]+)""#).ok()?.captures(text).map(|c| c[1].to_string());
            let deps: Vec<String> = if file == "requirements.txt" { text.lines().map(|l| l.split(['=', '<', '>', '~', '[', ' ', ';']).next().unwrap_or("").trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('-')).collect() } else { vec![] };
            Some(Manifest { path: path.into(), kind: "python".into(), name, scripts: vec![], dependencies: deps, main: None })
        }
        _ => None,
    }
}

const MANIFEST_NAMES: &[&str] = &["package.json", "Cargo.toml", "pyproject.toml", "requirements.txt", "tauri.conf.json"];

fn fingerprint(ws: &Workspace, rel: &str, size: u64) -> String {
    let mtime = std::fs::metadata(ws.root().join(rel)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    format!("{size}:{mtime}")
}

/// Build or update the index. Files whose fingerprint is unchanged keep their previous parse.
pub fn build(ws: &Workspace, previous: Option<&BrainIndex>) -> crate::error::Result<(BrainIndex, IndexStats)> {
    let listing = ws.list_files()?;
    let prev = previous.filter(|p| p.version == INDEX_VERSION);
    let mut stats = IndexStats::default();
    let mut skipped = 0usize;
    let mut truncated = false;
    let mut raw: BTreeMap<String, (FileNode, Vec<String>, bool)> = BTreeMap::new(); // path → (node, specs, reused)
    let mut manifests = Vec::new();
    for f in &listing {
        let name = f.path.rsplit('/').next().unwrap_or("");
        if f.sensitive || !f.is_text {
            skipped += 1;
            continue;
        }
        if MANIFEST_NAMES.contains(&name) && !f.path.contains("node_modules") {
            if let Ok(t) = ws.read_text(&f.path) {
                manifests.extend(parse_manifest(&f.path, &t));
            }
        }
        let Some(lang) = lang_of(&f.path) else { continue };
        if f.size > MAX_FILE_BYTES {
            skipped += 1;
            continue;
        }
        if raw.len() >= MAX_SOURCE_FILES {
            truncated = true;
            skipped += 1;
            continue;
        }
        let fp = fingerprint(ws, &f.path, f.size);
        if let Some(old) = prev.and_then(|p| p.files.get(&f.path)).filter(|o| o.fp == fp) {
            // Specs are re-derived below from the stored node only for resolution; keep the node as is.
            raw.insert(f.path.clone(), (old.clone(), vec![], true));
            stats.reused += 1;
            continue;
        }
        let Ok(text) = ws.read_text(&f.path) else {
            skipped += 1;
            continue;
        };
        let parsed = parse_source(lang, &f.path, &text);
        raw.insert(f.path.clone(), (FileNode { lang: lang.into(), size: f.size, fp, lines: parsed.lines, is_test: is_test_path(&f.path), symbols: parsed.symbols, imports: vec![], external: vec![], unresolved: vec![], calls: parsed.calls }, parsed.specs, false));
        stats.scanned += 1;
    }
    stats.removed = prev.map(|p| p.files.keys().filter(|k| !raw.contains_key(*k)).count()).unwrap_or(0);

    // Imports are resolved against the *current* file set. A changed file is resolved from its fresh
    // specifiers; an unchanged file only retries the specifiers that failed before, because a file
    // created since may now satisfy them. Edges to deleted files are dropped below.
    let nodes: HashSet<String> = raw.keys().cloned().collect();
    // Imports of stylesheets, JSON and other project files resolve (they are not unresolved), but only
    // scanned source files become graph edges.
    let all: HashSet<&str> = listing.iter().filter(|f| !f.sensitive).map(|f| f.path.as_str()).collect();
    // path → (merge into the existing node?, files, external packages, unresolved specifiers)
    type Resolution = (bool, Vec<String>, Vec<String>, Vec<String>);
    let mut resolved: HashMap<String, Resolution> = HashMap::new();
    for (path, (node, specs, reused)) in &raw {
        let specs: &Vec<String> = if *reused { &node.unresolved } else { specs };
        if specs.is_empty() {
            continue;
        }
        let (mut files, mut ext, mut unres) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        for s in specs {
            match resolve(&node.lang, path, s, &all) {
                Target::File(f) if f != *path => {
                    files.insert(f);
                }
                Target::File(_) => {}
                Target::External(p) => {
                    ext.insert(p);
                }
                Target::Unresolved(u) => {
                    unres.insert(u);
                }
            }
        }
        resolved.insert(path.clone(), (*reused, files.into_iter().collect(), ext.into_iter().collect(), unres.into_iter().collect()));
    }
    let mut files = BTreeMap::new();
    for (path, (mut node, _, _)) in raw {
        if let Some((merge, i, e, u)) = resolved.remove(&path) {
            if merge {
                node.imports.extend(i);
                node.imports.sort();
                node.imports.dedup();
                node.external.extend(e);
                node.external.sort();
                node.external.dedup();
            } else {
                node.imports = i;
                node.external = e;
            }
            node.unresolved = u;
        }
        node.imports.retain(|i| nodes.contains(i));
        files.insert(path, node);
    }
    manifests.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((BrainIndex { version: INDEX_VERSION, indexed_at: chrono::Utc::now().to_rfc3339(), files, manifests, skipped, truncated }, stats))
}

// ------------------------------------------------------------------------------------ call relations

/// Identifiers that look like calls: `name(`, `new Name`, JSX `<Name`, Rust `Type::func(`. Language
/// keywords and very short names are dropped. This is a text pattern, not name resolution.
fn call_names(lang: &str, text: &str) -> Vec<String> {
    static CALL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Za-z_$][\w$]*)\s*(?:::<[^>]*>)?\(").unwrap());
    static NEW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bnew\s+([A-Z][\w$]*)").unwrap());
    static JSX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([A-Z][\w$]*)[\s/>]").unwrap());
    static RS_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b([A-Z]\w*)::\w+").unwrap());
    const KEYWORDS: &[&str] = &[
        "if", "for", "while", "switch", "catch", "function", "return", "typeof", "await", "async", "else", "match", "fn", "def", "class", "super", "self", "Some", "None", "Ok", "Err", "Box", "Vec", "String", "Option", "Result", "assert", "assert_eq", "println", "format", "vec", "print", "len", "str", "int", "map", "filter", "push", "from", "into", "unwrap", "clone", "join", "get", "set", "has", "new", "require", "import", "export", "constructor", "lambda", "with", "range", "enumerate", "isinstance", "useState", "useEffect", "useMemo", "useRef", "useCallback",
    ];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut add = |n: &str| {
        if n.len() >= 3 && !KEYWORDS.contains(&n) && seen.len() < 400 {
            seen.insert(n.to_string());
        }
    };
    for l in text.lines() {
        for c in CALL.captures_iter(l) {
            let at = c.get(0).map_or(0, |m| m.start());
            let before = l[..at].trim_end();
            // `fn name(`, `def name(`, `function name(`, `class Name(` declare the name; they are not calls
            if ["fn", "def", "function", "class"].iter().any(|k| before.ends_with(k) && before.len() >= k.len() && before[..before.len() - k.len()].chars().last().is_none_or(|ch| !ch.is_alphanumeric() && ch != '_')) {
                continue;
            }
            add(&c[1]);
        }
        if matches!(lang, "typescript" | "javascript") {
            for c in NEW.captures_iter(l) {
                add(&c[1]);
            }
            for c in JSX.captures_iter(l) {
                add(&c[1]);
            }
        }
        if lang == "rust" {
            for c in RS_PATH.captures_iter(l) {
                add(&c[1]);
            }
        }
    }
    seen.into_iter().collect()
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub path: String,
    pub kind: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Caller {
    pub path: String,
    /// The calling file imports the file that defines the symbol (so the match is likely the same symbol).
    pub imports_definition: bool,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallInfo {
    pub symbol: String,
    pub definitions: Vec<Definition>,
    pub callers: Vec<Caller>,
    /// The name is defined in more than one file, so name matches may belong to another symbol.
    pub ambiguous: bool,
    pub truncated: bool,
    pub note: &'static str,
}

pub const CALL_NOTE: &str = "Callers are files whose text contains a call-like use of the name (a pattern scan, not name resolution): those that import the defining file are likely real, the others only share the name.";

/// Where `symbol` is defined and which scanned files use it.
pub fn callers(ix: &BrainIndex, symbol: &str, limit: usize) -> CallInfo {
    let symbol = symbol.trim();
    let definitions: Vec<Definition> = ix.files.iter().flat_map(|(p, n)| n.symbols.iter().filter(|s| s.name == symbol).map(|s| Definition { path: p.clone(), kind: s.kind.clone(), line: s.line })).collect();
    let def_files: HashSet<&str> = definitions.iter().map(|d| d.path.as_str()).collect();
    let mut found: Vec<Caller> = ix
        .files
        .iter()
        .filter(|(p, n)| !def_files.contains(p.as_str()) && n.calls.iter().any(|c| c == symbol))
        .map(|(p, n)| Caller { path: p.clone(), imports_definition: n.imports.iter().any(|i| def_files.contains(i.as_str())) })
        .collect();
    found.sort_by(|a, b| b.imports_definition.cmp(&a.imports_definition).then(a.path.cmp(&b.path)));
    let truncated = found.len() > limit;
    found.truncate(limit);
    CallInfo { symbol: symbol.to_string(), ambiguous: def_files.len() > 1, definitions, callers: found, truncated, note: CALL_NOTE }
}

/// Symbols defined elsewhere in the project that `path` calls: `(name, defining file)`. Definitions in
/// files it imports win over same-named ones elsewhere.
pub fn callees(ix: &BrainIndex, path: &str, limit: usize) -> Vec<(String, String)> {
    let Some(node) = ix.files.get(path) else { return vec![] };
    let mut defs: HashMap<&str, Vec<&str>> = HashMap::new();
    for (p, n) in &ix.files {
        if p != path {
            for s in n.symbols.iter().filter(|s| matches!(s.kind.as_str(), "function" | "class" | "component" | "struct" | "impl" | "const")) {
                defs.entry(s.name.as_str()).or_default().push(p.as_str());
            }
        }
    }
    let mut out = Vec::new();
    for c in &node.calls {
        let Some(files) = defs.get(c.as_str()) else { continue };
        if let Some(f) = files.iter().find(|f| node.imports.iter().any(|i| i == **f)).or_else(|| (files.len() == 1).then(|| &files[0])) {
            out.push((c.clone(), f.to_string()));
        }
        if out.len() >= limit {
            break;
        }
    }
    out
}

// ------------------------------------------------------------------------------------- queries

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Command {
    pub purpose: String,
    pub command: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleInfo {
    pub path: String,
    pub files: usize,
    pub lines: usize,
    pub languages: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFile {
    pub path: String,
    pub imported_by: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub project_type: Vec<String>,
    pub languages: Vec<(String, usize)>,
    pub entry_points: Vec<String>,
    pub commands: Vec<Command>,
    pub modules: Vec<ModuleInfo>,
    pub key_files: Vec<KeyFile>,
    pub external_deps: Vec<(String, usize)>,
    pub files: usize,
    pub lines: usize,
    pub symbols: usize,
    pub unresolved_imports: usize,
    pub tests: usize,
    pub indexed_at: String,
    pub truncated: bool,
    pub provenance: &'static str,
}

/// Reverse import graph: file → files that import it.
pub fn dependents(ix: &BrainIndex) -> HashMap<&str, Vec<&str>> {
    let mut rev: HashMap<&str, Vec<&str>> = HashMap::new();
    for (p, n) in &ix.files {
        for i in &n.imports {
            rev.entry(i.as_str()).or_default().push(p.as_str());
        }
    }
    rev
}

pub fn overview(ix: &BrainIndex) -> Overview {
    let rev = dependents(ix);
    let mut langs: BTreeMap<&str, usize> = BTreeMap::new();
    for n in ix.files.values() {
        *langs.entry(n.lang.as_str()).or_default() += 1;
    }
    let mut languages: Vec<(String, usize)> = langs.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    languages.sort_by(|a, b| b.1.cmp(&a.1));

    // Project type from manifests and layout.
    let dep = |name: &str| ix.manifests.iter().any(|m| m.dependencies.iter().any(|d| d == name));
    let mut project_type = Vec::new();
    if ix.manifests.iter().any(|m| m.kind == "tauri") || dep("@tauri-apps/api") || dep("tauri") {
        project_type.push("Tauri desktop app".to_string());
    }
    if dep("react") {
        project_type.push(format!("React{}", if dep("vite") { " (Vite)" } else if dep("next") { " (Next.js)" } else { "" }));
    } else if dep("vue") {
        project_type.push("Vue".into());
    } else if dep("svelte") {
        project_type.push("Svelte".into());
    } else if dep("express") {
        project_type.push("Node.js server (Express)".into());
    }
    if ix.manifests.iter().any(|m| m.kind == "cargo") {
        let crates = ix.manifests.iter().filter(|m| m.kind == "cargo").count();
        project_type.push(if crates > 1 { format!("Rust workspace ({crates} crates)") } else { "Rust crate".into() });
    }
    if ix.manifests.iter().any(|m| m.kind == "python") || languages.first().is_some_and(|l| l.0 == "python") {
        project_type.push("Python".into());
    }
    if project_type.is_empty() {
        if let Some((l, _)) = languages.first() {
            project_type.push(format!("{l} project"));
        } else {
            project_type.push("no recognised source files".into());
        }
    }

    // Entry points: declared in manifests, else conventional names that exist.
    let mut entry_points: Vec<String> = ix.manifests.iter().filter_map(|m| m.main.clone()).filter(|m| ix.files.contains_key(m.trim_start_matches("./"))).map(|m| m.trim_start_matches("./").to_string()).collect();
    for cand in ["src/main.tsx", "src/main.ts", "src/index.tsx", "src/index.ts", "src/main.jsx", "src/index.js", "index.js", "main.py", "app.py", "__main__.py", "src/main.rs", "src/lib.rs"] {
        if ix.files.contains_key(cand) && !entry_points.iter().any(|e| e == cand) {
            entry_points.push(cand.to_string());
        }
    }
    for p in ix.files.keys().filter(|p| p.ends_with("/src/main.rs") || p.ends_with("/src/lib.rs")).take(6) {
        if !entry_points.contains(p) {
            entry_points.push(p.clone());
        }
    }
    entry_points.truncate(10);

    // Commands: what the manifests really declare.
    let mut commands = Vec::new();
    for m in ix.manifests.iter().filter(|m| m.kind == "npm") {
        let dir = dir_of(&m.path);
        let prefix = if dir.is_empty() { String::new() } else { format!("cd {dir} && ") };
        for (purpose, names) in [("dev server", &["dev", "start"][..]), ("build", &["build"]), ("test", &["test"]), ("lint", &["lint"]), ("type check", &["typecheck", "tsc"])] {
            if let Some((n, body)) = m.scripts.iter().find(|(k, _)| names.contains(&k.as_str())) {
                commands.push(Command { purpose: purpose.into(), command: format!("{prefix}npm run {n}"), source: format!("{} scripts.{n}: {body}", m.path) });
            }
        }
    }
    if let Some(m) = ix.manifests.iter().find(|m| m.kind == "cargo") {
        let dir = dir_of(&m.path);
        let prefix = if dir.is_empty() { String::new() } else { format!("cd {dir} && ") };
        commands.push(Command { purpose: "build".into(), command: format!("{prefix}cargo build"), source: m.path.clone() });
        commands.push(Command { purpose: "test".into(), command: format!("{prefix}cargo test"), source: m.path.clone() });
    }
    if languages.iter().any(|l| l.0 == "python") && (ix.files.keys().any(|p| is_test_path(p) && p.ends_with(".py")) || dep("pytest")) {
        commands.push(Command { purpose: "test".into(), command: "pytest".into(), source: "Python test files / pytest dependency".into() });
    }

    // Modules: first meaningful directory level.
    let mut mods: BTreeMap<String, (usize, usize, BTreeSet<String>)> = BTreeMap::new();
    for (p, n) in &ix.files {
        let segs: Vec<&str> = p.split('/').collect();
        if segs.len() < 2 {
            continue;
        }
        let key = if matches!(segs[0], "crates" | "packages" | "apps") && segs.len() > 2 { format!("{}/{}", segs[0], segs[1]) } else if segs[0] == "src" && segs.len() > 2 { format!("src/{}", segs[1]) } else { segs[0].to_string() };
        let e = mods.entry(key).or_default();
        e.0 += 1;
        e.1 += n.lines;
        e.2.insert(n.lang.clone());
    }
    let mut modules: Vec<ModuleInfo> = mods.into_iter().map(|(path, (files, lines, l))| ModuleInfo { path, files, lines, languages: l.into_iter().collect() }).collect();
    modules.sort_by(|a, b| b.lines.cmp(&a.lines));
    modules.truncate(14);

    // Key files: entry points and the most-imported non-test files.
    let mut key_files: Vec<KeyFile> = entry_points.iter().map(|p| KeyFile { path: p.clone(), imported_by: rev.get(p.as_str()).map_or(0, |v| v.len()), reason: "entry point".into() }).collect();
    let mut by_imports: Vec<(&str, usize)> = rev.iter().filter(|(p, _)| ix.files.get(**p).is_some_and(|n| !n.is_test)).map(|(p, v)| (*p, v.len())).filter(|(_, c)| *c >= 2).collect();
    by_imports.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    for (p, c) in by_imports.into_iter().take(8) {
        if !key_files.iter().any(|k| k.path == p) {
            key_files.push(KeyFile { path: p.to_string(), imported_by: c, reason: format!("imported by {c} files") });
        }
    }

    let mut ext: BTreeMap<&str, usize> = BTreeMap::new();
    for n in ix.files.values() {
        for e in &n.external {
            *ext.entry(e.as_str()).or_default() += 1;
        }
    }
    let mut external_deps: Vec<(String, usize)> = ext.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    external_deps.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    external_deps.truncate(15);

    Overview {
        project_type,
        languages,
        entry_points,
        commands,
        modules,
        key_files,
        external_deps,
        files: ix.files.len(),
        lines: ix.files.values().map(|n| n.lines).sum(),
        symbols: ix.files.values().map(|n| n.symbols.len()).sum(),
        unresolved_imports: ix.files.values().map(|n| n.unresolved.len()).sum(),
        tests: ix.files.values().filter(|n| n.is_test).count(),
        indexed_at: ix.indexed_at.clone(),
        truncated: ix.truncated,
        provenance: PROVENANCE,
    }
}

impl Overview {
    /// Compact text for a model prompt.
    pub fn render(&self, max_chars: usize) -> String {
        let mut s = format!("PROJECT BRAIN (verified by file scan at {}; {})\n", &self.indexed_at[..19.min(self.indexed_at.len())], if self.truncated { "partial: the project has more source files than are indexed" } else { "complete for supported languages" });
        s.push_str(&format!("Type: {}. {} source files, {} lines, {} symbols, {} test files.\n", self.project_type.join(" + "), self.files, self.lines, self.symbols, self.tests));
        if !self.entry_points.is_empty() {
            s.push_str(&format!("Entry points: {}\n", self.entry_points.join(", ")));
        }
        for c in &self.commands {
            s.push_str(&format!("{}: `{}`\n", c.purpose, c.command));
        }
        if !self.modules.is_empty() {
            s.push_str("Modules: ");
            s.push_str(&self.modules.iter().take(10).map(|m| format!("{} ({} files)", m.path, m.files)).collect::<Vec<_>>().join(", "));
            s.push('\n');
        }
        if !self.key_files.is_empty() {
            s.push_str("Key files: ");
            s.push_str(&self.key_files.iter().map(|k| format!("{} [{}]", k.path, k.reason)).collect::<Vec<_>>().join("; "));
            s.push('\n');
        }
        s.push_str("Use the project_brain tool to find symbols, see what a change affects, or recall history.\n");
        s.chars().take(max_chars).collect()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    pub kind: String,
    pub name: String,
    pub line: usize,
    pub score: f32,
}

fn tokens(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in q.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if word.len() < 2 {
            continue;
        }
        out.push(word.to_lowercase());
        // camelCase / PascalCase parts: `UserProfileCard` → user, profile, card
        let mut cur = String::new();
        for ch in word.chars() {
            if ch.is_uppercase() && !cur.is_empty() && cur.chars().last().is_some_and(|c| c.is_lowercase()) {
                if cur.len() >= 3 {
                    out.push(cur.to_lowercase());
                }
                cur.clear();
            }
            cur.push(ch);
        }
        if cur.len() >= 3 && cur.to_lowercase() != word.to_lowercase() {
            out.push(cur.to_lowercase());
        }
    }
    out.sort();
    out.dedup();
    out.retain(|t| !matches!(t.as_str(), "the" | "and" | "for" | "how" | "where" | "what" | "does" | "with" | "this" | "that" | "from" | "into" | "file" | "code"));
    out
}

/// Ranked symbol and path hits for a question.
pub fn search(ix: &BrainIndex, query: &str, limit: usize) -> Vec<Hit> {
    let toks = tokens(query);
    if toks.is_empty() {
        return vec![];
    }
    let rev = dependents(ix);
    let mut hits = Vec::new();
    for (path, node) in &ix.files {
        let lp = path.to_lowercase();
        let popularity = (1.0 + rev.get(path.as_str()).map_or(0, |v| v.len()) as f32).ln() * 0.5;
        let test_penalty = if node.is_test { -1.0 } else { 0.0 };
        let path_score: f32 = toks.iter().filter(|t| lp.contains(t.as_str())).count() as f32 * 2.0;
        if path_score > 0.0 {
            hits.push(Hit { path: path.clone(), kind: "file".into(), name: path.rsplit('/').next().unwrap_or(path).into(), line: 1, score: path_score + popularity + test_penalty });
        }
        for s in &node.symbols {
            let ls = s.name.to_lowercase();
            let sc: f32 = toks.iter().map(|t| if ls == *t { 10.0 } else if ls.contains(t.as_str()) { 4.0 } else { 0.0 }).sum();
            if sc > 0.0 {
                hits.push(Hit { path: path.clone(), kind: s.kind.clone(), name: s.name.clone(), line: s.line, score: sc + path_score * 0.5 + popularity + test_penalty });
            }
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.path.cmp(&b.path)).then(a.line.cmp(&b.line)));
    hits.truncate(limit);
    hits
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Impacted {
    pub path: String,
    pub depth: usize,
    /// The file it reaches the changed file through (direct importers: the changed file itself).
    pub via: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    pub changed: Vec<String>,
    /// Paths that are not in the index (new, unsupported language, or ignored).
    pub unknown: Vec<String>,
    pub affected: Vec<Impacted>,
    /// Test files among the affected ones, plus tests named after a changed file.
    pub tests: Vec<String>,
    pub truncated: bool,
    pub note: &'static str,
}

/// Which files import (directly or through others) the changed files. Reverse reachability over the
/// scanned import graph — a lower bound: dynamic imports, framework conventions and runtime wiring
/// are invisible to it.
pub fn impact(ix: &BrainIndex, changed: &[String], max_depth: usize, max_results: usize) -> Impact {
    let rev = dependents(ix);
    let mut out = Impact { note: "Import-graph reachability from a pattern scan: a lower bound on what a change can affect (runtime wiring, dynamic imports and shared data are not visible).", ..Default::default() };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<(&str, usize, &str)> = VecDeque::new();
    for c in changed {
        let c = c.trim_start_matches("./");
        match ix.files.get_key_value(c) {
            Some((k, _)) => {
                out.changed.push(k.clone());
                seen.insert(k.as_str());
                queue.push_back((k.as_str(), 0, k.as_str()));
            }
            None => out.unknown.push(c.to_string()),
        }
    }
    while let Some((file, depth, via)) = queue.pop_front() {
        if depth > 0 {
            if out.affected.len() >= max_results {
                out.truncated = true;
                break;
            }
            out.affected.push(Impacted { path: file.to_string(), depth, via: via.to_string() });
        }
        if depth >= max_depth {
            continue;
        }
        for d in rev.get(file).into_iter().flatten() {
            if seen.insert(d) {
                queue.push_back((d, depth + 1, file));
            }
        }
    }
    let mut tests: BTreeSet<String> = out.affected.iter().filter(|a| ix.files.get(&a.path).is_some_and(|n| n.is_test)).map(|a| a.path.clone()).collect();
    for c in &out.changed {
        let stem = c.rsplit('/').next().unwrap_or(c).split('.').next().unwrap_or("").to_lowercase();
        if stem.len() >= 3 {
            tests.extend(ix.files.iter().filter(|(p, n)| n.is_test && p.to_lowercase().contains(&stem)).map(|(p, _)| p.clone()).take(5));
        }
    }
    out.tests = tests.into_iter().take(15).collect();
    out
}

/// Statements in recorded knowledge that no longer match the code.
pub fn stale_reasons(ix: &BrainIndex, text: &str, source: &str, updated_at: &str, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
    static PATHISH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b((?:[\w.-]+/)+[\w.-]+\.(?:tsx?|jsx?|rs|py|mjs|cjs))\b").unwrap());
    let mut why = Vec::new();
    let missing: Vec<String> = PATHISH.captures_iter(text).map(|c| c[1].to_string()).filter(|p| !ix.files.contains_key(p.trim_start_matches("./"))).collect::<BTreeSet<_>>().into_iter().take(3).collect();
    if !missing.is_empty() {
        why.push(format!("mentions files that no longer exist: {}", missing.join(", ")));
    }
    if source != "user" && source != "repo" {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(updated_at) {
            let days = now.signed_duration_since(t).num_days();
            if days >= 30 {
                why.push(format!("an agent inference last updated {days} days ago"));
            }
        }
    }
    why
}

// ------------------------------------------------------------------------------------ storage

pub fn storage_key(project_id: &str) -> String {
    format!("brain:{project_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        ws.write_text("package.json", r#"{"name":"demo","main":"src/main.tsx","scripts":{"dev":"vite","build":"tsc && vite build","test":"vitest run","lint":"eslint ."},"dependencies":{"react":"^19"},"devDependencies":{"vite":"^7","vitest":"^3"}}"#).unwrap();
        ws.write_text("src/main.tsx", "import React from 'react';\nimport App from './App';\nimport './index.css';\nrender(<App />);\n").unwrap();
        ws.write_text("src/App.tsx", "import { useState } from 'react';\nimport {\n  formatPrice,\n  parsePrice,\n} from './lib/format';\nimport Card from './components/Card';\nexport default function App() {\n  return <Card />;\n}\n").unwrap();
        ws.write_text("src/index.css", "body { margin: 0 }\n").unwrap();
        ws.write_text("src/components/Card.tsx", "import { formatPrice } from '../lib/format';\nexport const Card = () => <div>{formatPrice(1)}</div>;\nexport default Card;\n").unwrap();
        ws.write_text("src/lib/format.ts", "export function formatPrice(n: number) { return `$${n}`; }\nexport function parsePrice(s: string) { return Number(s); }\nexport interface Price { value: number }\n").unwrap();
        ws.write_text("src/lib/format.test.ts", "import { formatPrice } from './format';\ntest('x', () => expect(formatPrice(1)).toBe('$1'));\n").unwrap();
        ws.write_text(".env", "SECRET=1\n").unwrap();
        (dir, ws)
    }

    #[test]
    fn scans_symbols_imports_and_resolves_the_graph() {
        let (_d, ws) = project();
        let (ix, st) = build(&ws, None).unwrap();
        assert_eq!((st.scanned, st.reused, st.removed), (5, 0, 0));
        let app = &ix.files["src/App.tsx"];
        assert_eq!(app.imports, ["src/components/Card.tsx", "src/lib/format.ts"], "multi-line import and default import resolved");
        assert_eq!(app.external, ["react"]);
        assert!(app.symbols.iter().any(|s| s.name == "App" && s.kind == "function" && s.line == 7));
        let fmt = &ix.files["src/lib/format.ts"];
        assert_eq!(fmt.symbols.iter().map(|s| (s.name.as_str(), s.kind.as_str(), s.line)).collect::<Vec<_>>(), [("formatPrice", "function", 1), ("parsePrice", "function", 2), ("Price", "interface", 3)]);
        assert!(ix.files["src/main.tsx"].imports.contains(&"src/App.tsx".to_string()));
        assert!(ix.files["src/lib/format.test.ts"].is_test);
        assert!(!ix.files.contains_key(".env"), "secrets are never scanned");
        assert!(ix.files["src/main.tsx"].unresolved.is_empty(), "the css import resolves too");
    }

    #[test]
    fn overview_reports_type_entry_points_commands_and_key_files() {
        let (_d, ws) = project();
        let (ix, _) = build(&ws, None).unwrap();
        let o = overview(&ix);
        assert_eq!(o.project_type, ["React (Vite)"]);
        assert_eq!(o.entry_points, ["src/main.tsx"]);
        let cmd = |p: &str| o.commands.iter().find(|c| c.purpose == p).map(|c| c.command.as_str());
        assert_eq!((cmd("build"), cmd("test"), cmd("dev server")), (Some("npm run build"), Some("npm run test"), Some("npm run dev")));
        assert!(o.commands.iter().find(|c| c.purpose == "build").unwrap().source.contains("tsc && vite build"), "commands cite the manifest line they came from");
        assert!(o.key_files.iter().any(|k| k.path == "src/lib/format.ts" && k.imported_by == 3));
        let text = o.render(2000);
        assert!(text.contains("verified by file scan") && text.contains("project_brain"));
        assert!(o.provenance.contains("not a compiler"));
    }

    #[test]
    fn reindexing_reads_only_what_changed_and_follows_new_and_deleted_files() {
        let (_d, ws) = project();
        let (ix1, _) = build(&ws, None).unwrap();
        let (ix2, st) = build(&ws, Some(&ix1)).unwrap();
        assert_eq!((st.scanned, st.reused), (0, 5), "nothing changed → nothing re-read");
        assert_eq!(ix2.files, ix1.files);

        ws.write_text("src/lib/extra.ts", "export const extra = 1;\n").unwrap();
        ws.write_text("src/lib/format.ts", "import { extra } from './extra';\nexport function formatPrice(n: number) { return `$${n}${extra}`; }\n").unwrap();
        let (ix3, st) = build(&ws, Some(&ix2)).unwrap();
        assert_eq!((st.scanned, st.reused), (2, 4));
        assert_eq!(ix3.files["src/lib/format.ts"].imports, ["src/lib/extra.ts"]);
        assert!(ix3.files["src/lib/format.ts"].symbols.iter().all(|s| s.name != "parsePrice"), "removed symbol is gone");

        ws.delete_file("src/components/Card.tsx").unwrap();
        let (ix4, st) = build(&ws, Some(&ix3)).unwrap();
        assert_eq!(st.removed, 1);
        assert!(!ix4.files["src/App.tsx"].imports.contains(&"src/components/Card.tsx".to_string()), "an unchanged importer loses its dangling edge");
    }

    #[test]
    fn a_file_created_later_resolves_imports_that_were_unresolved() {
        let (_d, ws) = project();
        ws.write_text("src/late.ts", "import { z } from './zed';\nexport const late = z;\n").unwrap();
        let (ix1, _) = build(&ws, None).unwrap();
        assert_eq!(ix1.files["src/late.ts"].unresolved, ["./zed"]);
        ws.write_text("src/zed.ts", "export const z = 1;\n").unwrap();
        let (ix2, _) = build(&ws, Some(&ix1)).unwrap();
        assert_eq!(ix2.files["src/late.ts"].imports, ["src/zed.ts"], "the unchanged importer now resolves");
        assert!(ix2.files["src/late.ts"].unresolved.is_empty());
    }

    #[test]
    fn impact_follows_importers_transitively_and_finds_tests() {
        let (_d, ws) = project();
        let (ix, _) = build(&ws, None).unwrap();
        let i = impact(&ix, &["src/lib/format.ts".into(), "src/nope.ts".into()], 4, 50);
        assert_eq!(i.unknown, ["src/nope.ts"]);
        let by_depth = |d: usize| i.affected.iter().filter(|a| a.depth == d).map(|a| a.path.as_str()).collect::<BTreeSet<_>>();
        assert_eq!(by_depth(1), BTreeSet::from(["src/App.tsx", "src/components/Card.tsx", "src/lib/format.test.ts"]));
        assert_eq!(by_depth(2), BTreeSet::from(["src/main.tsx"]), "main imports App");
        assert_eq!(i.tests, ["src/lib/format.test.ts"]);
        assert!(i.note.contains("lower bound"));
        let capped = impact(&ix, &["src/lib/format.ts".into()], 4, 2);
        assert!(capped.truncated && capped.affected.len() == 2);
    }

    #[test]
    fn search_ranks_exact_symbols_above_loose_matches_and_splits_camel_case() {
        let (_d, ws) = project();
        let (ix, _) = build(&ws, None).unwrap();
        let h = search(&ix, "where is formatPrice defined", 5);
        assert_eq!((h[0].name.as_str(), h[0].path.as_str(), h[0].line), ("formatPrice", "src/lib/format.ts", 1));
        let h = search(&ix, "ParsePrice", 3);
        assert_eq!(h[0].name, "parsePrice");
        assert!(search(&ix, "zzzz nothing", 5).is_empty());
        assert!(search(&ix, "the and for", 5).is_empty(), "stop words alone match nothing");
    }

    #[test]
    fn rust_and_python_projects_are_understood() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        ws.write_text("Cargo.toml", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n").unwrap();
        ws.write_text("src/main.rs", "mod util;\nuse crate::util::helper;\nuse serde::Serialize;\nfn main() { helper(); }\n").unwrap();
        ws.write_text("src/util.rs", "pub fn helper() {}\npub struct Thing;\nimpl Thing { pub fn new() -> Self { Thing } }\n").unwrap();
        ws.write_text("pkg/__init__.py", "").unwrap();
        ws.write_text("pkg/core.py", "import os\nfrom .util import run\nclass Engine:\n    pass\ndef start():\n    pass\n").unwrap();
        ws.write_text("pkg/util.py", "def run():\n    pass\n").unwrap();
        ws.write_text("tests/test_core.py", "from pkg.core import start\n").unwrap();
        let (ix, _) = build(&ws, None).unwrap();
        assert_eq!(ix.files["src/main.rs"].imports, ["src/util.rs"]);
        assert_eq!(ix.files["src/main.rs"].external, ["serde"]);
        assert!(ix.files["src/util.rs"].symbols.iter().any(|s| s.name == "helper") && ix.files["src/util.rs"].symbols.iter().any(|s| s.kind == "struct" && s.name == "Thing"));
        assert_eq!(ix.files["pkg/core.py"].imports, ["pkg/util.py"]);
        assert_eq!(ix.files["pkg/core.py"].external, ["os"]);
        assert_eq!(ix.files["tests/test_core.py"].imports, ["pkg/core.py"]);
        assert!(ix.files["tests/test_core.py"].is_test);
        let o = overview(&ix);
        assert!(o.project_type.contains(&"Rust crate".to_string()) && o.project_type.contains(&"Python".to_string()), "{:?}", o.project_type);
        assert!(o.commands.iter().any(|c| c.command == "cargo test") && o.commands.iter().any(|c| c.command == "pytest"));
        let i = impact(&ix, &["pkg/util.py".into()], 3, 20);
        assert!(i.affected.iter().any(|a| a.path == "pkg/core.py") && i.tests == ["tests/test_core.py"] || i.affected.iter().any(|a| a.path == "tests/test_core.py"));
    }

    #[test]
    fn recorded_knowledge_is_flagged_stale_when_it_no_longer_matches() {
        let (_d, ws) = project();
        let (ix, _) = build(&ws, None).unwrap();
        let now = chrono::Utc::now();
        let old = (now - chrono::Duration::days(90)).to_rfc3339();
        assert!(stale_reasons(&ix, "Prices are formatted in src/lib/format.ts", "agent", &now.to_rfc3339(), now).is_empty());
        let r = stale_reasons(&ix, "Auth lives in src/auth/session.ts", "agent", &now.to_rfc3339(), now);
        assert!(r[0].contains("src/auth/session.ts"));
        let r = stale_reasons(&ix, "We prefer small components", "agent", &old, now);
        assert!(r[0].contains("90 days"));
        assert!(stale_reasons(&ix, "We prefer small components", "user", &old, now).is_empty(), "the user's own statements do not expire by age");
    }

    #[test]
    fn a_hostile_import_cannot_escape_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path()).unwrap();
        ws.write_text("a.ts", "import x from '../../../etc/passwd';\nimport y from './b';\n").unwrap();
        ws.write_text("b.ts", "export default 1;\n").unwrap();
        let (ix, _) = build(&ws, None).unwrap();
        assert_eq!(ix.files["a.ts"].imports, ["b.ts"]);
        assert_eq!(ix.files["a.ts"].unresolved, ["../../../etc/passwd"], "recorded as unresolved, never read");
    }
}
