//! Streaming parser for the Genius file-block protocol.
//!
//! The model emits file operations as plain text, which works with every chat model
//! (no dependency on function calling) and can be parsed incrementally while streaming:
//!
//! ```text
//! <<<FILE: src/App.tsx>>>
//! ...complete file content...
//! <<<END FILE>>>
//! <<<DELETE: src/Old.tsx>>>
//! ```
//!
//! Anything outside blocks is the model's explanation. Markdown fences that some models
//! wrap around file content are stripped. A block still open when the stream ends is
//! reported as incomplete and is never applied.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlockEvent {
    Text { line: String },
    FileStart { path: String },
    FileDelta { path: String, text: String },
    FileEnd { path: String, content: String, complete: bool },
    Delete { path: String },
}

#[derive(Default)]
pub struct BlockParser {
    partial: String,
    current: Option<OpenBlock>,
}

struct OpenBlock {
    path: String,
    content: String,
    lines_seen: usize,
    held_fence: Option<String>,
}

pub(crate) fn parse_marker(line: &str) -> Option<(&'static str, String)> {
    let t = line.trim();
    if !(t.starts_with("<<<") && t.ends_with(">>>")) {
        return None;
    }
    let inner = t[3..t.len() - 3].trim();
    let upper = inner.to_ascii_uppercase();
    if upper == "END FILE" || upper == "END" || upper == "/FILE" {
        return Some(("end", String::new()));
    }
    for (kw, kind) in [("FILE", "file"), ("DELETE", "delete")] {
        if upper.starts_with(kw) {
            let rest = inner[kw.len()..].trim_start_matches([':', ' ', '=']).trim();
            let path = rest.trim_matches(|c| c == '"' || c == '\'' || c == '`').trim().to_string();
            if !path.is_empty() {
                return Some((kind, path));
            }
        }
    }
    None
}

fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

impl BlockParser {
    pub fn push(&mut self, chunk: &str) -> Vec<BlockEvent> {
        self.partial.push_str(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=pos).collect();
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            self.line(line, &mut out);
        }
        out
    }

    pub fn finish(&mut self) -> Vec<BlockEvent> {
        let mut out = Vec::new();
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            self.line(line, &mut out);
        }
        if let Some(b) = self.current.take() {
            out.push(BlockEvent::FileEnd { path: b.path, content: b.content, complete: false });
        }
        out
    }

    fn line(&mut self, line: String, out: &mut Vec<BlockEvent>) {
        if let Some((kind, path)) = parse_marker(&line) {
            match kind {
                "end" => {
                    if let Some(b) = self.current.take() {
                        // a trailing fence held back is dropped here
                        out.push(BlockEvent::FileEnd { path: b.path, content: b.content, complete: true });
                    }
                }
                "file" => {
                    if let Some(b) = self.current.take() {
                        out.push(BlockEvent::FileEnd { path: b.path, content: b.content, complete: false });
                    }
                    out.push(BlockEvent::FileStart { path: path.clone() });
                    self.current = Some(OpenBlock { path, content: String::new(), lines_seen: 0, held_fence: None });
                }
                "delete" => {
                    if let Some(b) = self.current.take() {
                        out.push(BlockEvent::FileEnd { path: b.path, content: b.content, complete: false });
                    }
                    out.push(BlockEvent::Delete { path });
                }
                _ => {}
            }
            return;
        }
        match self.current.as_mut() {
            Some(b) => {
                b.lines_seen += 1;
                if b.lines_seen == 1 && is_fence(&line) {
                    return; // opening fence
                }
                if is_fence(&line) && line.trim() == "```" {
                    // could be the closing fence; hold until we see more content
                    if let Some(prev) = b.held_fence.replace(line) {
                        emit_line(b, prev, out);
                    }
                    return;
                }
                if let Some(prev) = b.held_fence.take() {
                    emit_line(b, prev, out);
                }
                emit_line(b, line, out);
            }
            None => {
                if !is_fence(&line) {
                    out.push(BlockEvent::Text { line });
                }
            }
        }
    }
}

fn emit_line(b: &mut OpenBlock, line: String, out: &mut Vec<BlockEvent>) {
    let text = format!("{line}\n");
    b.content.push_str(&text);
    out.push(BlockEvent::FileDelta { path: b.path.clone(), text });
}

