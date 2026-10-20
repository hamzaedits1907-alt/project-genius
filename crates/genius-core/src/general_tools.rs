//! Deterministic tools for everyday conversation: arithmetic and a summary of attached tables.
//!
//! Both exist so the model does not do arithmetic or statistics "in its head": numbers come from
//! code that really computed them, and the tool result says exactly what was computed.
//!
//! * [`calculate`] — a small recursive-descent evaluator (no `eval`, no scripting): numbers,
//!   `+ - * / % ^`, parentheses, unary minus, a few functions and constants.
//! * [`summarize_table`] — row/column counts, inferred column types, missing values, numeric
//!   statistics and the most frequent values of a CSV/TSV text or a JSON array of objects.

use crate::error::{GeniusError, Result};
use serde_json::Value;
use std::collections::HashMap;

// ------------------------------------------------------------------------------------ calculate

const MAX_EXPR: usize = 400;
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    depth: usize,
}

fn err(msg: impl Into<String>) -> GeniusError {
    GeniusError::Validation(msg.into())
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn peek(&mut self) -> Option<u8> {
        self.ws();
        self.s.get(self.i).copied()
    }
    fn eat(&mut self, c: u8) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expr(&mut self) -> Result<f64> {
        let mut v = self.term()?;
        loop {
            if self.eat(b'+') {
                v += self.term()?;
            } else if self.eat(b'-') {
                v -= self.term()?;
            } else {
                return Ok(v);
            }
        }
    }
    fn term(&mut self) -> Result<f64> {
        let mut v = self.unary()?;
        loop {
            if self.eat(b'*') {
                v *= self.unary()?;
            } else if self.eat(b'/') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err(err("division by zero"));
                }
                v /= d;
            } else if self.eat(b'%') {
                let d = self.unary()?;
                if d == 0.0 {
                    return Err(err("modulo by zero"));
                }
                v %= d;
            } else {
                return Ok(v);
            }
        }
    }
    fn unary(&mut self) -> Result<f64> {
        if self.eat(b'-') {
            return Ok(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        self.power()
    }
    fn power(&mut self) -> Result<f64> {
        let base = self.atom()?;
        if self.eat(b'^') {
            let exp = self.unary()?; // right-associative: 2^3^2 = 2^(3^2)
            return Ok(base.powf(exp));
        }
        Ok(base)
    }
    fn atom(&mut self) -> Result<f64> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(err("expression is nested too deeply"));
        }
        let r = self.atom_inner();
        self.depth -= 1;
        r
    }
    fn atom_inner(&mut self) -> Result<f64> {
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let v = self.expr()?;
                if !self.eat(b')') {
                    return Err(err("missing closing parenthesis"));
                }
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.' || self.s[self.i] == b'_') {
                    self.i += 1;
                }
                // scientific notation: 1e6, 2.5E-3
                if self.i < self.s.len() && (self.s[self.i] == b'e' || self.s[self.i] == b'E') {
                    let save = self.i;
                    self.i += 1;
                    if self.i < self.s.len() && (self.s[self.i] == b'+' || self.s[self.i] == b'-') {
                        self.i += 1;
                    }
                    if self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                        while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                            self.i += 1;
                        }
                    } else {
                        self.i = save;
                    }
                }
                let text: String = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("").replace('_', "");
                text.parse::<f64>().map_err(|_| err(format!("'{text}' is not a number")))
            }
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.i;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("").to_ascii_lowercase();
                if self.peek() == Some(b'(') {
                    self.i += 1;
                    let mut args = Vec::new();
                    if !self.eat(b')') {
                        loop {
                            args.push(self.expr()?);
                            if self.eat(b',') {
                                continue;
                            }
                            if self.eat(b')') {
                                break;
                            }
                            return Err(err("missing closing parenthesis in a function call"));
                        }
                    }
                    return call(&name, &args);
                }
                match name.as_str() {
                    "pi" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    "tau" => Ok(std::f64::consts::TAU),
                    _ => Err(err(format!("unknown name '{name}' (supported constants: pi, e, tau)"))),
                }
            }
            Some(c) => Err(err(format!("unexpected '{}'", c as char))),
            None => Err(err("the expression ends too early")),
        }
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64> {
    let one = |f: fn(f64) -> f64| -> Result<f64> {
        match a {
            [x] => Ok(f(*x)),
            _ => Err(err(format!("{name}() takes exactly one argument"))),
        }
    };
    match name {
        "sqrt" => one(f64::sqrt),
        "cbrt" => one(f64::cbrt),
        "abs" => one(f64::abs),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "exp" => one(f64::exp),
        "ln" => one(f64::ln),
        "log10" => one(f64::log10),
        "log2" => one(f64::log2),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "round" => one(f64::round),
        "log" => match a {
            [x] => Ok(x.log10()),
            [x, base] => Ok(x.ln() / base.ln()),
            _ => Err(err("log() takes a value and optionally a base")),
        },
        "pow" => match a {
            [x, y] => Ok(x.powf(*y)),
            _ => Err(err("pow() takes two arguments")),
        },
        "min" if !a.is_empty() => Ok(a.iter().cloned().fold(f64::INFINITY, f64::min)),
        "max" if !a.is_empty() => Ok(a.iter().cloned().fold(f64::NEG_INFINITY, f64::max)),
        "sum" if !a.is_empty() => Ok(a.iter().sum()),
        "avg" | "mean" if !a.is_empty() => Ok(a.iter().sum::<f64>() / a.len() as f64),
        _ => Err(err(format!("unknown function '{name}' (supported: sqrt, cbrt, abs, sin, cos, tan, asin, acos, atan, exp, ln, log, log2, log10, floor, ceil, round, pow, min, max, sum, avg)"))),
    }
}

