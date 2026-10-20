//! Tool-call normalisation and validation for the agent's text protocol.
//!
//! Models write tool calls in many dialects. Everything here turns those shapes into the
//! canonical `{name, args}` *before* anything runs, so a call that is merely spelled
//! differently never costs a model round-trip, and a call that is genuinely incomplete is
//! rejected with a short, precise error and one correct example:
//!
//! * call shapes — `{"name", "args"}`, `{"name", "arguments": {..} | "<json string>"}`,
//!   OpenAI `{"type": "function", "function": {"name", "arguments"}}`, `{"tool", "parameters"}`,
//!   `{"function": "name", "args"}`, `{"action", "action_input"}`, `{"tool_calls": [..]}`,
//!   arrays of calls, `functions.name` prefixes, common tool-name synonyms, and a bare object
//!   whose keys identify the tool (`{"path", "content"}` → `write_file`);
//! * arguments — one alias table ([`ARG_ALIASES`]) per tool and per array entry, plus
//!   shape fixes: `write_files.files` as an object map `{path: content}`, as entries keyed
//!   `file`/`filename`/`name`, as `[path, content]` pairs, as a JSON string, or a single
//!   `{path, content}`; `content` given as a JSON value or an array of lines; `edits` given as
//!   one object; a bare string for single-argument tools.

use serde_json::{json, Map, Value};

/// Argument aliases: (scope, canonical, aliases). The scope is a tool name, or `tool.field[]`
/// for the entries of an array argument. An alias is renamed only when the canonical key is
/// absent. This is the single place where argument spellings are accepted.
pub const ARG_ALIASES: &[(&str, &str, &[&str])] = &[
    ("list_files", "prefix", &["path", "dir", "folder", "directory"]),
    ("search", "query", &["pattern", "q", "text", "search", "term", "keyword"]),
    ("search", "prefix", &["path", "dir", "folder", "directory"]),
    ("read_file", "path", &["file", "file_path", "filePath", "filename", "file_name", "name"]),
    ("read_file", "paths", &["files", "file_paths", "filePaths", "filenames"]),
    ("read_file", "startLine", &["start_line", "start", "offset", "from", "line_start", "lineStart"]),
    ("read_file", "endLine", &["end_line", "end", "to", "line_end", "lineEnd"]),
    ("write_file", "path", &["file", "file_path", "filePath", "filename", "file_name", "fileName", "name", "target"]),
    ("write_file", "content", &["contents", "text", "code", "body", "source", "data", "file_content", "fileContent", "file_text"]),
    ("edit_file", "path", &["file", "file_path", "filePath", "filename", "file_name", "fileName", "target"]),
    ("edit_file", "old", &["old_string", "old_str", "oldString", "old_text", "oldText", "search", "find", "original", "before", "from"]),
    ("edit_file", "new", &["new_string", "new_str", "newString", "new_text", "newText", "replace", "replacement", "after", "to"]),
    ("edit_file", "replaceAll", &["replace_all", "all", "global"]),
    ("edit_file", "edits", &["changes", "replacements", "diffs", "operations", "patches"]),
    ("edit_file.edits[]", "old", &["old_string", "old_str", "oldString", "old_text", "oldText", "search", "find", "original", "before", "from"]),
    ("edit_file.edits[]", "new", &["new_string", "new_str", "newString", "new_text", "newText", "replace", "replacement", "after", "to"]),
    ("edit_file.edits[]", "replaceAll", &["replace_all", "all", "global"]),
    ("write_files", "files", &["file_list", "fileList", "writes", "items", "documents"]),
    ("write_files.files[]", "path", &["file", "filename", "file_name", "fileName", "name", "file_path", "filePath", "target"]),
    ("write_files.files[]", "content", &["contents", "code", "text", "body", "source", "data", "file_content", "fileContent"]),
    ("delete_file", "path", &["file", "file_path", "filePath", "filename", "file_name"]),
    ("git_diff", "path", &["file", "file_path", "filePath", "filename"]),
    ("add_dependency", "packages", &["names", "package", "name", "dependencies", "deps", "pkgs", "package_names"]),
    ("add_dependency", "dev", &["devDependency", "isDev", "save_dev", "saveDev", "development", "dev_dependency"]),
    ("run_check", "check", &["name", "kind", "type", "command", "script", "check_type"]),
    ("last_run_output", "check", &["name", "kind", "type", "op"]),
    ("remember", "kind", &["type", "category"]),
    ("remember", "content", &["text", "memory", "note", "fact"]),
    ("delegate", "role", &["agent", "subagent", "specialist"]),
    ("delegate", "task", &["prompt", "assignment", "question", "instructions", "request"]),
    ("todo", "items", &["todos", "list", "tasks", "steps"]),
    ("todo.items[]", "text", &["content", "title", "task", "description", "name"]),
    ("use_skill", "name", &["skill", "id", "skill_name"]),
    ("web_search", "query", &["q", "text", "search", "term"]),
    ("fetch_url", "url", &["link", "href", "uri", "address"]),
    ("read_reference", "id", &["reference", "ref", "reference_id"]),
    ("calculate", "expression", &["expr", "formula", "input", "query", "math", "calculation"]),
    ("data_summary", "id", &["file", "file_id", "fileId", "reference", "attachment"]),
    ("data_summary", "column", &["col", "field"]),
    ("data_query", "id", &["file", "file_id", "fileId", "reference", "attachment"]),
    ("data_query", "where", &["filter", "filters", "conditions"]),
    ("data_query", "group_by", &["groupBy", "group", "by"]),
    ("data_query", "metrics", &["aggregate", "aggregates", "aggregations", "agg"]),
    ("data_query", "sort_by", &["sortBy", "sort", "order_by", "orderBy"]),
    ("data_query", "columns", &["select", "fields"]),
    ("search_references", "query", &["q", "text", "search", "term"]),
];

