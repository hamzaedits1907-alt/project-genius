//! Hook contract check: does every caller destructure a hook the way the hook returns?
//!
//! A classic runtime crash in generated React apps: a custom `useDrag()` returns `{ ... }`
//! while a component writes `const [state, ref] = useDrag()` — the browser throws
//! "useDrag is not a function or its return value is not iterable". Vite's dev server does
//! not type-check, so the preview hits it at runtime (and `.js`/`.jsx` files or `any` types
//! hide it from `tsc` too). This module finds the shape a hook returns (object, array or
//! function) and the way each call site destructures it, for the project's own hooks and for
//! well-known library hooks, and reports mismatches.
//!
//! The analysis is deliberately conservative: when a shape cannot be determined with
//! confidence (e.g. `return value;`), nothing is reported.

use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Object,
    Array,
    Function,
}

impl Shape {
    fn describe(self) -> &'static str {
        match self {
            Shape::Object => "an object",
            Shape::Array => "an array",
            Shape::Function => "a function",
        }
    }
}

/// Return shapes of well-known library hooks (package, hook) → shape.
const KNOWN: &[(&str, &str, Shape)] = &[
    ("react", "useState", Shape::Array),
    ("react", "useReducer", Shape::Array),
    ("react", "useTransition", Shape::Array),
    ("react", "useActionState", Shape::Array),
    ("react", "useOptimistic", Shape::Array),
    ("react-dnd", "useDrag", Shape::Array),
    ("react-dnd", "useDrop", Shape::Array),
    ("@use-gesture/react", "useDrag", Shape::Function),
    ("@use-gesture/react", "useGesture", Shape::Function),
    ("@use-gesture/react", "usePinch", Shape::Function),
    ("@use-gesture/react", "useWheel", Shape::Function),
    ("@use-gesture/react", "useScroll", Shape::Function),
    ("@use-gesture/react", "useMove", Shape::Function),
    ("@use-gesture/react", "useHover", Shape::Function),
    ("@dnd-kit/core", "useDraggable", Shape::Object),
    ("@dnd-kit/core", "useDroppable", Shape::Object),
    ("@dnd-kit/sortable", "useSortable", Shape::Object),
    ("react-router-dom", "useSearchParams", Shape::Array),
    ("react-router", "useSearchParams", Shape::Array),
    ("react-router-dom", "useParams", Shape::Object),
    ("react-router", "useParams", Shape::Object),
    ("react-router-dom", "useLocation", Shape::Object),
    ("react-router", "useLocation", Shape::Object),
    ("react-hook-form", "useForm", Shape::Object),
    ("react-hook-form", "useFieldArray", Shape::Object),
    ("framer-motion", "useCycle", Shape::Array),
];

pub fn known_shape(package: &str, hook: &str) -> Option<Shape> {
    KNOWN.iter().find(|(p, h, _)| *p == package && *h == hook).map(|(_, _, s)| *s)
}