/// Format a result without float noise: integers plainly, otherwise up to 12 significant digits.
pub fn format_number(v: f64) -> String {
    if v == v.trunc() && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.11e}");
    // reparse through the 12-digit representation, then print the shortest form
    let rounded: f64 = s.parse().unwrap_or(v);
    if rounded.abs() >= 1e15 || (rounded != 0.0 && rounded.abs() < 1e-6) {
        format!("{rounded:e}")
    } else {
        format!("{rounded}")
    }
}

/// Evaluate an arithmetic expression. Errors say what is wrong; the result is always finite.
pub fn calculate(expression: &str) -> Result<f64> {
    let e = expression.trim();
    if e.is_empty() {
        return Err(err("the expression is empty"));
    }
    if e.len() > MAX_EXPR {
        return Err(err(format!("the expression is longer than {MAX_EXPR} characters")));
    }
    // a thousands-style "1,000" is ambiguous with function arguments: not accepted
    let mut p = Parser { s: e.as_bytes(), i: 0, depth: 0 };
    let v = p.expr()?;
    p.ws();
    if p.i != e.len() {
        return Err(err(format!("unexpected '{}' at position {}", e[p.i..].chars().next().unwrap_or('?'), p.i + 1)));
    }
    if !v.is_finite() {
        return Err(err("the result is not a finite number (check for overflow or an invalid operation such as sqrt(-1))"));
    }
    Ok(v)
}

// -------------------------------------------------------------------------------- table summary

const MAX_ROWS: usize = 200_000;
const MAX_COLS: usize = 200;