/// Tool-name synonyms models use (after lower-casing and `-`/space → `_`).
const TOOL_ALIASES: &[(&str, &[&str])] = &[
    ("write_file", &["create_file", "createfile", "writefile", "save_file", "new_file", "write", "create", "file_write", "put_file", "overwrite_file"]),
    ("write_files", &["writefiles", "write_multiple_files", "create_files", "save_files", "write_many", "batch_write", "write_files_batch", "files_write"]),
    ("edit_file", &["editfile", "str_replace", "str_replace_editor", "str_replace_based_edit_tool", "replace_in_file", "apply_edit", "edit", "modify_file", "update_file", "search_replace", "patch_file"]),
    ("read_file", &["readfile", "read", "open_file", "view_file", "cat", "view", "get_file", "read_files"]),
    ("list_files", &["listfiles", "ls", "list_dir", "list_directory", "list", "tree"]),
    ("search", &["grep", "search_files", "find", "code_search", "search_code"]),
    ("delete_file", &["deletefile", "remove_file", "rm", "delete", "remove"]),
    ("add_dependency", &["add_dependencies", "install", "npm_install", "add_package", "add_packages", "install_package", "install_packages"]),
    ("run_check", &["runcheck", "check", "run_typecheck", "typecheck", "run_build", "build", "run_tests", "test", "run_test"]),
    ("preview_errors", &["get_preview_errors", "runtime_errors", "console_errors"]),
];

/// Single-argument tools: a bare string or array becomes this argument.
const PRIMARY: &[(&str, &str)] = &[
    ("read_file", "path"),
    ("list_files", "prefix"),
    ("search", "query"),
    ("delete_file", "path"),
    ("git_diff", "path"),
    ("run_check", "check"),
    ("last_run_output", "check"),
    ("web_search", "query"),
    ("fetch_url", "url"),
    ("use_skill", "name"),
    ("read_reference", "id"),
    ("calculate", "expression"),
    ("data_summary", "id"),
    ("data_query", "id"),
    ("search_references", "query"),
    ("add_dependency", "packages"),
    ("write_files", "files"),
    ("todo", "items"),
    ("edit_file", "edits"),
];