/// Blank comments (and, if `strings`, string/template literal contents) with spaces, keeping
/// byte offsets and newlines. Quotes only open a string when it closes on the same line, so
/// apostrophes in JSX text ("Don't") do not swallow code.
pub fn sanitize(src: &str, strings: bool) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in out.iter_mut().take(to).skip(from) {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..].iter().position(|&c| c == b'\n').map(|p| i + p).unwrap_or(b.len());
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(b.len());
                blank(&mut out, i, end);
                i = end;
            }
            q @ (b'\'' | b'"') => {
                let mut j = i + 1;
                let mut closed = None;
                while j < b.len() && b[j] != b'\n' {
                    if b[j] == b'\\' {
                        j += 2;
                        continue;
                    }
                    if b[j] == q {
                        closed = Some(j);
                        break;
                    }
                    j += 1;
                }
                match closed {
                    Some(end) => {
                        if strings {
                            blank(&mut out, i + 1, end);
                        }
                        i = end + 1;
                    }
                    None => i += 1,
                }
            }
            b'`' => {
                let mut j = i + 1;
                let mut expr_depth = 0usize;
                while j < b.len() {
                    match b[j] {
                        b'\\' => {
                            j += 2;
                            continue;
                        }
                        b'$' if expr_depth == 0 && b.get(j + 1) == Some(&b'{') => {
                            expr_depth = 1;
                            j += 2;
                            continue;
                        }
                        q @ (b'\'' | b'"') if expr_depth > 0 => {
                            // string inside `${ ... }`: skip it so its braces don't count
                            let close = b[j + 1..].iter().take_while(|&&c| c != b'\n').position(|&c| c == q);
                            if let Some(p) = close {
                                j += p + 2;
                                continue;
                            }
                        }
                        b'{' if expr_depth > 0 => expr_depth += 1,
                        b'}' if expr_depth > 0 => expr_depth -= 1,
                        b'`' if expr_depth == 0 => break,
                        _ => {}
                    }
                    j += 1;
                }
                let end = j.min(b.len());
                if strings {
                    blank(&mut out, i + 1, end);
                }
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| src.to_string())
}