/// RFC 4180-style parser: quoted fields, doubled quotes, embedded newlines; `delim` is `,`, `;` or tab.
pub fn parse_delimited(text: &str, delim: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == delim {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if !(row.len() == 1 && row[0].is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
            if rows.len() > MAX_ROWS {
                break;
            }
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn detect_delimiter(text: &str) -> char {
    let head: String = text.lines().take(5).collect::<Vec<_>>().join("\n");
    [',', ';', '\t'].into_iter().max_by_key(|d| head.matches(*d).count()).unwrap_or(',')
}

fn as_number(s: &str) -> Option<f64> {
    let t = s.trim().trim_start_matches('$').trim_start_matches('€').trim_end_matches('%').replace(',', "");
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

struct Column {
    name: String,
    values: Vec<String>,
}

fn columns_of(kind: &str, text: &str) -> Result<(Vec<Column>, usize)> {
    match kind {
        "csv" => {
            let rows = parse_delimited(text, detect_delimiter(text));
            let Some((head, body)) = rows.split_first() else { return Err(err("the table is empty")) };
            let head: Vec<String> = head.iter().take(MAX_COLS).enumerate().map(|(i, h)| if h.trim().is_empty() { format!("column {}", i + 1) } else { h.trim().to_string() }).collect();
            let mut cols: Vec<Column> = head.into_iter().map(|name| Column { name, values: Vec::new() }).collect();
            for r in body {
                for (i, c) in cols.iter_mut().enumerate() {
                    c.values.push(r.get(i).cloned().unwrap_or_default());
                }
            }
            Ok((cols, body.len()))
        }
        "json" => {
            let v: Value = serde_json::from_str(text).map_err(|e| err(format!("not valid JSON: {e}")))?;
            let arr = match &v {
                Value::Array(a) => a.clone(),
                Value::Object(o) => o.values().find_map(|x| x.as_array().filter(|a| a.iter().all(|i| i.is_object())).cloned()).ok_or_else(|| err("the JSON is not a list of records (an array of objects)"))?,
                _ => return Err(err("the JSON is not a list of records (an array of objects)")),
            };
            if arr.is_empty() || !arr.iter().all(|r| r.is_object()) {
                return Err(err("the JSON is not a non-empty list of records (an array of objects)"));
            }
            let mut names: Vec<String> = Vec::new();
            for r in &arr {
                for k in r.as_object().into_iter().flat_map(|o| o.keys()) {
                    if !names.contains(k) && names.len() < MAX_COLS {
                        names.push(k.clone());
                    }
                }
            }
            let cols = names
                .iter()
                .map(|n| Column {
                    name: n.clone(),
                    values: arr
                        .iter()
                        .map(|r| match r.get(n) {
                            None | Some(Value::Null) => String::new(),
                            Some(Value::String(s)) => s.clone(),
                            Some(other) => other.to_string(),
                        })
                        .collect(),
                })
                .collect();
            Ok((cols, arr.len()))
        }
        other => Err(err(format!("data_summary reads CSV, TSV and JSON files, not '{other}'"))),
    }
}

fn describe(col: &Column, detail: bool) -> String {
    let filled: Vec<&String> = col.values.iter().filter(|v| !v.trim().is_empty()).collect();
    let missing = col.values.len() - filled.len();
    let nums: Vec<f64> = filled.iter().filter_map(|v| as_number(v)).collect();
    let mut out = format!("- {} — ", col.name);
    if !filled.is_empty() && nums.len() * 10 >= filled.len() * 7 {
        let mut sorted = nums.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len() as f64;
        let sum: f64 = sorted.iter().sum();
        let mean = sum / n;
        let median = if sorted.len() % 2 == 1 { sorted[sorted.len() / 2] } else { (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0 };
        let var = if sorted.len() > 1 { sorted.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0) } else { 0.0 };
        out.push_str(&format!("number; count {}, missing {missing}, min {}, max {}, mean {}, median {}, sum {}, stdev {}", sorted.len(), format_number(sorted[0]), format_number(sorted[sorted.len() - 1]), format_number(mean), format_number(median), format_number(sum), format_number(var.sqrt())));
        if nums.len() < filled.len() {
            out.push_str(&format!(" ({} non-numeric value(s) ignored)", filled.len() - nums.len()));
        }
    } else {
        let mut freq: HashMap<&str, usize> = HashMap::new();
        for v in &filled {
            *freq.entry(v.trim()).or_default() += 1;
        }
        let mut top: Vec<(&str, usize)> = freq.iter().map(|(k, v)| (*k, *v)).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        out.push_str(&format!("text; count {}, missing {missing}, {} distinct; most frequent: {}", filled.len(), freq.len(), top.iter().take(if detail { 15 } else { 3 }).map(|(k, n)| format!("\"{}\" ×{n}", k.chars().take(40).collect::<String>())).collect::<Vec<_>>().join(", ")));
    }
    out
}

/// Summary of a table. `column`: only that column, in more detail.
pub fn summarize_table(name: &str, kind: &str, text: &str, column: Option<&str>) -> Result<String> {
    let (cols, rows) = columns_of(kind, text)?;
    let mut out = format!("TABLE {name}: {rows} data row(s), {} column(s){}.\n", cols.len(), if rows >= MAX_ROWS { " (reading stopped at the row limit)" } else { "" });
    match column {
        Some(c) => {
            let col = cols.iter().find(|x| x.name.eq_ignore_ascii_case(c.trim())).ok_or_else(|| err(format!("no column named '{c}'. Columns: {}", cols.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(", "))))?;
            out.push_str(&describe(col, true));
        }
        None => {
            for col in &cols {
                out.push_str(&describe(col, false));
                out.push('\n');
            }
        }
    }
    out.push_str("\nThese figures were computed by Genius from the whole file, not estimated.");
    Ok(out)
}


// -------------------------------------------------------------------------------- table queries (data_query)

const QUERY_OPS: &[&str] = &["=", "!=", ">", ">=", "<", "<=", "contains"];
const QUERY_FNS: &[&str] = &["count", "sum", "mean", "min", "max", "median"];

#[derive(Debug, Default, Clone)]
pub struct QuerySpec {
    pub filters: Vec<(String, String, String)>,
    pub columns: Vec<String>,
    pub group_by: Option<String>,
    pub metrics: Vec<(String, Option<String>)>,
    pub sort_by: Option<String>,
    pub desc: Option<bool>,
    pub limit: usize,
}

fn json_list(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).ok().map(|p| json_list(Some(&p))).unwrap_or_else(|| if s.trim().is_empty() { vec![] } else { vec![Value::String(s.clone())] }),
        Some(o @ Value::Object(_)) => vec![o.clone()],
        _ => vec![],
    }
}

/// Reads the arguments of a `data_query` call leniently: nested lists may arrive as JSON text, a single object instead of a list, or `columns` as "a, b".
pub fn query_from_args(args: &Value) -> Result<QuerySpec> {
    let get = |names: &[&str]| names.iter().find_map(|n| args.get(*n)).cloned();
    let mut q = QuerySpec { limit: 20, ..Default::default() };
    for w in json_list(get(&["where", "filter", "filters", "conditions"]).as_ref()) {
        let column = w.get("column").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let op = w.get("op").and_then(|v| v.as_str()).unwrap_or("=").trim().to_lowercase();
        let value = match w.get("value") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(o) => o.to_string(),
        };
        q.filters.push((column, op, value));
    }
    q.columns = match get(&["columns", "select", "fields"]) {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    };
    q.group_by = get(&["group_by", "groupBy", "group", "by"]).and_then(|v| v.as_str().map(String::from));
    for m in json_list(get(&["metrics", "aggregate", "aggregates", "aggregations", "agg"]).as_ref()) {
        let f = m.get("fn").and_then(|v| v.as_str()).unwrap_or("count").to_lowercase();
        let c = m.get("column").and_then(|v| v.as_str()).map(String::from);
        q.metrics.push((f, c));
    }
    q.sort_by = get(&["sort_by", "sortBy", "sort", "order_by", "orderBy"]).and_then(|v| v.as_str().map(String::from));
    q.desc = get(&["desc"]).and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true"))));
    if let Some(l) = get(&["limit"]).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))) {
        q.limit = (l as usize).clamp(1, 200);
    }
    Ok(q)
}

