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