/// Canonical tool name for what the model wrote (`functions.write_file`, `Write-File`, `create_file`…).
pub fn canonical_tool(name: &str, known: &[&str]) -> String {
    let raw = name.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`');
    let raw = raw.rsplit(['.', ':', '/']).next().unwrap_or(raw);
    if known.contains(&raw) {
        return raw.to_string();
    }
    let mut n = String::new();
    for (i, c) in raw.chars().enumerate() {
        // camelCase → snake_case
        if c.is_ascii_uppercase() && i > 0 && !n.ends_with('_') {
            n.push('_');
        }
        n.push(if c == '-' || c == ' ' { '_' } else { c.to_ascii_lowercase() });
    }
    if known.contains(&n.as_str()) {
        return n;
    }
    let flat = n.replace('_', "");
    if let Some(k) = known.iter().find(|k| k.replace('_', "") == flat) {
        return k.to_string();
    }
    for (canon, aliases) in TOOL_ALIASES {
        if aliases.iter().any(|a| *a == n || a.replace('_', "") == flat) && known.contains(canon) {
            return canon.to_string();
        }
    }
    raw.to_string()
}

/// Infer the tool from the argument keys of a call that has no name (only unambiguous shapes).
pub fn infer_tool(args: &Value) -> Option<&'static str> {
    let o = args.as_object()?;
    let has = |keys: &[&str]| keys.iter().any(|k| o.contains_key(*k));
    let path = has(&["path", "file", "file_path", "filePath", "filename", "file_name"]);
    if has(&["files"]) && !has(&["paths"]) && o.get("files").is_some_and(|f| f.is_object() || f.as_array().is_some_and(|a| a.iter().any(|x| x.is_object() || x.is_array()))) {
        return Some("write_files");
    }
    if path && has(&["edits", "old", "old_string", "old_str", "oldString", "search"]) {
        return Some("edit_file");
    }
    if path && has(&["content", "contents", "code", "file_content", "fileContent"]) {
        return Some("write_file");
    }
    None
}

fn rename_aliases(scope: &str, o: &mut Map<String, Value>) {
    for (s, canon, aliases) in ARG_ALIASES {
        if *s != scope || o.contains_key(*canon) {
            continue;
        }
        if let Some(v) = aliases.iter().find_map(|a| o.remove(*a)) {
            o.insert(canon.to_string(), v);
        }
    }
}

/// File content as a string: JSON values are serialised (package.json written as an object),
/// arrays of lines joined.
fn content_string(v: Value) -> Value {
    match v {
        Value::String(_) => v,
        Value::Array(a) if a.iter().all(|x| x.is_string()) => Value::String(a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join("\n") + "\n"),
        Value::Object(_) | Value::Array(_) => Value::String(serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"),
        Value::Number(n) => Value::String(n.to_string()),
        Value::Bool(b) => Value::String(b.to_string()),
        Value::Null => Value::Null,
    }
}

/// A JSON string that holds a JSON array/object is parsed (models double-encode arguments).
fn decode_nested(v: Value) -> Value {
    match &v {
        Value::String(s) if matches!(s.trim_start().chars().next(), Some('[') | Some('{')) => crate::agent::lenient_json(s).unwrap_or(v),
        _ => v,
    }
}

fn normalize_entries(scope: &str, v: Value, entry: impl Fn(Map<String, Value>) -> Map<String, Value>) -> Value {
    match decode_nested(v) {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|x| match decode_nested(x) {
                    Value::Object(mut o) => {
                        rename_aliases(scope, &mut o);
                        Value::Object(entry(o))
                    }
                    other => other,
                })
                .collect(),
        ),
        Value::Object(mut o) => {
            rename_aliases(scope, &mut o);
            Value::Array(vec![Value::Object(entry(o))])
        }
        other => other,
    }
}

/// Normalise a tool's arguments to the canonical shape. Returns the (possibly corrected)
/// tool name too: `write_file` given a `files` list is really `write_files`. Idempotent.
pub fn normalize(name: &str, args: Value) -> (String, Value) {
    let mut name = name.to_string();
    let primary = PRIMARY.iter().find(|(t, _)| *t == name).map(|(_, k)| *k);
    let mut args = match decode_nested(args) {
        Value::Null => json!({}),
        Value::Object(o) => Value::Object(o),
        Value::String(s) if s.trim().is_empty() => json!({}),
        other => match primary {
            Some(k) => json!({ k: other }),
            None => other,
        },
    };
    let Some(o) = args.as_object_mut() else { return (name, args) };
    if name == "write_file" && (o.contains_key("files") || o.contains_key("file_list")) && !o.contains_key("content") {
        name = "write_files".into();
    }
    rename_aliases(&name, o);
    match name.as_str() {
        "write_file" => {
            if let Some(c) = o.remove("content") {
                o.insert("content".into(), content_string(c));
            }
        }
        "write_files" => {
            // a single {path, content} instead of a list
            if !o.contains_key("files") {
                let mut single = o.clone();
                rename_aliases("write_files.files[]", &mut single);
                if single.contains_key("path") && single.contains_key("content") {
                    o.clear();
                    o.insert("files".into(), json!([{ "path": single["path"], "content": single["content"] }]));
                }
            }
            if let Some(files) = o.remove("files") {
                let files = match decode_nested(files) {
                    // {path: content, ...} map (but not a single {path, content} entry)
                    Value::Object(m) if !m.keys().any(|k| ARG_ALIASES.iter().any(|(s, c, a)| *s == "write_files.files[]" && (k == c || a.contains(&k.as_str())))) => Value::Array(m.into_iter().map(|(p, c)| json!({ "path": p, "content": c })).collect()),
                    // [[path, content], ...]
                    Value::Array(a) if a.iter().all(|x| x.as_array().is_some_and(|p| p.len() == 2 && p[0].is_string())) => Value::Array(a.into_iter().map(|p| json!({ "path": p[0], "content": p[1] })).collect()),
                    other => other,
                };
                let files = normalize_entries("write_files.files[]", files, |mut e| {
                    if let Some(c) = e.remove("content") {
                        e.insert("content".into(), content_string(c));
                    }
                    e
                });
                o.insert("files".into(), files);
            }
        }
        "edit_file" => {
            if let Some(edits) = o.remove("edits") {
                let edits = normalize_entries("edit_file.edits[]", edits, |e| e);
                // the path inside the edit entries instead of at the top level
                if !o.contains_key("path") {
                    if let Some(p) = edits.as_array().and_then(|a| a.iter().find_map(|e| ["path", "file", "file_path", "filePath"].iter().find_map(|k| e.get(*k)).cloned())) {
                        o.insert("path".into(), p);
                    }
                }
                o.insert("edits".into(), edits);
            }
        }
        "fetch_url" => {
            if let Some(Value::Array(a)) = o.get("url") {
                if let Some(f) = a.iter().find_map(|v| v.as_str()).map(String::from) {
                    o.insert("url".into(), Value::String(f));
                }
            }
            if let Some(Value::String(u)) = o.get("url") {
                let cleaned = clean_url(u);
                o.insert("url".into(), Value::String(cleaned));
            }
        }
        "web_search" => {
            if let Some(Value::String(q)) = o.get("query") {
                let trimmed = q.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string();
                o.insert("query".into(), Value::String(trimmed));
            }
        }
        "read_file" => {
            if o.get("path").is_some_and(|p| p.is_array()) && !o.contains_key("paths") {
                let p = o.remove("path").unwrap_or_default();
                o.insert("paths".into(), p);
            }
            if let Some(p) = o.remove("paths") {
                o.insert("paths".into(), decode_nested(p));
            }
        }
        "todo" => {
            if let Some(items) = o.remove("items") {
                let items = match decode_nested(items) {
                    Value::Array(a) => Value::Array(a.into_iter().map(|x| if x.is_string() { json!({ "text": x }) } else { x }).collect()),
                    other => other,
                };
                o.insert("items".into(), normalize_entries("todo.items[]", items, |e| e));
            }
        }
        _ => {}
    }
    (name, args)
}

/// A protocol problem with a call's arguments: a stable code (for repeat detection) and the
/// message for the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ArgError {
    pub code: String,
    pub message: String,
}

fn need_str(o: &Map<String, Value>, key: &str, at: &str) -> Option<ArgError> {
    match o.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() || key == "content" || key == "new" => None,
        Some(Value::String(_)) => Some(ArgError { code: format!("{at}{key}:empty"), message: format!("\"{at}{key}\" is empty") }),
        Some(v) => Some(ArgError { code: format!("{at}{key}:type"), message: format!("\"{at}{key}\" must be a string, got {}", kind(v)) }),
        None => Some(ArgError { code: format!("{at}{key}:missing"), message: format!("\"{at}{key}\" is missing") }),
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Check required arguments of a normalised call. `inline_files`: the message carries file
/// blocks (or path-labelled code), which is what `write_files {}` saves.
pub fn validate(name: &str, args: &Value, inline_files: bool) -> Result<(), ArgError> {
    let empty = Map::new();
    let o = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        v => return Err(ArgError { code: "args:type".into(), message: format!("arguments must be a JSON object, got {}", kind(v)) }),
    };
    let first = |checks: &[(&str, &str)]| checks.iter().find_map(|(k, at)| need_str(o, k, at));
    let err = match name {
        "read_file" => match (o.get("path"), o.get("paths")) {
            (None, None) => Some(ArgError { code: "path:missing".into(), message: "\"path\" (or \"paths\") is missing".into() }),
            (_, Some(Value::Array(a))) if a.iter().any(|p| !p.is_string()) => Some(ArgError { code: "paths:type".into(), message: "\"paths\" must be a list of strings".into() }),
            (_, Some(v)) if !v.is_array() => Some(ArgError { code: "paths:type".into(), message: format!("\"paths\" must be a list of strings, got {}", kind(v)) }),
            (Some(_), _) => need_str(o, "path", ""),
            _ => None,
        },
        "write_file" => first(&[("path", ""), ("content", "")]),
        "delete_file" | "git_diff" => first(&[("path", "")]),
        "search" | "web_search" | "search_references" => first(&[("query", "")]),
        "project_brain" => match (o.get("query"), o.get("impact")) {
            (None, None) if matches!(o.get("callers"), Some(Value::String(s)) if !s.trim().is_empty()) => None,
            (None, None) => Some(ArgError { code: "brain:empty".into(), message: "give \"query\" (what to find), \"impact\" (a list of file paths) and/or \"callers\" (a symbol name)".into() }),
            (_, Some(v)) if !v.as_array().is_some_and(|a| a.iter().all(|p| p.is_string())) => Some(ArgError { code: "impact:type".into(), message: "\"impact\" must be a list of file path strings".into() }),
            (Some(Value::String(_)), _) | (None, _) => None,
            (Some(v), _) => Some(ArgError { code: "query:type".into(), message: format!("\"query\" must be a string, got {}", kind(v)) }),
        },
        "fetch_url" => first(&[("url", "")]),
        "use_skill" => first(&[("name", "")]),
        "read_reference" | "data_summary" | "data_query" => first(&[("id", "")]),
        "calculate" => first(&[("expression", "")]),
        "remember" => first(&[("content", "")]),
        "delegate" => first(&[("role", ""), ("task", "")]),
        "edit_file" => need_str(o, "path", "").or_else(|| match o.get("edits") {
            Some(Value::Array(edits)) if !edits.is_empty() => edits.iter().enumerate().find_map(|(i, e)| match e.as_object() {
                Some(e) => need_str(e, "old", &format!("edits[{i}].")).or_else(|| need_str(e, "new", &format!("edits[{i}]."))),
                None => Some(ArgError { code: "edits[]:type".into(), message: format!("\"edits[{i}]\" must be an object {{\"old\": …, \"new\": …}}") }),
            }),
            Some(Value::Array(_)) if o.contains_key("old") => first(&[("old", ""), ("new", "")]),
            Some(Value::Array(_)) => Some(ArgError { code: "edits:empty".into(), message: "\"edits\" is empty".into() }),
            Some(v) => Some(ArgError { code: "edits:type".into(), message: format!("\"edits\" must be a list, got {}", kind(v)) }),
            None => first(&[("old", ""), ("new", "")]),
        }),
        "write_files" => match o.get("files") {
            Some(Value::Array(files)) if !files.is_empty() => files.iter().enumerate().find_map(|(i, f)| match f.as_object() {
                Some(f) => need_str(f, "path", &format!("files[{i}].")).or_else(|| need_str(f, "content", &format!("files[{i}]."))),
                None => Some(ArgError { code: "files[]:type".into(), message: format!("\"files[{i}]\" must be an object {{\"path\": …, \"content\": …}}, got {}", kind(f)) }),
            }),
            _ if inline_files => None,
            Some(Value::Array(_)) | None => Some(ArgError { code: "files:missing".into(), message: "no files: \"files\" is missing or empty and the message has no <<<FILE: path>>> blocks".into() }),
            Some(v) => Some(ArgError { code: "files:type".into(), message: format!("\"files\" must be a list of {{\"path\", \"content\"}} objects, got {}", kind(v)) }),
        },
        "add_dependency" => match o.get("packages") {
            Some(Value::String(s)) if !s.trim().is_empty() => None,
            Some(Value::Array(a)) if !a.is_empty() && a.iter().all(|x| x.is_string()) => None,
            Some(v) => Some(ArgError { code: "packages:type".into(), message: format!("\"packages\" must be a list of package names, got {}", kind(v)) }),
            None => Some(ArgError { code: "packages:missing".into(), message: "\"packages\" is missing".into() }),
        },
        "todo" => match o.get("items") {
            Some(Value::Array(a)) if !a.is_empty() => a.iter().enumerate().find_map(|(i, it)| it.as_object().map_or(Some(ArgError { code: "items[]:type".into(), message: format!("\"items[{i}]\" must be an object {{\"text\", \"status\"}}") }), |it| need_str(it, "text", &format!("items[{i}].")))),
            Some(_) | None => Some(ArgError { code: "items:missing".into(), message: "\"items\" must be a non-empty list".into() }),
        },
        _ => None,
    };
    match err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// One minimal, correct call for a tool (shown with every argument error).
pub fn example(name: &str) -> String {
    let args = match name {
        "read_file" => r#"{"path": "src/App.tsx"}"#,
        "write_file" => r#"{"path": "src/components/Card.tsx", "content": "export default function Card() {\n  return <div className=\"card\" />;\n}\n"}"#,
        "edit_file" => r#"{"path": "src/App.tsx", "old": "<h1>Old title</h1>", "new": "<h1>New title</h1>"}"#,
        "write_files" => r#"{"files": [{"path": "src/a.ts", "content": "export const a = 1;\n"}, {"path": "src/b.ts", "content": "export const b = 2;\n"}]}"#,
        "delete_file" | "git_diff" => r#"{"path": "src/Old.tsx"}"#,
        "search" => r#"{"query": "useState"}"#,
        "web_search" => r#"{"query": "vite react typescript setup"}"#,
        "search_references" => r#"{"query": "pricing"}"#,
        "fetch_url" => r#"{"url": "https://react.dev/reference/react/useState"}"#,
        "use_skill" => r#"{"name": "debugging"}"#,
        "read_reference" => r#"{"id": "<reference id from search_references>"}"#,
        "calculate" => r#"{"expression": "(1250 * 0.19) + sqrt(144)"}"#,
        "data_summary" => r#"{"id": "<file id from FILES IN THIS CONVERSATION>", "column": "price"}"#,
        "data_query" => r#"{"id": "<file id from FILES IN THIS CONVERSATION>", "group_by": "category", "metrics": [{"column": "amount", "fn": "sum"}], "sort_by": "sum(amount)", "limit": 10}"#,
        "remember" => r#"{"kind": "convention", "content": "Components live in src/components."}"#,
        "delegate" => r#"{"role": "explorer", "task": "Find where routing is defined."}"#,
        "add_dependency" => r#"{"packages": ["date-fns"]}"#,
        "todo" => r#"{"items": [{"text": "Build the layout", "status": "active"}, {"text": "Add charts", "status": "pending"}]}"#,
        "run_check" => r#"{"check": "typecheck"}"#,
        "project_brain" => r#"{"query": "where is formatPrice defined", "impact": ["src/lib/format.ts"], "callers": "formatPrice"}"#,
        "project_facts" | "list_files" | "git_status" | "recent_changes" | "memory" | "capabilities" | "runtime_info" | "preview_errors" | "last_run_output" => "{}",
        _ => return example("read_file"),
    };
    format!("<tool>{{\"name\": \"{name}\", \"args\": {args}}}</tool>")
}

/// What to do instead when a form of call keeps failing.
pub fn recovery(name: &str) -> &'static str {
    match name {
        "write_file" | "write_files" => "Stop using JSON for file content. Write each complete file as a block in your message (no escaping needed); blocks are saved automatically, no tool call required:\n<<<FILE: src/components/Card.tsx>>>\n…complete file content…\n<<<END FILE>>>",
        "edit_file" => "Call read_file on the file to get its exact current text, then retry edit_file with that text as \"old\" — or rewrite the whole file as a <<<FILE: path>>> … <<<END FILE>>> block.",
        "read_file" => "Check the exact path with list_files first.",
        "add_dependency" => "Pass plain npm package names, e.g. {\"packages\": [\"date-fns\"]}, or skip the dependency and implement it yourself.",
        _ => "Use exactly the example format, or continue without this tool.",
    }
}

/// Signature of a malformed call that ignores content and positions, so the same mistake is
/// recognised when it is repeated with different file content.
pub fn signature(name: &str, code: &str) -> String {
    let code: String = code.chars().filter(|c| !c.is_ascii_digit()).collect();
    format!("{name}|{code}")
}

/// The tool a malformed call was probably meant for (for a targeted example).
pub fn guess_name(body: &str, known: &[&str]) -> Option<String> {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"(?:"(?:name|tool|function)"\s*:\s*"|<function=)([A-Za-z_][\w.\-]*)"#).unwrap());
    let raw = RE.captures(body)?.get(1)?.as_str().to_string();
    let c = canonical_tool(&raw, known);
    known.contains(&c.as_str()).then_some(c)
}

// ----------------------------------------------------------------------------- tool-named tags

/// Text with fenced code blocks and inline code blanked out (byte offsets preserved), so syntax shown
/// *as an example* in an answer is never read as a call and never stripped.
pub fn mask_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let blank = |s: &str| s.chars().map(|c| if c == '\n' { "\n".to_string() } else { " ".repeat(c.len_utf8()) }).collect::<String>();
    while let Some(i) = rest.find('`') {
        out.push_str(&rest[..i]);
        let after = &rest[i..];
        let ticks = after.chars().take_while(|c| *c == '`').count();
        let fence = "`".repeat(ticks);
        match after[ticks..].find(&fence) {
            Some(j) if ticks >= 3 || !after[ticks..ticks + j].contains('\n') => {
                let end = ticks + j + ticks;
                out.push_str(&blank(&after[..end]));
                rest = &after[end..];
            }
            _ => {
                out.push_str(&after[..ticks]);
                rest = &after[ticks..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A call written as an XML-ish tag that carries the tool's own name: `<web_search {"query": "x"}>`,
/// `<web_search query="x"/>`, `<fetch_url>https://…</fetch_url>`, `<web_search>{"query": "x"}</web_search>`.
#[derive(Debug, Clone, PartialEq)]
pub struct TagCall {
    pub start: usize,
    pub end: usize,
    pub name: String,
    /// `Err` when the arguments could not be read (the call is reported as malformed, never shown).
    pub args: std::result::Result<Value, String>,
}

fn parse_attrs(head: &str) -> Option<Map<String, Value>> {
    static ATTR: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"([A-Za-z_][\w-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))"#).unwrap());
    let mut o = Map::new();
    let mut covered = 0usize;
    for c in ATTR.captures_iter(head) {
        let v = c.get(2).or_else(|| c.get(3)).or_else(|| c.get(4)).map_or("", |m| m.as_str());
        covered += c.get(0).map_or(0, |m| m.as_str().len());
        let value = match v {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            n if !n.is_empty() && n.len() < 10 && n.chars().all(|ch| ch.is_ascii_digit()) => json!(n.parse::<u64>().unwrap_or(0)),
            s => Value::String(s.to_string()),
        };
        o.insert(c[1].to_string(), value);
    }
    (!o.is_empty() && covered * 2 >= head.trim().len()).then_some(o)
}

/// Arguments from what stands between the tool name and the closing `>` of its tag.
fn tag_args(head: &str, name: &str) -> Option<Value> {
    let h = head.trim().trim_end_matches('/').trim();
    if h.is_empty() {
        return Some(json!({}));
    }
    if h.starts_with('{') || h.starts_with('[') {
        let v = crate::agent::lenient_json(h).ok()?;
        return if v.is_object() { Some(v) } else { PRIMARY.iter().find(|(t, _)| *t == name).map(|(_, k)| json!({ *k: v })) };
    }
    if let Some(o) = parse_attrs(h) {
        return Some(Value::Object(o));
    }
    let primary = PRIMARY.iter().find(|(t, _)| *t == name).map(|(_, k)| *k)?;
    let bare = h.trim_matches(|c| c == '"' || c == '\'');
    (!bare.is_empty()).then(|| json!({ primary: bare }))
}

/// Length of the balanced JSON object/array that starts `s` (string- and escape-aware), if it closes.
fn balanced_json_len(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    let (mut in_str, mut esc) = (false, false);
    for (i, c) in s.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i + c.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Tool-named tags in `masked` (a copy of the reply with code and file blocks blanked out). The tag name may
/// be spelled loosely (`<websearch …>`, `<Web-Search …>`); a name made of one plain word (`<todo>`,
/// `<search>`) counts only when JSON follows it, because such words are also ordinary HTML and prose.
/// The arguments may be JSON with or without a closing `>` (`<web_search {"query": "x"}`), attributes,
/// or the text between an opening and a closing tag.
pub fn named_tag_calls(masked: &str, known: &[&str]) -> Vec<TagCall> {
    static OPEN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"<([A-Za-z][A-Za-z0-9_-]*)").unwrap());
    let mut out: Vec<TagCall> = Vec::new();
    let mut from = 0;
    while let Some(m) = OPEN.captures_at(masked, from) {
        let whole = m.get(0).unwrap();
        let start = whole.start();
        let head_start = whole.end();
        from = head_start;
        let canon = canonical_tool(&m[1], known);
        if !known.contains(&canon.as_str()) {
            continue;
        }
        let rest = &masked[head_start..];
        let lead = rest.len() - rest.trim_start().len();
        let body = rest.trim_start();
        let json_next = body.starts_with('{') || body.starts_with('[');
        // `<todo>{…}</todo>`: the JSON comes after the closing '>'
        let json_after_gt = body.strip_prefix('>').map(|b| b.trim_start()).is_some_and(|b| b.starts_with('{') || b.starts_with('['));
        let plain_word = !canon.contains('_');
        if plain_word && !(json_next || json_after_gt) {
            continue;
        }
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/' || c == '{' || c == '[') && !rest.is_empty() {
            continue; // `<web_searches…`, `<todo-list…`: a different word
        }
        // 1. JSON straight after the name, with or without a closing '>'
        if json_next {
            if let Some(len) = balanced_json_len(body) {
                if let Some(args) = tag_args(&body[..len], &canon) {
                    let mut end = head_start + lead + len;
                    let tail = &masked[end..];
                    let t = tail.trim_start();
                    if let Some(after) = t.strip_prefix("/>").or_else(|| t.strip_prefix('>')) {
                        end += tail.len() - after.len();
                        let close = format!("</{}>", &m[1]);
                        if let Some(c) = masked[end..].trim_start().strip_prefix(close.as_str()) {
                            end = masked.len() - c.len();
                        }
                    }
                    out.push(TagCall { start, end, name: canon, args: Ok(args) });
                    from = end.max(start + 1);
                    continue;
                }
            }
        }
        // 2. `<name>inner</name>` and attribute forms, up to the first '>' that makes the arguments readable
        let gts: Vec<usize> = masked[head_start..].match_indices('>').map(|(i, _)| head_start + i).take(60).collect();
        let found = gts.iter().find_map(|gt| tag_args(&masked[head_start..*gt], &canon).map(|a| (*gt, a)));
        let (end, args) = match found {
            Some((gt, mut args)) => {
                let self_closing = masked[head_start..gt].trim_end().ends_with('/');
                let mut end = gt + 1;
                if !self_closing {
                    let close = format!("</{}>", &m[1]);
                    if let Some(c) = masked[end..].find(&close) {
                        let inner = masked[end..end + c].trim();
                        if !inner.is_empty() && args.as_object().is_some_and(|o| o.is_empty()) {
                            match tag_args(inner, &canon) {
                                Some(a) => args = a,
                                None => {
                                    let stop = end + c + close.len();
                                    out.push(TagCall { start, end: stop, name: canon.clone(), args: Err(format!("the arguments of {canon} are not valid JSON")) });
                                    from = stop;
                                    continue;
                                }
                            }
                        }
                        end += c + close.len();
                    }
                }
                (end, Ok(args))
            }
            None => {
                // no readable arguments: swallow the tag up to the first '>' (or the rest of the line)
                let end = gts.first().map(|g| g + 1).unwrap_or_else(|| masked[head_start..].find('\n').map_or(masked.len(), |n| head_start + n));
                (end, Err(format!("the arguments of {canon} are not valid JSON")))
            }
        };
        out.push(TagCall { start, end, name: canon, args });
        from = end.max(start + 1);
    }
    out
}

/// The reply with every tool-named tag rewritten to the canonical `<tool>{…}</tool>` form (`None` when it has none).
pub fn rewrite_named_tags(text: &str, masked: &str, known: &[&str]) -> Option<String> {
    let calls = named_tag_calls(masked, known);
    if calls.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for c in calls {
        out.push_str(&text[at..c.start]);
        match &c.args {
            Ok(args) => out.push_str(&format!("<tool>{}</tool>", json!({ "name": c.name, "args": args }))),
            // an unreadable argument string is reported through the normal malformed-call path
            Err(_) => out.push_str(&format!("<tool>{{\"name\": {}, \"arguments\": \"{{\"}}</tool>", Value::String(c.name.clone()))),
        }
        at = c.end;
    }
    out.push_str(&text[at..]);
    Some(out)
}

static RESIDUE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?is)<\s*/?\s*(?:tool|tool_call|tool_result|function_call|function|invoke|parameter)\b").unwrap());

/// Raw tool-call syntax left in a user-facing answer (outside code the user is meant to see).
pub fn has_residue(text: &str, known: &[&str]) -> bool {
    let masked = mask_code(text);
    RESIDUE.is_match(&masked) || !named_tag_calls(&masked, known).is_empty()
}

/// An answer without protocol syntax: well-formed `<tool_result>` blocks and tool tags are removed, loose
/// tags dropped. Code the answer shows on purpose (fences, inline code) is left alone.
pub fn strip_residue(text: &str, known: &[&str]) -> String {
    static BLOCK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?is)<\s*(tool_result|tool_call|tool|function_call|invoke)\b.*?<\s*/\s*(?:tool_result|tool_call|tool|function_call|invoke)\s*>").unwrap());
    let masked = mask_code(text);
    let mut spans: Vec<(usize, usize)> = named_tag_calls(&masked, known).into_iter().map(|c| (c.start, c.end)).collect();
    spans.extend(BLOCK.find_iter(&masked).map(|m| (m.start(), m.end())));
    spans.sort();
    let mut out = String::new();
    let mut at = 0;
    for (s, e) in spans {
        if s < at {
            continue;
        }
        out.push_str(&text[at..s]);
        at = e;
    }
    out.push_str(&text[at..]);
    // loose open/close tags that remain
    let masked_out = mask_code(&out);
    let mut cleaned = String::new();
    let mut last = 0;
    for m in RESIDUE.find_iter(&masked_out) {
        if m.start() < last {
            continue;
        }
        let end = masked_out[m.end()..].find('>').map_or(masked_out.len(), |g| m.end() + g + 1);
        cleaned.push_str(&out[last..m.start()]);
        last = end;
    }
    cleaned.push_str(&out[last..]);
    let collapsed = regex::Regex::new(r"\n{3,}").unwrap().replace_all(&cleaned, "\n\n").into_owned();
    collapsed.trim().to_string()
}

/// A URL argument as the model may have written it: a Markdown link `[text](https://…)`, angle brackets,
/// quotes, trailing punctuation, or a bare domain. Returns the plain URL (without checking it is safe:
/// the fetcher does that).
pub fn clean_url(raw: &str) -> String {
    static MD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[^)]*)?\)").unwrap());
    let mut t = raw.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').trim();
    if let Some(c) = MD.captures(t) {
        t = c.get(1).map_or(t, |m| m.as_str());
    }
    let t = t.trim_start_matches('<').trim_end_matches('>');
    static EMBEDDED: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"https?://[^\s<>"']+"#).unwrap());
    let mut t = match EMBEDDED.find(t) {
        Some(m) if !t.starts_with("http") => m.as_str().to_string(),
        _ => t.split_whitespace().next().unwrap_or("").to_string(),
    };
    // trailing sentence punctuation or an unbalanced ")" from "(see https://x.y/z)"
    loop {
        let strip = match t.chars().last() {
            Some('.') | Some(',') | Some(';') | Some(':') | Some('!') | Some('?') | Some('"') | Some('\'') => true,
            Some(')') => t.matches(')').count() > t.matches('(').count(),
            _ => false,
        };
        if strip {
            t.pop();
        } else {
            break;
        }
    }
    let scheme_like = t.contains("://") || t.starts_with("mailto:") || t.starts_with("data:") || t.starts_with("javascript:");
    if !t.is_empty() && !scheme_like && t.contains('.') && !t.starts_with('/') {
        t = format!("https://{t}");
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &[&str] = &["read_file", "write_file", "write_files", "edit_file", "list_files", "search", "run_check", "add_dependency"];

    #[test]
    fn tool_names_are_canonicalised() {
        assert_eq!(canonical_tool("functions.write_file", KNOWN), "write_file");
        assert_eq!(canonical_tool("writeFiles", KNOWN), "write_files");
        assert_eq!(canonical_tool("create_file", KNOWN), "write_file");
        assert_eq!(canonical_tool("str_replace", KNOWN), "edit_file");
        assert_eq!(canonical_tool("Read-File", KNOWN), "read_file");
        assert_eq!(canonical_tool("run_shell", KNOWN), "run_shell");
    }

    #[test]
    fn write_files_shapes() {
        let want = json!({ "files": [{ "path": "src/a.ts", "content": "a" }, { "path": "src/b.ts", "content": "b" }] });
        // object map
        assert_eq!(normalize("write_files", json!({ "files": { "src/a.ts": "a", "src/b.ts": "b" } })).1, want);
        // entry aliases
        assert_eq!(normalize("write_files", json!({ "files": [{ "file": "src/a.ts", "contents": "a" }, { "filename": "src/b.ts", "code": "b" }] })).1, want);
        assert_eq!(normalize("write_files", json!({ "files": [{ "name": "src/a.ts", "text": "a" }, { "path": "src/b.ts", "content": "b" }] })).1, want);
        // pairs, JSON string, top-level array
        assert_eq!(normalize("write_files", json!({ "files": [["src/a.ts", "a"], ["src/b.ts", "b"]] })).1, want);
        assert_eq!(normalize("write_files", json!({ "files": want["files"].to_string() })).1, want);
        assert_eq!(normalize("write_files", want["files"].clone()).1, want);
        // single {path, content}, at the top level or as "files"
        let one = json!({ "files": [{ "path": "src/a.ts", "content": "a" }] });
        assert_eq!(normalize("write_files", json!({ "file_path": "src/a.ts", "content": "a" })).1, one);
        assert_eq!(normalize("write_files", json!({ "files": { "path": "src/a.ts", "content": "a" } })).1, one);
        // write_file given a list is write_files
        assert_eq!(normalize("write_file", json!({ "files": { "src/a.ts": "a", "src/b.ts": "b" } })), ("write_files".to_string(), want.clone()));
        // idempotent
        assert_eq!(normalize("write_files", want.clone()).1, want);
        // content as a JSON value
        let (_, a) = normalize("write_files", json!({ "files": { "package.json": { "name": "x" } } }));
        assert_eq!(a["files"][0]["content"], "{\n  \"name\": \"x\"\n}\n");
    }

    #[test]
    fn write_and_edit_aliases() {
        assert_eq!(normalize("write_file", json!({ "file_path": "a.ts", "contents": "x" })).1, json!({ "path": "a.ts", "content": "x" }));
        assert_eq!(normalize("write_file", json!({ "filename": "a.ts", "code": ["l1", "l2"] })).1, json!({ "path": "a.ts", "content": "l1\nl2\n" }));
        assert_eq!(normalize("edit_file", json!({ "file_path": "a", "old_string": "x", "new_string": "y", "replace_all": true })).1, json!({ "path": "a", "old": "x", "new": "y", "replaceAll": true }));
        assert_eq!(normalize("edit_file", json!({ "path": "a", "changes": { "search": "x", "replace": "y" } })).1, json!({ "path": "a", "edits": [{ "old": "x", "new": "y" }] }));
        assert_eq!(normalize("edit_file", json!({ "edits": [{ "file": "a", "old_str": "x", "new_str": "y" }] })).1["path"], "a");
        assert_eq!(normalize("read_file", json!("src/App.tsx")).1, json!({ "path": "src/App.tsx" }));
        assert_eq!(normalize("read_file", json!({ "path": ["a", "b"] })).1, json!({ "paths": ["a", "b"] }));
        assert_eq!(normalize("run_check", json!({ "type": "build" })).1, json!({ "check": "build" }));
        assert_eq!(normalize("todo", json!({ "todos": ["a"] })).1, json!({ "items": [{ "text": "a" }] }));
    }

    #[test]
    fn validation_names_the_field() {
        assert!(validate("write_file", &json!({ "path": "a", "content": "" }), false).is_ok());
        let e = validate("write_file", &json!({ "path": "a" }), false).unwrap_err();
        assert_eq!((e.code.as_str(), e.message.as_str()), ("content:missing", "\"content\" is missing"));
        let e = validate("write_files", &json!({ "files": [{ "path": "a", "content": "x" }, { "path": "b" }] }), false).unwrap_err();
        assert_eq!(e.message, "\"files[1].content\" is missing");
        assert!(validate("write_files", &json!({}), true).is_ok(), "file blocks in the message");
        assert_eq!(validate("write_files", &json!({}), false).unwrap_err().code, "files:missing");
        let e = validate("edit_file", &json!({ "path": "a", "edits": [{ "old": "x" }] }), false).unwrap_err();
        assert_eq!(e.message, "\"edits[0].new\" is missing");
        assert_eq!(validate("edit_file", &json!({ "path": "a" }), false).unwrap_err().message, "\"old\" is missing");
        assert_eq!(validate("read_file", &json!({ "path": 3 }), false).unwrap_err().message, "\"path\" must be a string, got a number");
        assert!(validate("project_facts", &json!({}), false).is_ok());
        assert!(example("write_files").contains("\"files\": [{\"path\""));
        assert_eq!(signature("write_files", "files[12].content:missing"), signature("write_files", "files[3].content:missing"));
    }

    #[test]
    fn infers_tools_from_keys() {
        assert_eq!(infer_tool(&json!({ "path": "a", "content": "x" })), Some("write_file"));
        assert_eq!(infer_tool(&json!({ "files": { "a": "x" } })), Some("write_files"));
        assert_eq!(infer_tool(&json!({ "file_path": "a", "old_string": "x", "new_string": "y" })), Some("edit_file"));
        assert_eq!(infer_tool(&json!({ "name": "Alice" })), None);
        assert_eq!(infer_tool(&json!({})), None);
        assert_eq!(guess_name(r#"{"name": "functions.write_files", "args": {"files": [oops"#, KNOWN).as_deref(), Some("write_files"));
    }

    const WEB: &[&str] = &["web_search", "fetch_url", "read_file", "todo"];

    fn rewritten(text: &str) -> Option<String> {
        rewrite_named_tags(text, &mask_code(text), WEB)
    }

    #[test]
    fn tool_named_tags_become_canonical_calls() {
        // the exact shape seen in the field
        let r = rewritten(r#"Let me look. <web_search {"query": "lufthansa connect ptfs"}>"#).unwrap();
        assert_eq!(r, r#"Let me look. <tool>{"name":"web_search","args":{"query":"lufthansa connect ptfs"}}</tool>"#);
        // attribute form, self-closing, quoted URL containing '>' and '&'
        let r = rewritten(r#"<fetch_url url="https://example.com/a?b=1&c=2"/>"#).unwrap();
        assert!(r.contains(r#""url":"https://example.com/a?b=1&c=2""#), "{r}");
        // inner text and inner JSON
        assert!(rewritten("<web_search>rust async traits</web_search>").unwrap().contains(r#""query":"rust async traits""#));
        assert!(rewritten(r#"<web_search>{"query": "a > b"}</web_search>"#).unwrap().contains(r#""query":"a > b""#));
        // a '>' inside the JSON string does not end the tag early
        assert!(rewritten(r#"<web_search {"query": "5 > 3 true?"}>"#).unwrap().contains(r#""query":"5 > 3 true?""#));
        // a URL value with Markdown-looking text and unescaped quotes is still one call
        let r = rewritten(r#"<fetch_url {"url": "[the site](https://lufthansa-connect-ptfs.netlify.app/)"}>"#).unwrap();
        assert!(r.contains("lufthansa-connect-ptfs.netlify.app"), "{r}");
        // examples shown in code are left alone, prose that merely mentions a tool is not a call
        assert!(rewritten("Use `<web_search {\"query\": \"x\"}>` like this").is_none());
        assert!(rewritten("```\n<web_search query=\"x\"/>\n```").is_none());
        assert!(rewritten("The web_search tool is great.").is_none());
        // unreadable arguments become a malformed call, never text
        let r = rewritten(r#"<web_search {"query": }>"#).unwrap();
        assert!(r.contains("\"arguments\""), "{r}");
    }

    #[test]
    fn dialects_seen_in_research_are_calls_not_text() {
        // exactly what a research answer showed: no underscore, no closing '>', several calls on one line, a <todo> block
        let text = "I'll research.\n\n<todo>\n{\"items\": [{\"text\": \"Search\", \"status\": \"active\"}]}\n</todo>\n\n<websearch {\"query\": \"quantum physics definition\"} <websearch {\"query\": \"quantum mechanics textbook\"}\n<web_search {\"query\": \"quantum site:.edu OR site:.gov\"}";
        let r = rewritten(text).unwrap();
        assert_eq!(r.matches("<tool>").count(), 4, "{r}");
        assert!(r.contains(r#""name":"todo","args":{"items""#) && r.contains(r#""name":"web_search","args":{"query":"quantum physics definition"}"#) && r.contains("quantum mechanics textbook") && r.contains("site:.edu OR site:.gov"), "{r}");
        assert!(!r.contains("<websearch") && !r.contains("<todo>"), "{r}");
        // plain words are ordinary prose/HTML unless JSON follows
        assert!(rewritten("Use the <todo> list and <search> element in HTML.").is_none());
        assert!(rewritten("<todo>buy milk</todo>").is_none());
        // a different word that merely starts with a tool name
        assert!(rewritten(r#"<web_searches {"query": "x"}>"#).is_none());
        // loose spellings
        assert!(rewritten(r#"<Web-Search {"query": "x"}>"#).unwrap().contains(r#""name":"web_search""#));
        // residue detection and removal cover these too
        assert!(has_residue(text, WEB));
        assert_eq!(strip_residue("Hello <websearch {\"query\": \"x\"} there", WEB), "Hello  there");
    }

    #[test]
    fn leftover_syntax_is_detected_and_removed() {
        assert!(has_residue("Result: <tool_result name=\"x\">hi</tool_result>", WEB));
        assert!(has_residue("Searching <web_search query=\"x\"> now", WEB));
        assert!(!has_residue("Use `<tool>` blocks; also ```\n<tool>{}</tool>\n``` is shown code", WEB));
        let cleaned = strip_residue("Here you go.\n<tool_result name=\"web_search\" ok=\"true\">secret data</tool_result>\nDone <web_search {\"query\": \"x\"}>", WEB);
        assert_eq!(cleaned, "Here you go.\n\nDone");
        assert_eq!(strip_residue("plain `<tool>` example", WEB), "plain `<tool>` example");
    }

    #[test]
    fn urls_are_cleaned_before_fetching() {
        assert_eq!(clean_url("https://a.example/x"), "https://a.example/x");
        assert_eq!(clean_url("[docs](https://a.example/x)"), "https://a.example/x");
        assert_eq!(clean_url("<https://a.example/x>"), "https://a.example/x");
        assert_eq!(clean_url("\"https://a.example/x\"."), "https://a.example/x");
        assert_eq!(clean_url("(see https://a.example/x)"), "https://a.example/x");
        assert_eq!(clean_url("https://en.wikipedia.org/wiki/Rust_(programming_language)"), "https://en.wikipedia.org/wiki/Rust_(programming_language)");
        assert_eq!(clean_url("a.example/page"), "https://a.example/page");
        assert_eq!(clean_url("file:///etc/passwd"), "file:///etc/passwd", "unsafe schemes are not rewritten; the fetcher refuses them");
        let (_, a) = normalize("fetch_url", json!({ "link": "[x](https://a.example/y)" }));
        assert_eq!(a, json!({ "url": "https://a.example/y" }));
        let (_, a) = normalize("fetch_url", json!({ "url": ["https://a.example/1", "https://a.example/2"] }));
        assert_eq!(a, json!({ "url": "https://a.example/1" }));
        let (_, a) = normalize("web_search", json!("\"cats\""));
        assert_eq!(a, json!({ "query": "cats" }));
    }
}
