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

/// Render files back into the block protocol (used to show the model its own earlier output).
pub fn render_files(files: &[(String, String)]) -> String {
    files.iter().map(|(p, c)| format!("<<<FILE: {p}>>>\n{}{}<<<END FILE>>>\n", c, if c.ends_with('\n') { "" } else { "\n" })).collect()
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

fn parse_marker(line: &str) -> Option<(&'static str, String)> {
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
}
