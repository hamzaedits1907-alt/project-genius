//! Structured project specification produced by the planning step, and robust
//! extraction of JSON from model output (models differ in how strictly they follow
//! "JSON only" instructions).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{GeniusError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectSpec {
    pub name: String,
    pub purpose: String,
    pub features: Vec<Named>,
    pub pages: Vec<Page>,
    pub components: Vec<Named>,
    pub data_models: Vec<DataModel>,
    pub architecture: String,
    pub dependencies: Vec<Dependency>,
    pub milestones: Vec<String>,
    pub test_strategy: String,
    pub risks: Vec<String>,
    pub assumptions: Vec<String>,
    pub files: Vec<PlannedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Named {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Page {
    pub name: String,
    pub description: String,
    pub components: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DataModel {
    pub name: String,
    pub fields: Vec<String>,
    pub relations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Dependency {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct PlannedFile {
    pub path: String,
    pub purpose: String,
}

/// Accept either `[{"name","description"}]` or `["text"]` for named lists.
fn coerce_named_lists(v: &mut Value) {
    for key in ["features", "components"] {
        if let Some(arr) = v.get_mut(key).and_then(|a| a.as_array_mut()) {
            for item in arr.iter_mut() {
                if let Some(s) = item.as_str() {
                    let (n, d) = s.split_once(':').map(|(a, b)| (a.trim(), b.trim())).unwrap_or((s, ""));
                    *item = serde_json::json!({ "name": n, "description": d });
                }
            }
        }
    }
    for key in ["dependencies"] {
        if let Some(arr) = v.get_mut(key).and_then(|a| a.as_array_mut()) {
            for item in arr.iter_mut() {
                if let Some(s) = item.as_str() {
                    *item = serde_json::json!({ "name": s, "reason": "" });
                }
            }
        }
    }
    for key in ["pages"] {
        if let Some(arr) = v.get_mut(key).and_then(|a| a.as_array_mut()) {
            for item in arr.iter_mut() {
                if let Some(s) = item.as_str() {
                    *item = serde_json::json!({ "name": s, "description": "", "components": [] });
                }
            }
        }
    }
    for key in ["dataModels"] {
        if let Some(arr) = v.get_mut(key).and_then(|a| a.as_array_mut()) {
            for item in arr.iter_mut() {
                if let Some(fields) = item.get_mut("fields").and_then(|f| f.as_array_mut()) {
                    for f in fields.iter_mut() {
                        if !f.is_string() {
                            let text = match &*f {
                                Value::Object(o) => o
                                    .iter()
                                    .map(|(k, v)| format!("{k}: {}", v.as_str().map(String::from).unwrap_or_else(|| v.to_string())))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                other => other.to_string(),
                            };
                            *f = Value::String(text);
                        }
                    }
                }
            }
        }
    }
    for key in ["milestones", "risks", "assumptions"] {
        if let Some(arr) = v.get_mut(key).and_then(|a| a.as_array_mut()) {
            for item in arr.iter_mut() {
                if !item.is_string() {
                    let text = item
                        .get("name")
                        .or_else(|| item.get("title"))
                        .or_else(|| item.get("description"))
                        .and_then(|x| x.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| item.to_string());
                    *item = Value::String(text);
                }
            }
        }
    }
    for key in ["architecture", "testStrategy", "purpose"] {
        if let Some(x) = v.get_mut(key) {
            if !x.is_string() && !x.is_null() {
                let text = match &*x {
                    Value::Array(a) => a.iter().map(|i| i.as_str().map(String::from).unwrap_or_else(|| i.to_string())).collect::<Vec<_>>().join("\n"),
                    other => other.to_string(),
                };
                *x = Value::String(text);
            }
        }
    }
}

/// Find the first balanced top-level JSON object in `text`, ignoring braces inside strings.
pub fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if b == b'\\' {
                esc = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' if start.is_some() => in_str = true,
            b'{' => {
                if start.is_none() {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' if start.is_some() => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start?..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn parse_spec(text: &str) -> Result<ProjectSpec> {
    let raw = extract_json_object(text).ok_or_else(|| GeniusError::Validation("The model did not return a JSON plan.".into()))?;
    let mut v: Value = serde_json::from_str(raw).map_err(|e| GeniusError::Validation(format!("The plan JSON is invalid: {e}")))?;
    // accept snake_case keys too
    if let Some(o) = v.as_object_mut() {
        for (snake, camel) in [("data_models", "dataModels"), ("test_strategy", "testStrategy")] {
            if let Some(x) = o.remove(snake) {
                o.entry(camel.to_string()).or_insert(x);
            }
        }
    }
    coerce_named_lists(&mut v);
    let spec: ProjectSpec = serde_json::from_value(v).map_err(|e| GeniusError::Validation(format!("The plan does not match the expected structure: {e}")))?;
    validate(&spec)?;
    Ok(spec)
}

pub fn validate(spec: &ProjectSpec) -> Result<()> {
    if spec.name.trim().is_empty() {
        return Err(GeniusError::Validation("The plan has no project name.".into()));
    }
    if spec.features.is_empty() {
        return Err(GeniusError::Validation("The plan lists no features.".into()));
    }
    for d in &spec.dependencies {
        if !valid_npm_name(&d.name) {
            return Err(GeniusError::Validation(format!("The plan names an invalid npm package: '{}'", d.name)));
        }
    }
    Ok(())
}

pub fn valid_npm_name(n: &str) -> bool {
    let n = n.trim();
    if n.is_empty() || n.len() > 214 || n.starts_with('.') || n.starts_with('_') || n.starts_with('-') {
        return false;
    }
    let body = match n.strip_prefix('@') {
        Some(rest) => match rest.split_once('/') {
            Some((scope, name)) if !scope.is_empty() && !name.is_empty() => format!("{scope}{name}"),
            _ => return false,
        },
        None => n.to_string(),
    };
    body.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-._".contains(c))
}

/// Human-readable markdown rendering used for project memory and the conversation log.
pub fn to_markdown(s: &ProjectSpec) -> String {
    let mut m = format!("# {}\n\n{}\n\n## Features\n", s.name, s.purpose);
    for f in &s.features {
        m.push_str(&format!("- **{}** — {}\n", f.name, f.description));
    }
    if !s.pages.is_empty() {
        m.push_str("\n## Pages\n");
        for p in &s.pages {
            m.push_str(&format!("- **{}** — {}\n", p.name, p.description));
        }
    }
    if !s.data_models.is_empty() {
        m.push_str("\n## Data models\n");
        for d in &s.data_models {
            m.push_str(&format!("- **{}**: {}\n", d.name, d.fields.join(", ")));
        }
    }
    if !s.architecture.is_empty() {
        m.push_str(&format!("\n## Architecture\n{}\n", s.architecture));
    }
    if !s.assumptions.is_empty() {
        m.push_str("\n## Assumptions\n");
        for a in &s.assumptions {
            m.push_str(&format!("- {a}\n"));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_with_noise_and_braces_in_strings() {
        let t = "Sure! Here is the plan:\n```json\n{\"name\":\"A {b}\",\"x\":{\"y\":1}}\n```\nthanks";
        assert_eq!(extract_json_object(t), Some("{\"name\":\"A {b}\",\"x\":{\"y\":1}}"));
        assert_eq!(extract_json_object("no json"), None);
    }

    #[test]
    fn parses_lenient_shapes() {
        let t = r#"{"name":"Tasks","purpose":"p","features":["Boards: drag tasks","Search"],"pages":["Home"],
          "data_models":[{"name":"Task","fields":[{"id":"string"},"title: string"]}],"dependencies":["date-fns"],
          "milestones":[{"title":"M1"}],"architecture":["SPA","localStorage"]}"#;
        let s = parse_spec(t).unwrap();
        assert_eq!(s.features[0].name, "Boards");
        assert_eq!(s.features[0].description, "drag tasks");
        assert_eq!(s.data_models[0].fields, vec!["id: string", "title: string"]);
        assert_eq!(s.dependencies[0].name, "date-fns");
        assert_eq!(s.milestones, vec!["M1"]);
        assert_eq!(s.architecture, "SPA\nlocalStorage");
    }

    #[test]
    fn rejects_invalid_plans() {
        assert!(parse_spec(r#"{"name":"","features":[{"name":"x"}]}"#).is_err());
        assert!(parse_spec(r#"{"name":"x","features":[]}"#).is_err());
        assert!(parse_spec(r#"{"name":"x","features":["a"],"dependencies":["; rm -rf /"]}"#).is_err());
    }

    #[test]
    fn npm_names() {
        assert!(valid_npm_name("react-router-dom"));
        assert!(valid_npm_name("@tanstack/react-query"));
        assert!(!valid_npm_name("React"));
        assert!(!valid_npm_name("@scope"));
        assert!(!valid_npm_name("a b"));
    }
}