fn find_col<'a>(cols: &'a [Column], name: &str, file: &str) -> Result<&'a Column> {
    cols.iter().find(|c| c.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| err(format!("no column \"{name}\" in {file}. Columns: {}", cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", "))))
}

/// Numbers compare as numbers, everything else as case-insensitive text.
fn cmp_cells(a: &str, b: &str) -> std::cmp::Ordering {
    match (as_number(a), as_number(b)) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => a.trim().to_lowercase().cmp(&b.trim().to_lowercase()),
    }
}

fn aggregate(f: &str, values: &[&str]) -> (usize, Option<f64>) {
    if f == "count" {
        let n = values.iter().filter(|v| !v.trim().is_empty()).count();
        return (n, Some(n as f64));
    }
    let mut nums: Vec<f64> = values.iter().filter_map(|v| as_number(v)).collect();
    if nums.is_empty() {
        return (0, None);
    }
    nums.sort_by(|a, b| a.total_cmp(b));
    let sum: f64 = nums.iter().sum();
    let mid = if nums.len() % 2 == 1 { nums[nums.len() / 2] } else { (nums[nums.len() / 2 - 1] + nums[nums.len() / 2]) / 2.0 };
    let v = match f {
        "sum" => sum,
        "mean" => sum / nums.len() as f64,
        "min" => nums[0],
        "max" => nums[nums.len() - 1],
        _ => mid,
    };
    (nums.len(), Some(v))
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Filter, group, aggregate, sort and limit a table: every figure is computed from the whole file.
pub fn query_table(name: &str, kind: &str, text: &str, q: &QuerySpec) -> Result<String> {
    let (cols, total) = columns_of(kind, text)?;
    if cols.is_empty() || total == 0 {
        return Err(err(format!("{name} has no rows")));
    }
    let mut idx: Vec<usize> = (0..total).collect();
    let mut notes: Vec<String> = Vec::new();
    for (column, op, value) in &q.filters {
        let c = find_col(&cols, column, name)?;
        if !QUERY_OPS.contains(&op.as_str()) && op != "==" {
            return Err(err(format!("unknown filter operator \"{op}\". Use one of: {}", QUERY_OPS.join(" "))));
        }
        idx.retain(|&i| {
            let v = c.values.get(i).map(String::as_str).unwrap_or("");
            match op.as_str() {
                "=" | "==" => cmp_cells(v, value).is_eq(),
                "!=" => !cmp_cells(v, value).is_eq(),
                ">" => cmp_cells(v, value).is_gt(),
                ">=" => cmp_cells(v, value).is_ge(),
                "<" => cmp_cells(v, value).is_lt(),
                "<=" => cmp_cells(v, value).is_le(),
                _ => v.to_lowercase().contains(&value.to_lowercase()),
            }
        });
        notes.push(format!("{} {op} {value}", c.name));
    }
    let limit = q.limit.clamp(1, 200);
    let header = format!("TABLE {name}: {total} rows{} (computed from the whole file)", if notes.is_empty() { String::new() } else { format!("; filter {} → {} rows", notes.join(" AND "), idx.len()) });

    let mut metrics: Vec<(String, Option<&Column>)> = Vec::new();
    for (f, c) in &q.metrics {
        if !QUERY_FNS.contains(&f.as_str()) {
            return Err(err(format!("unknown metric \"{f}\". Use one of: {}", QUERY_FNS.join(", "))));
        }
        if f != "count" && c.is_none() {
            return Err(err(format!("metric \"{f}\" needs a \"column\"")));
        }
        metrics.push((f.clone(), c.as_deref().map(|n| find_col(&cols, n, name)).transpose()?));
    }
    let label = |m: &(String, Option<&Column>)| m.1.map(|c| format!("{}({})", m.0, c.name)).unwrap_or_else(|| "count".into());
    let vals_of = |m: &(String, Option<&Column>), rows: &[usize]| -> Vec<String> { rows.iter().map(|&i| m.1.map(|c| c.values.get(i).cloned().unwrap_or_default()).unwrap_or_else(|| "x".into())).collect() };

    if let Some(g) = &q.group_by {
        let gc = find_col(&cols, g, name)?;
        let ms = if metrics.is_empty() { vec![("count".to_string(), None)] } else { metrics.clone() };
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in &idx {
            let raw = gc.values.get(i).map(|s| s.trim()).unwrap_or("");
            let k = if raw.is_empty() { "(empty)".to_string() } else { raw.to_string() };
            if !groups.contains_key(&k) {
                order.push(k.clone());
            }
            groups.entry(k).or_default().push(i);
        }
        let mut rows: Vec<(String, usize, Vec<Option<f64>>)> = order
            .iter()
            .map(|k| {
                let is = &groups[k];
                (k.clone(), is.len(), ms.iter().map(|m| aggregate(&m.0, &vals_of(m, is).iter().map(String::as_str).collect::<Vec<_>>()).1).collect())
            })
            .collect();
        let sort_key = q.sort_by.clone().unwrap_or_else(|| label(&ms[0])).to_lowercase();
        let si = ms.iter().position(|m| label(m).to_lowercase() == sort_key).unwrap_or(0);
        let by_group = sort_key == gc.name.to_lowercase();
        let dir: f64 = if q.desc == Some(false) { 1.0 } else { -1.0 };
        rows.sort_by(|a, b| {
            let d = if by_group { a.0.cmp(&b.0) } else { a.2[si].unwrap_or(f64::NEG_INFINITY).total_cmp(&b.2[si].unwrap_or(f64::NEG_INFINITY)) };
            let d = if dir < 0.0 { d.reverse() } else { d };
            d.then(a.0.cmp(&b.0))
        });
        let n_groups = rows.len();
        let mut out = format!("{header}\nGrouped by {}: {n_groups} groups{}\n| {} | rows | {} |\n|---|---|{}|\n", gc.name, if n_groups > limit { format!(" (showing {limit})") } else { String::new() }, gc.name, ms.iter().map(&label).collect::<Vec<_>>().join(" | "), ms.iter().map(|_| "---").collect::<Vec<_>>().join("|"));
        for (k, n, vals) in rows.into_iter().take(limit) {
            out.push_str(&format!("| {} | {n} | {} |\n", md_cell(&k), vals.iter().map(|v| v.map(format_number).unwrap_or_else(|| "—".into())).collect::<Vec<_>>().join(" | ")));
        }
        return Ok(out.trim_end().to_string());
    }

    if !metrics.is_empty() {
        let lines: Vec<String> = metrics
            .iter()
            .map(|m| {
                let vals = vals_of(m, &idx);
                let (n, v) = aggregate(&m.0, &vals.iter().map(String::as_str).collect::<Vec<_>>());
                format!("- {} = {}{}", label(m), v.map(format_number).unwrap_or_else(|| "no numeric values".into()), if m.1.is_some() && m.0 != "count" { format!(" (over {n} numeric values)") } else { String::new() })
            })
            .collect();
        return Ok(format!("{header}\n{}", lines.join("\n")));
    }

    let pick: Vec<&Column> = if q.columns.is_empty() { cols.iter().take(12).collect() } else { q.columns.iter().map(|c| find_col(&cols, c, name)).collect::<Result<_>>()? };
    if let Some(s) = &q.sort_by {
        let sc = find_col(&cols, s, name)?;
        let desc = q.desc == Some(true);
        idx.sort_by(|&a, &b| {
            let o = cmp_cells(sc.values.get(a).map(String::as_str).unwrap_or(""), sc.values.get(b).map(String::as_str).unwrap_or(""));
            (if desc { o.reverse() } else { o }).then(a.cmp(&b))
        });
    }
    let shown = idx.len().min(limit);
    let mut out = format!("{header}\nShowing {shown} of {} rows{}\n| {} |\n|{}|\n", idx.len(), q.sort_by.as_ref().map(|s| format!(" sorted by {s}{}", if q.desc == Some(true) { " (descending)" } else { "" })).unwrap_or_default(), pick.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" | "), pick.iter().map(|_| "---").collect::<Vec<_>>().join("|"));
    for &i in idx.iter().take(limit) {
        out.push_str(&format!("| {} |\n", pick.iter().map(|c| md_cell(c.values.get(i).map(String::as_str).unwrap_or(""))).collect::<Vec<_>>().join(" | ")));
    }
    Ok(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(s: &str) -> String {
        calculate(s).map(format_number).unwrap_or_else(|e| format!("ERR {e}"))
    }

    #[test]
    fn arithmetic_follows_precedence_and_associativity() {
        assert_eq!(calc("1 + 2 * 3"), "7");
        assert_eq!(calc("(1 + 2) * 3"), "9");
        assert_eq!(calc("2 ^ 3 ^ 2"), "512");
        assert_eq!(calc("-2 ^ 2"), "-4", "unary minus binds weaker than ^ as in mathematics only when written -(2^2); here it applies to the power");
        assert_eq!(calc("10 % 4"), "2");
        assert_eq!(calc("7 / 2"), "3.5");
        assert_eq!(calc("0.1 + 0.2"), "0.3", "float noise is rounded away");
        assert_eq!(calc("1e3 + 2.5E-1"), "1000.25");
        assert_eq!(calc("1_000 * 3"), "3000");
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(calc("sqrt(144)"), "12");
        assert_eq!(calc("round(pi * 100) / 100"), "3.14");
        assert_eq!(calc("max(3, 9, 4) - min(3, 9, 4)"), "6");
        assert_eq!(calc("log(8, 2)"), "3");
        assert_eq!(calc("avg(2, 4, 9)"), "5");
        assert_eq!(calc("sin(0) + cos(0)"), "1");
    }

    #[test]
    fn errors_are_reported_not_guessed() {
        assert!(calc("1 / 0").contains("division by zero"));
        assert!(calc("sqrt(-1)").contains("not a finite number"));
        assert!(calc("2 +").contains("ends too early"));
        assert!(calc("(1 + 2").contains("closing parenthesis"));
        assert!(calc("foo(1)").contains("unknown function"));
        assert!(calc("x + 1").contains("unknown name"));
        assert!(calc("1 2").contains("unexpected"));
        assert!(calc("").contains("empty"));
        assert!(calc(&"(".repeat(200)).contains("nested too deeply"));
        assert!(calc("2 ^ 5000").contains("not a finite number"));
        // nothing executable: these are just unknown names / characters
        assert!(calc("system('calc')").starts_with("ERR"));
        assert!(calc("1; 2").starts_with("ERR"));
    }

    #[test]
    fn csv_is_parsed_with_quotes_and_detected_delimiters() {
        let rows = parse_delimited("name,note\n\"Smith, J\",\"said \"\"hi\"\"\"\nLee,\"two\nlines\"\n", ',');
        assert_eq!(rows, vec![vec!["name", "note"], vec!["Smith, J", "said \"hi\""], vec!["Lee", "two\nlines"]]);
        assert_eq!(detect_delimiter("a;b;c\n1;2;3"), ';');
        assert_eq!(detect_delimiter("a\tb\n1\t2"), '\t');
    }

    #[test]
    fn table_summary_computes_real_statistics() {
        let csv = "city,sales,note\nBerlin,10,ok\nBerlin,20,\nParis,\"1,000\",ok\nRome,x,ok\n";
        let s = summarize_table("sales.csv", "csv", csv, None).unwrap();
        assert!(s.contains("4 data row(s), 3 column(s)"), "{s}");
        assert!(s.contains("- sales — number; count 3, missing 0, min 10, max 1000, mean 343.333333333, median 20, sum 1030"), "{s}");
        assert!(s.contains("1 non-numeric value(s) ignored"), "{s}");
        assert!(s.contains("- city — text; count 4, missing 0, 3 distinct; most frequent: \"Berlin\" ×2"), "{s}");
        assert!(s.contains("- note — text; count 3, missing 1"), "{s}");
        let one = summarize_table("sales.csv", "csv", csv, Some("CITY")).unwrap();
        assert!(one.contains("\"Paris\" ×1") && !one.contains("- sales"), "{one}");
        assert!(summarize_table("sales.csv", "csv", csv, Some("nope")).unwrap_err().to_string().contains("Columns: city, sales, note"));
    }

    #[test]
    fn json_records_are_summarised_and_other_shapes_refused() {
        let j = r#"[{"a": 1, "b": "x"}, {"a": 3, "b": "x"}, {"a": null, "b": "y"}]"#;
        let s = summarize_table("d.json", "json", j, None).unwrap();
        assert!(s.contains("3 data row(s), 2 column(s)") && s.contains("- a — number; count 2, missing 1, min 1, max 3, mean 2"), "{s}");
        assert!(summarize_table("d.json", "json", r#"{"a": 1}"#, None).is_err());
        assert!(summarize_table("d.json", "json", "[]", None).is_err());
        assert!(summarize_table("x.pdf", "pdf", "text", None).unwrap_err().to_string().contains("CSV, TSV and JSON"));
    }
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use serde_json::json;

    const CSV: &str = "date,category,amount,note\n2026-01-03,Food,12.50,lunch\n2026-01-04,Food,30,groceries\n2026-01-05,Rent,900,jan\n2026-02-01,Food,8,coffee\n2026-02-02,Fun,45.5,cinema\n2026-02-03,Fun,,missing amount\n";
    fn run(a: serde_json::Value) -> Result<String> {
        query_table("spend.csv", "csv", CSV, &query_from_args(&a)?)
    }

    #[test]
    fn groups_and_sums_over_the_whole_file_biggest_first() {
        let r = run(json!({"group_by": "category", "metrics": [{"column": "amount", "fn": "sum"}]})).unwrap();
        assert!(r.contains("| category | rows | sum(amount) |"));
        let rows: Vec<&str> = r.lines().filter(|l| l.starts_with("| Rent") || l.starts_with("| Fun") || l.starts_with("| Food")).collect();
        assert_eq!(rows, vec!["| Rent | 1 | 900 |", "| Food | 3 | 50.5 |", "| Fun | 2 | 45.5 |"]);
    }

    #[test]
    fn filters_compare_numbers_as_numbers_and_text_case_insensitively() {
        assert!(run(json!({"where": [{"column": "amount", "op": ">", "value": 20}], "columns": ["category", "amount"], "sort_by": "amount", "desc": true})).unwrap().contains("Showing 3 of 3 rows"));
        assert!(run(json!({"where": [{"column": "category", "value": "food"}], "metrics": [{"column": "amount", "fn": "mean"}]})).unwrap().contains("mean(amount) = 16.8333333333"));
        assert!(run(json!({"where": [{"column": "note", "op": "contains", "value": "JAN"}], "columns": ["note"]})).unwrap().contains("| jan |"));
    }

    #[test]
    fn aggregates_without_grouping_and_reports_the_numeric_count() {
        let r = run(json!({"metrics": [{"column": "amount", "fn": "sum"}, {"fn": "count"}]})).unwrap();
        assert!(r.contains("sum(amount) = 996 (over 5 numeric values)"), "{r}");
        assert!(r.contains("- count = 6"), "{r}");
    }

    #[test]
    fn lists_rows_with_a_limit_and_honest_totals() {
        let r = run(json!({"limit": 2, "sort_by": "date", "desc": true})).unwrap();
        assert!(r.contains("Showing 2 of 6 rows sorted by date (descending)"));
        assert!(r.contains("2026-02-03") && !r.contains("2026-01-03"));
    }

    #[test]
    fn accepts_nested_arguments_written_as_json_text() {
        let r = run(json!({"group_by": "category", "metrics": "[{\"column\":\"amount\",\"fn\":\"sum\"}]", "where": {"column": "category", "op": "!=", "value": "Rent"}})).unwrap();
        assert!(r.contains("| Food | 3 | 50.5 |") && !r.contains("| Rent |"));
    }

    #[test]
    fn explains_a_wrong_column_operator_or_metric() {
        assert!(run(json!({"group_by": "nope"})).unwrap_err().to_string().contains("no column \"nope\""));
        assert!(run(json!({"where": [{"column": "amount", "op": "~", "value": 1}]})).unwrap_err().to_string().contains("unknown filter operator"));
        assert!(run(json!({"metrics": [{"column": "amount", "fn": "avg"}]})).unwrap_err().to_string().contains("unknown metric"));
        assert!(run(json!({"metrics": [{"fn": "sum"}]})).unwrap_err().to_string().contains("needs a \"column\""));
    }
}