/// Index of the bracket matching the opener at `open` (same kind), on sanitized text.
fn matching(s: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match s[open] {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let mut depth = 0i32;
    for (k, &ch) in s.iter().enumerate().skip(open) {
        if ch == o {
            depth += 1;
        } else if ch == c {
            depth -= 1;
            if depth == 0 {
                return Some(k);
            }
        }
    }
    None
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}

/// Shape implied by a declared TypeScript return type, if obvious.
fn declared_shape(s: &[u8], i: usize) -> Option<Shape> {
    let rest = std::str::from_utf8(&s[i..]).ok()?;
    let rest = rest.trim_start();
    if rest.starts_with('[') || rest.starts_with("readonly [") {
        Some(Shape::Array)
    } else if rest.starts_with('{') {
        Some(Shape::Object)
    } else if rest.starts_with('(') && rest.contains("=>") {
        // `(): (() => void)` style function types are rare; avoid guessing.
        None
    } else {
        None
    }
}

/// Skip a return-type annotation starting at `i` (just after `:`) until `stop` matches at depth 0.
fn skip_type(s: &[u8], mut i: usize, arrow: bool) -> Option<usize> {
    i = skip_ws(s, i);
    if i < s.len() && (s[i] == b'{' || s[i] == b'[') {
        i = matching(s, i)? + 1;
    }
    let mut depth = 0i32;
    while i < s.len() {
        let c = s[i];
        if arrow && c == b'=' && s.get(i + 1) == Some(&b'>') && depth == 0 {
            return Some(i);
        }
        match c {
            b'(' | b'[' | b'<' => depth += 1,
            b')' | b']' | b'>' => depth -= 1,
            b'{' if depth == 0 && !arrow => return Some(i),
            b'{' => {
                i = matching(s, i)? + 1;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Shape of the value starting at `i` (after `return` or a concise arrow `=>`).
fn value_shape(s: &[u8], i: usize) -> Option<Shape> {
    let i = skip_ws(s, i);
    let rest = std::str::from_utf8(s.get(i..)?).ok()?;
    static FN_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:async\s+)?(?:function\b|\([^()]*\)\s*(?::[^=]+)?=>|[A-Za-z_$][\w$]*\s*=>)").unwrap());
    match s.get(i)? {
        b'{' => Some(Shape::Object),
        b'[' => Some(Shape::Array),
        b'(' => {
            let j = skip_ws(s, i + 1);
            match s.get(j)? {
                b'{' => Some(Shape::Object),
                b'[' => Some(Shape::Array),
                _ if FN_VALUE.is_match(rest) => Some(Shape::Function),
                _ => None,
            }
        }
        _ if FN_VALUE.is_match(rest) => Some(Shape::Function),
        _ => None,
    }
}

/// Shapes returned directly by a function body (between braces), ignoring nested functions.
fn body_shape(s: &[u8], open: usize, close: usize) -> Option<Shape> {
    // frame stack: true = nested function body
    let mut frames: Vec<bool> = Vec::new();
    let mut shapes: Vec<Option<Shape>> = Vec::new();
    let mut i = open + 1;
    while i < close {
        let c = s[i];
        if c == b'{' {
            frames.push(starts_function(s, i));
        } else if c == b'}' {
            frames.pop();
        } else if c == b'r' && s[i..].starts_with(b"return") && (i == 0 || !is_ident(s[i - 1])) && !s.get(i + 6).is_some_and(|&n| is_ident(n)) {
            if !frames.iter().any(|&f| f) {
                let after = skip_ws(s, i + 6);
                let void = after >= close || matches!(s[after], b';' | b'}');
                if !void {
                    shapes.push(value_shape(s, after));
                }
            }
            i += 6;
            continue;
        }
        i += 1;
    }
    let first = (*shapes.first()?)?;
    shapes.iter().all(|&x| x == Some(first)).then_some(first)
}

/// Whether the `{` at `i` opens a function body (vs. a block or object literal).
fn starts_function(s: &[u8], i: usize) -> bool {
    let mut j = i;
    while j > 0 && s[j - 1].is_ascii_whitespace() {
        j -= 1;
    }
    if j >= 2 && &s[j - 2..j] == b"=>" {
        return true;
    }
    if j == 0 || s[j - 1] != b')' {
        return false;
    }
    // find the matching '(' backwards
    let mut depth = 0i32;
    let mut k = j - 1;
    loop {
        match s[k] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        if k == 0 {
            return false;
        }
        k -= 1;
    }
    let mut e = k;
    while e > 0 && s[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    let mut st = e;
    while st > 0 && is_ident(s[st - 1]) {
        st -= 1;
    }
    let word = &s[st..e];
    !matches!(word, b"if" | b"for" | b"while" | b"switch" | b"catch" | b"with")
}

static FN_DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bfunction\s+(use[A-Z0-9]\w*)\s*(?:<[^()]*?>)?\s*\(").unwrap());
static ARROW_DEF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:const|let|var)\s+(use[A-Z0-9]\w*)\s*(?::[^=]+)?=\s*(?:async\s+)?(?:function\b[^(]*)?(?:<[^()]*?>)?\s*\(").unwrap());
static DEFAULT_EXPORT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bexport\s+default\s+(?:function\s+)?(use[A-Z0-9]\w*)\b").unwrap());

/// Hooks defined in a source file and the shape each returns (when determinable).
/// The key `"default"` maps to the default-exported hook, if any.
pub fn hook_definitions(src: &str) -> BTreeMap<String, Shape> {
    let clean = sanitize(src, true);
    let s = clean.as_bytes();
    let mut out = BTreeMap::new();
    for (re, arrow_capable) in [(&*FN_DEF, false), (&*ARROW_DEF, true)] {
        for cap in re.captures_iter(&clean) {
            let name = cap[1].to_string();
            let paren = cap.get(0).map(|m| m.end() - 1).unwrap_or(0);
            let Some(close_paren) = matching(s, paren) else { continue };
            let mut i = skip_ws(s, close_paren + 1);
            let is_arrow_fn = arrow_capable && !cap[0].contains("function");
            let mut declared = None;
            if s.get(i) == Some(&b':') {
                declared = declared_shape(s, i + 1);
                let Some(next) = skip_type(s, i + 1, is_arrow_fn) else { continue };
                i = next;
            }
            let shape = if is_arrow_fn {
                if !s[i..].starts_with(b"=>") {
                    continue;
                }
                let body = skip_ws(s, i + 2);
                if s.get(body) == Some(&b'{') {
                    matching(s, body).and_then(|end| body_shape(s, body, end))
                } else {
                    value_shape(s, body)
                }
            } else {
                if s.get(i) != Some(&b'{') {
                    continue;
                }
                matching(s, i).and_then(|end| body_shape(s, i, end))
            };
            if let Some(sh) = declared.or(shape) {
                out.insert(name, sh);
            }
        }
    }
    if let Some(c) = DEFAULT_EXPORT_NAME.captures(&clean) {
        if let Some(sh) = out.get(&c[1]).copied() {
            out.insert("default".into(), sh);
        }
    }
    out
}

static IMPORT_CLAUSE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\bimport\s+(?:type\s+)?([^'";]+?)\s+from\s+['"]([^'"\n]+)['"]"#).unwrap());

/// Local name → (module specifier, imported name or "default").
pub fn imported_bindings(src: &str) -> Vec<(String, String, String)> {
    let clean = sanitize(src, false);
    let mut out = Vec::new();
    for c in IMPORT_CLAUSE.captures_iter(&clean) {
        let clause = c[1].trim();
        let spec = c[2].to_string();
        let (default_part, named_part) = match clause.find('{') {
            Some(p) => (clause[..p].trim().trim_end_matches(',').trim(), Some(&clause[p..])),
            None => (clause, None),
        };
        if !default_part.is_empty() && !default_part.starts_with('*') {
            out.push((default_part.to_string(), spec.clone(), "default".into()));
        }
        if let Some(named) = named_part {
            for item in named.trim_matches(|c| c == '{' || c == '}' || char::is_whitespace(c)).split(',') {
                let item = item.trim().trim_start_matches("type ").trim();
                if item.is_empty() {
                    continue;
                }
                let (imported, local) = match item.split_once(" as ") {
                    Some((a, b)) => (a.trim(), b.trim()),
                    None => (item, item),
                };
                out.push((local.to_string(), spec.clone(), imported.to_string()));
            }
        }
    }
    out
}

/// How a call site destructures `hook(...)`: `Some(Array)` for `const [..] =`, `Some(Object)`
/// for `const {..} =`. Returns (byte offset, shape) for each destructuring call.
pub fn destructuring_calls(src: &str, hook: &str) -> Vec<(usize, Shape)> {
    let clean = sanitize(src, true);
    let re = Regex::new(&format!(r"\b(?:const|let|var)\s+([\[{{])[^;=]*=\s*(?:await\s+)?{}\s*(?:<[^>()]*>)?\s*\(", regex::escape(hook))).expect("valid regex");
    re.captures_iter(&clean)
        .map(|c| {
            let shape = if &c[1] == "[" { Shape::Array } else { Shape::Object };
            (c.get(0).map(|m| m.start()).unwrap_or(0), shape)
        })
        .collect()
}

/// True when a call site's destructuring is incompatible with the hook's return shape.
pub fn incompatible(returned: Shape, destructured: Shape) -> bool {
    returned != destructured
}

pub fn mismatch_message(file: &str, hook: &str, destructured: Shape, returned: Shape, origin: &str, line: usize) -> String {
    let (pattern, fix) = match destructured {
        Shape::Array => (format!("const [...] = {hook}(...)"), "is not iterable"),
        _ => (format!("const {{ ... }} = {hook}(...)"), "gives undefined values"),
    };
    let suggestion = match returned {
        Shape::Object => format!("use `const {{ ... }} = {hook}(...)` with the hook's property names"),
        Shape::Array => format!("use `const [...] = {hook}(...)` in the hook's element order"),
        Shape::Function => format!("call it as `const bind = {hook}(...)` and spread `{{...bind()}}` onto the element"),
    };
    format!(
        "{file}:{line} destructures {hook}() as {} (`{pattern}`), but {hook} from {origin} returns {}. This crashes or misbehaves at runtime (\"{fix}\"). Either {suggestion}, or change the hook so caller and hook agree.",
        destructured.describe(),
        returned.describe(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_object_and_array_returning_hooks() {
        let src = r#"
import { useState, useRef } from 'react';
export function useDrag(initial: number) {
  const [pos, setPos] = useState(initial);
  const ref = useRef<HTMLDivElement>(null);
  const onDown = () => { return [1, 2]; };          // nested function: ignored
  if (pos > 10) {
    return { pos, ref, onDown, dragging: true };    // inside an if block: counted
  }
  return { pos, ref, onDown, dragging: false };
}
export const useToggle = (v = false) => {
  const [on, set] = useState(v);
  return [on, () => set(!on)] as const;
};
export const useConcise = () => ({ a: 1 });
export const useBind = () => (e: Event) => e;
export function useTyped(): [number, (n: number) => void] { return useState(0); }
export function useUnknown() { const x = { a: 1 }; return x; }
"#;
        let d = hook_definitions(src);
        assert_eq!(d.get("useDrag"), Some(&Shape::Object));
        assert_eq!(d.get("useToggle"), Some(&Shape::Array));
        assert_eq!(d.get("useConcise"), Some(&Shape::Object));
        assert_eq!(d.get("useBind"), Some(&Shape::Function));
        assert_eq!(d.get("useTyped"), Some(&Shape::Array), "declared return type wins");
        assert_eq!(d.get("useUnknown"), None, "undeterminable shapes are not guessed");
    }

    #[test]
    fn mixed_returns_are_not_guessed() {
        let d = hook_definitions("export function useX(a: boolean) { if (a) { return [1]; } return { b: 1 }; }");
        assert_eq!(d.get("useX"), None);
    }

    #[test]
    fn default_export_and_strings_with_braces() {
        let src = "const label = 'Don\\'t {break} me';\nexport default function useThing() { const s = `${'{'}`; return { s }; }\n";
        let d = hook_definitions(src);
        assert_eq!(d.get("useThing"), Some(&Shape::Object));
        assert_eq!(d.get("default"), Some(&Shape::Object));
    }

    #[test]
    fn finds_imports_and_destructuring() {
        let src = r#"
import useDrag, { useToggle as useT, type Foo } from '../hooks/useDrag';
import { useDrag as useGestureDrag } from '@use-gesture/react';
// const [x] = useDrag() in a comment is ignored
export function Card() {
  const [{ isDragging }, drag] = useDrag({ id: 1 });
  const {
    on,
  } = useT();
  const bind = useGestureDrag(() => {});
  return null
}
"#;
        let b = imported_bindings(src);
        assert!(b.contains(&("useDrag".into(), "../hooks/useDrag".into(), "default".into())));
        assert!(b.contains(&("useT".into(), "../hooks/useDrag".into(), "useToggle".into())));
        assert!(b.contains(&("useGestureDrag".into(), "@use-gesture/react".into(), "useDrag".into())));
        assert_eq!(destructuring_calls(src, "useDrag").iter().map(|c| c.1).collect::<Vec<_>>(), vec![Shape::Array]);
        assert_eq!(destructuring_calls(src, "useT").iter().map(|c| c.1).collect::<Vec<_>>(), vec![Shape::Object]);
        assert!(destructuring_calls(src, "useGestureDrag").is_empty(), "plain assignment is fine");
    }

    #[test]
    fn asi_style_statements_do_not_bleed_together() {
        let src = "const [a] = useOther()\nconst { b } = useDrag()\n";
        assert_eq!(destructuring_calls(src, "useDrag").iter().map(|c| c.1).collect::<Vec<_>>(), vec![Shape::Object]);
    }

    #[test]
    fn known_library_shapes() {
        assert_eq!(known_shape("react-dnd", "useDrag"), Some(Shape::Array));
        assert_eq!(known_shape("@use-gesture/react", "useDrag"), Some(Shape::Function));
        assert_eq!(known_shape("@dnd-kit/core", "useDraggable"), Some(Shape::Object));
        assert_eq!(known_shape("left-pad", "useDrag"), None);
    }
}