/// Parse a complete response at once.
pub fn parse_all(text: &str) -> Vec<BlockEvent> {
    let mut p = BlockParser::default();
    let mut ev = p.push(text);
    ev.extend(p.finish());
    ev
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedOutput {
    pub files: Vec<(String, String)>,
    pub deletes: Vec<String>,
    pub incomplete: Vec<String>,
    pub explanation: String,
}

pub fn collect(events: &[BlockEvent]) -> ParsedOutput {
    let mut o = ParsedOutput::default();
    let mut text = Vec::new();
    for e in events {
        match e {
            BlockEvent::Text { line } => text.push(line.as_str()),
            BlockEvent::FileEnd { path, content, complete: true } => {
                o.files.retain(|(p, _)| p != path);
                o.files.push((path.clone(), content.clone()));
            }
            BlockEvent::FileEnd { path, complete: false, .. } => o.incomplete.push(path.clone()),
            BlockEvent::Delete { path } => o.deletes.push(path.clone()),
            _ => {}
        }
    }
    o.explanation = text.join("\n").trim().to_string();
    o
}

/// Files a model wrote as Markdown code fences labelled with a path instead of file blocks:
/// a path in the fence info (```` ```tsx src/App.tsx ````, `title="…"`), on the line just
/// above the fence (`**src/App.tsx**`, `### src/App.tsx:`), or as a first-line comment.
/// Fences that look like excerpts ("// ... rest unchanged") are reported in `skipped`
/// rather than returned, because applying them would truncate the file.
#[derive(Debug, Default, PartialEq)]
pub struct FencedOutput {
    pub files: Vec<(String, String)>,
    pub skipped: Vec<String>,
}

pub fn fenced_files(text: &str) -> FencedOutput {
    let mut out = FencedOutput::default();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim_start();
        if !t.starts_with("```") {
            i += 1;
            continue;
        }
        let fence_len = t.chars().take_while(|c| *c == '`').count();
        let info = t[fence_len..].trim();
        let end = (i + 1..lines.len()).find(|&j| {
            let l = lines[j].trim();
            l.len() >= fence_len && l.chars().all(|c| c == '`')
        });
        let Some(end) = end else { break };
        let mut body: Vec<&str> = lines[i + 1..end].to_vec();
        let mut path = path_in_info(info).or_else(|| (1..=2).filter(|d| i >= *d).map(|d| lines[i - d]).find(|l| !l.trim().is_empty()).and_then(path_in_heading));
        if path.is_none() {
            if let Some(p) = body.first().and_then(|l| path_in_comment(l)) {
                path = Some(p);
                if path.as_deref().is_some_and(|p| p.ends_with(".json")) {
                    body.remove(0);
                }
            }
        }
        if let Some(p) = path {
            let content = format!("{}\n", body.join("\n"));
            if looks_like_excerpt(&content) {
                out.skipped.push(p);
            } else {
                out.files.retain(|(q, _)| q != &p);
                out.files.push((p, content));
            }
        }
        i = end + 1;
    }
    out
}

fn is_path(s: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^(\./)?[A-Za-z0-9_@.\-\[\]]+(/[A-Za-z0-9_@.\-\[\]]+)*\.[A-Za-z0-9]{1,8}$").unwrap());
    let s = s.strip_prefix("./").unwrap_or(s);
    RE.is_match(s) && !s.contains("..") && !s.starts_with('.')
}

fn clean_path(s: &str) -> Option<String> {
    let s = s.trim().trim_matches(|c: char| matches!(c, '`' | '"' | '\'' | '*' | ':' | '(' | ')')).trim();
    is_path(s).then(|| s.trim_start_matches("./").to_string())
}

fn path_in_info(info: &str) -> Option<String> {
    for part in info.split_whitespace() {
        let v = part.split_once('=').map(|(k, v)| if ["title", "file", "filename", "path", "name"].contains(&k) { v } else { "" }).unwrap_or(part);
        let v = v.split_once(':').map(|(lang, rest)| if lang.chars().all(|c| c.is_ascii_alphanumeric()) && !rest.is_empty() { rest } else { v }).unwrap_or(v);
        if let Some(p) = clean_path(v) {
            return Some(p);
        }
    }
    None
}

fn path_in_heading(line: &str) -> Option<String> {
    let mut l = line.trim().trim_start_matches(['#', '-', '>', ' ']).trim();
    for prefix in ["File:", "file:", "Path:", "path:", "Filename:", "Create", "Update", "Updated", "New file:"] {
        if let Some(rest) = l.strip_prefix(prefix) {
            l = rest.trim();
        }
    }
    let l = l.trim_end_matches(':').trim();
    // the heading must be essentially just the path (optionally with a short parenthetical)
    let head = l.split(" (").next().unwrap_or(l).trim();
    if head.split_whitespace().count() != 1 {
        return None;
    }
    clean_path(head)
}

fn path_in_comment(line: &str) -> Option<String> {
    let t = line.trim();
    let inner = t
        .strip_prefix("//")
        .or_else(|| t.strip_prefix("/*").map(|r| r.trim_end_matches("*/")))
        .or_else(|| t.strip_prefix("<!--").map(|r| r.trim_end_matches("-->")))
        .or_else(|| t.strip_prefix('#'))?;
    let inner = inner.trim();
    let inner = inner.strip_prefix("File:").or_else(|| inner.strip_prefix("file:")).unwrap_or(inner).trim();
    if inner.split_whitespace().count() != 1 || !inner.contains('/') && !inner.contains('.') {
        return None;
    }
    clean_path(inner)
}

/// True when a fenced "file" is evidently an excerpt rather than complete content.
pub fn looks_like_excerpt(content: &str) -> bool {
    content.lines().any(|l| {
        let t = l.trim().to_lowercase();
        let comment = t.starts_with("//") || t.starts_with("/*") || t.starts_with("{/*") || t.starts_with('#') || t.starts_with("<!--");
        t == "..." || t == "…" || (comment && (t.contains("...") || t.contains('…')) && (t.contains("rest") || t.contains("existing") || t.contains("unchanged") || t.contains("same") || t.contains("other") || t.len() < 12))
            || (comment && (t.contains("rest of the") || t.contains("remains unchanged") || t.contains("existing code") || t.contains("keep the same")))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "I'll add a header.\n<<<FILE: src/Header.tsx>>>\n```tsx\nexport function Header() {\n  return <h1>Hi</h1>;\n}\n```\n<<<END FILE>>>\n<<<DELETE: src/Old.tsx>>>\nDone.\n";

    #[test]
    fn parses_complete_output() {
        let o = collect(&parse_all(SAMPLE));
        assert_eq!(o.files.len(), 1);
        assert_eq!(o.files[0].0, "src/Header.tsx");
        assert_eq!(o.files[0].1, "export function Header() {\n  return <h1>Hi</h1>;\n}\n");
        assert_eq!(o.deletes, vec!["src/Old.tsx"]);
        assert_eq!(o.explanation, "I'll add a header.\nDone.");
        assert!(o.incomplete.is_empty());
    }

    #[test]
    fn identical_result_when_streamed_char_by_char() {
        let mut p = BlockParser::default();
        let mut ev = Vec::new();
        for ch in SAMPLE.chars() {
            ev.extend(p.push(&ch.to_string()));
        }
        ev.extend(p.finish());
        assert_eq!(collect(&ev).files, collect(&parse_all(SAMPLE)).files);
        let deltas: String = ev.iter().filter_map(|e| match e { BlockEvent::FileDelta { text, .. } => Some(text.as_str()), _ => None }).collect();
        assert_eq!(deltas, "export function Header() {\n  return <h1>Hi</h1>;\n}\n");
    }

    #[test]
    fn inner_fences_are_preserved() {
        let s = "<<<FILE: README.md>>>\n# T\n```bash\nnpm i\n```\nend\n<<<END FILE>>>\n";
        let o = collect(&parse_all(s));
        assert_eq!(o.files[0].1, "# T\n```bash\nnpm i\n```\nend\n");
    }

    #[test]
    fn truncated_block_is_incomplete() {
        let o = collect(&parse_all("<<<FILE: src/a.ts>>>\nconst a = 1;\n"));
        assert!(o.files.is_empty());
        assert_eq!(o.incomplete, vec!["src/a.ts"]);
    }

    #[test]
    fn marker_variants() {
        let s = "<<<FILE: \"src/a.ts\">>>\nx\n<<<END>>>\n  <<<file src/b.ts>>>  \ny\n<<</FILE>>>\n";
        let o = collect(&parse_all(s));
        assert_eq!(o.files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["src/a.ts", "src/b.ts"]);
    }

    #[test]
    fn later_block_for_same_path_wins() {
        let s = "<<<FILE: a.ts>>>\n1\n<<<END FILE>>>\n<<<FILE: a.ts>>>\n2\n<<<END FILE>>>\n";
        let o = collect(&parse_all(s));
        assert_eq!(o.files, vec![("a.ts".to_string(), "2\n".to_string())]);
    }

    #[test]
    fn fenced_files_with_path_labels() {
        let s = "Here you go.\n\n**src/App.tsx**\n```tsx\nexport default function App() { return null; }\n```\n\n```css src/index.css\n:root { color: red; }\n```\n\n```json\n// package.json\n{\"a\": 1}\n```\n\n```bash\nnpm run dev\n```\n\n### `src/lib/x.ts`:\n```ts\n// src/lib/x.ts\nexport const x = 1;\n```\n";
        let o = fenced_files(s);
        let paths: Vec<&str> = o.files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(paths, vec!["src/App.tsx", "src/index.css", "package.json", "src/lib/x.ts"]);
        assert_eq!(o.files[2].1, "{\"a\": 1}\n", "path comment removed from JSON");
        assert!(o.files[3].1.starts_with("// src/lib/x.ts"), "comments kept in code");
    }

    #[test]
    fn fenced_excerpts_and_prose_are_not_files() {
        let o = fenced_files("Update src/App.tsx like this:\n```tsx src/App.tsx\nimport x;\n// ... rest of the file unchanged\n```\n");
        assert!(o.files.is_empty());
        assert_eq!(o.skipped, vec!["src/App.tsx"]);
        let o = fenced_files("Run this in the terminal now:\n```\nnpm i\n```\n");
        assert_eq!(o, FencedOutput::default());
        assert!(fenced_files("```ts ../etc/passwd.ts\nx\n```\n").files.is_empty());
        assert!(fenced_files("```tsx\nunclosed").files.is_empty());
    }
}
