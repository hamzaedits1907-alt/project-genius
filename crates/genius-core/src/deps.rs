//! Dependency state of a project, computed from the files on disk every time it is asked for.
//!
//! There is deliberately no cached "dependencies installed" flag. The answer is derived from:
//!
//! * `package.json` — parsed strictly (duplicate keys are an error, not "last one wins"),
//! * every declared package actually being present in `node_modules/<name>/package.json`,
//! * a fingerprint of the dependency-relevant parts of `package.json` plus the lockfile,
//!   recorded in `node_modules/.genius/install-state.json` after a *successful* install.
//!
//! Editing a script does not require a reinstall; adding, removing or re-versioning a package,
//! or changing the lockfile, does. Deleting `node_modules` resets everything.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::error::{GeniusError, Result};

const DEP_SECTIONS: &[&str] = &["dependencies", "devDependencies", "optionalDependencies"];
const FINGERPRINT_KEYS: &[&str] = &["dependencies", "devDependencies", "optionalDependencies", "peerDependencies", "overrides", "resolutions", "workspaces"];
const STATE_FILE: &str = "node_modules/.genius/install-state.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DepState {
    /// Every declared package is installed and nothing changed since the last install.
    Ready,
    /// An install is required (see `reason`).
    NeedsInstall,
    NoPackageJson,
    InvalidPackageJson,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DepStatus {
    pub state: DepState,
    /// Plain-language explanation for the user.
    pub reason: String,
    /// Declared packages that are not present in node_modules.
    pub missing: Vec<String>,
    /// Declared packages (all dependency sections).
    pub packages: Vec<String>,
    /// Declared packages that were never part of a successful install or the Genius template
    /// (installing them runs code the user has not approved yet).
    pub new_packages: Vec<String>,
    pub fingerprint: Option<String>,
}

impl DepStatus {
    pub fn needs_install(&self) -> bool {
        self.state == DepState::NeedsInstall
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InstallState {
    pub fingerprint: String,
    pub packages: Vec<String>,
    pub installed_at: String,
}

/// Parse JSON strictly: syntax errors and duplicate object keys at any depth are rejected.
/// (`serde_json::Value` keeps the last duplicate silently, and so does `JSON.parse` in npm —
/// which is how a duplicated `"dependencies"` block silently drops packages.)
pub fn parse_strict(text: &str) -> std::result::Result<Value, String> {
    use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct Strict;
    impl<'de> DeserializeSeed<'de> for Strict {
        type Value = Value;
        fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> std::result::Result<Value, D::Error> {
            d.deserialize_any(StrictVisitor)
        }
    }
    struct StrictVisitor;
    impl<'de> Visitor<'de> for StrictVisitor {
        type Value = Value;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a JSON value")
        }
        fn visit_bool<E>(self, v: bool) -> std::result::Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> std::result::Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_u64<E>(self, v: u64) -> std::result::Result<Value, E> {
            Ok(Value::from(v))
        }
        fn visit_f64<E>(self, v: f64) -> std::result::Result<Value, E> {
            Ok(serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null))
        }
        fn visit_str<E>(self, v: &str) -> std::result::Result<Value, E> {
            Ok(Value::String(v.to_string()))
        }
        fn visit_string<E>(self, v: String) -> std::result::Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> std::result::Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> std::result::Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
            let mut out = Vec::new();
            while let Some(v) = seq.next_element_seed(Strict)? {
                out.push(v);
            }
            Ok(Value::Array(out))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
            let mut out = serde_json::Map::new();
            while let Some(k) = map.next_key::<String>()? {
                if out.contains_key(&k) {
                    return Err(de::Error::custom(format!("duplicate key \"{k}\"")));
                }
                let v = map.next_value_seed(Strict)?;
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }
    }
    let mut de = serde_json::Deserializer::from_str(text);
    let v = Strict.deserialize(&mut de).map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(v)
}

/// Package names declared in all dependency sections.
pub fn declared(pkg: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for s in DEP_SECTIONS {
        if let Some(o) = pkg.get(s).and_then(|v| v.as_object()) {
            for (k, v) in o {
                out.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
    }
    out
}

fn fnv1a(bytes: &[u8], mut h: u64) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Fingerprint of everything that determines what `npm install` puts in node_modules.
pub fn fingerprint(pkg: &Value, lockfile: Option<&[u8]>) -> String {
    let mut h = 0xcbf29ce484222325u64;
    for k in FINGERPRINT_KEYS {
        // serde_json::Map is ordered (BTreeMap), so serialization is canonical.
        let part = pkg.get(k).map(|v| v.to_string()).unwrap_or_default();
        h = fnv1a(k.as_bytes(), h);
        h = fnv1a(part.as_bytes(), h);
    }
    h = fnv1a(b"lock", h);
    if let Some(l) = lockfile {
        h = fnv1a(l, h);
    }
    format!("{h:016x}")
}

fn read_lock(root: &Path) -> Option<Vec<u8>> {
    std::fs::read(root.join("package-lock.json")).ok()
}

pub fn read_install_state(root: &Path) -> Option<InstallState> {
    std::fs::read_to_string(root.join(STATE_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Record a successful install. Called after npm exits with 0 (npm may have rewritten the
/// lockfile, so the fingerprint is computed from the files as they are now).
pub fn record_install(root: &Path) -> Result<InstallState> {
    let text = std::fs::read_to_string(root.join("package.json"))?;
    let pkg = parse_strict(&text).map_err(|e| GeniusError::Validation(format!("package.json: {e}")))?;
    let st = InstallState { fingerprint: fingerprint(&pkg, read_lock(root).as_deref()), packages: declared(&pkg).into_keys().collect(), installed_at: crate::db::now() };
    let file = root.join(STATE_FILE);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, serde_json::to_string_pretty(&st)?)?;
    Ok(st)
}

fn installed(root: &Path, name: &str) -> bool {
    let mut p = root.join("node_modules");
    for part in name.split('/') {
        p.push(part);
    }
    p.join("package.json").is_file()
}

/// Compute the dependency state from disk. `trusted` lists packages that may be installed
/// without asking (the Genius template's own packages).
pub fn status(root: &Path, trusted: &[&str]) -> DepStatus {
    let mk = |state, reason: String| DepStatus { state, reason, missing: vec![], packages: vec![], new_packages: vec![], fingerprint: None };
    let text = match std::fs::read_to_string(root.join("package.json")) {
        Ok(t) => t,
        Err(_) => return mk(DepState::NoPackageJson, "This project has no package.json.".into()),
    };
    let pkg = match parse_strict(&text) {
        Ok(v) if v.is_object() => v,
        Ok(_) => return mk(DepState::InvalidPackageJson, "package.json must contain a JSON object.".into()),
        Err(e) => return mk(DepState::InvalidPackageJson, format!("package.json is not valid: {e}.")),
    };
    for s in DEP_SECTIONS {
        if let Some(sec) = pkg.get(s) {
            let ok = sec.as_object().is_some_and(|o| o.values().all(|v| v.is_string()));
            if !ok {
                return mk(DepState::InvalidPackageJson, format!("\"{s}\" in package.json must map package names to version strings."));
            }
        }
    }
    let decl = declared(&pkg);
    let packages: Vec<String> = decl.keys().cloned().collect();
    let fp = fingerprint(&pkg, read_lock(root).as_deref());
    let state = read_install_state(root);
    let approved: BTreeSet<&str> = state.as_ref().map(|s| s.packages.iter().map(String::as_str).collect()).unwrap_or_default();
    let new_packages: Vec<String> = packages.iter().filter(|p| !approved.contains(p.as_str()) && !trusted.contains(&p.as_str())).cloned().collect();
    let missing: Vec<String> = packages.iter().filter(|p| !installed(root, p)).cloned().collect();
    let base = DepStatus { state: DepState::Ready, reason: String::new(), missing: missing.clone(), packages, new_packages, fingerprint: Some(fp.clone()) };
    if decl.is_empty() && !root.join("node_modules").is_dir() {
        return DepStatus { reason: "No dependencies are declared.".into(), ..base };
    }
    if !root.join("node_modules").is_dir() {
        return DepStatus { state: DepState::NeedsInstall, reason: "Dependencies have not been installed yet.".into(), ..base };
    }
    if !missing.is_empty() {
        let list = missing.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        let more = if missing.len() > 6 { format!(" and {} more", missing.len() - 6) } else { String::new() };
        return DepStatus { state: DepState::NeedsInstall, reason: format!("{} declared package(s) are not installed: {list}{more}.", missing.len()), ..base };
    }
    match state {
        Some(s) if s.fingerprint != fp => DepStatus { state: DepState::NeedsInstall, reason: "package.json or package-lock.json changed since the last install.".into(), ..base },
        _ => DepStatus { reason: "All declared packages are installed.".into(), ..base },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, content: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn strict_parse_rejects_duplicates_at_any_depth() {
        assert!(parse_strict(r#"{"a":1,"b":{"c":[1,{"d":2}]}}"#).is_ok());
        let e = parse_strict(r#"{"dependencies":{"react":"^19"},"dependencies":{"uuid":"^11"}}"#).unwrap_err();
        assert!(e.contains("duplicate key \"dependencies\""), "{e}");
        let e = parse_strict(r#"{"dependencies":{"uuid":"^9","uuid":"^11"}}"#).unwrap_err();
        assert!(e.contains("uuid"), "{e}");
        assert!(parse_strict(r#"{"a":1,}"#).is_err());
        assert!(parse_strict(r#"{"a":1} trailing"#).is_err());
    }

    #[test]
    fn status_follows_the_files_not_a_flag() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        assert_eq!(status(r, &[]).state, DepState::NoPackageJson);
        write(r, "package.json", r#"{"dependencies":{"react":"^19.0.0"}}"#);
        let s = status(r, &["react"]);
        assert_eq!(s.state, DepState::NeedsInstall);
        assert!(s.new_packages.is_empty(), "template packages are trusted");
        // simulate a successful install
        write(r, "node_modules/react/package.json", "{}");
        record_install(r).unwrap();
        assert_eq!(status(r, &[]).state, DepState::Ready);
        // editing a script does not require reinstalling
        write(r, "package.json", r#"{"scripts":{"dev":"vite"},"dependencies":{"react":"^19.0.0"}}"#);
        assert_eq!(status(r, &[]).state, DepState::Ready);
        // adding a dependency does, and names the missing package
        write(r, "package.json", r#"{"dependencies":{"react":"^19.0.0","uuid":"^11.0.0"}}"#);
        let s = status(r, &[]);
        assert_eq!(s.state, DepState::NeedsInstall);
        assert_eq!(s.missing, vec!["uuid".to_string()]);
        assert_eq!(s.new_packages, vec!["uuid".to_string()]);
        // a version change of an installed package is also detected
        write(r, "node_modules/uuid/package.json", "{}");
        write(r, "package.json", r#"{"dependencies":{"react":"^18.0.0","uuid":"^11.0.0"}}"#);
        assert_eq!(status(r, &[]).state, DepState::NeedsInstall);
        record_install(r).unwrap();
        assert_eq!(status(r, &[]).state, DepState::Ready);
        // lockfile changes are detected
        write(r, "package-lock.json", "{\"lockfileVersion\":3}");
        assert_eq!(status(r, &[]).state, DepState::NeedsInstall);
        // scoped packages resolve to nested folders
        write(r, "package.json", r#"{"dependencies":{"@supabase/supabase-js":"^2"}}"#);
        assert_eq!(status(r, &[]).missing, vec!["@supabase/supabase-js".to_string()]);
        // duplicate keys are reported, never silently resolved
        write(r, "package.json", r#"{"dependencies":{"react":"^19"},"dependencies":{"uuid":"^11"}}"#);
        let s = status(r, &[]);
        assert_eq!(s.state, DepState::InvalidPackageJson);
        assert!(s.reason.contains("duplicate key"), "{}", s.reason);
    }

    #[test]
    fn deleting_node_modules_resets_state() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        write(r, "package.json", r#"{"dependencies":{"react":"^19.0.0"}}"#);
        write(r, "node_modules/react/package.json", "{}");
        record_install(r).unwrap();
        std::fs::remove_dir_all(r.join("node_modules")).unwrap();
        let s = status(r, &[]);
        assert_eq!(s.state, DepState::NeedsInstall);
        assert!(s.reason.contains("not been installed"));
        assert_eq!(s.new_packages, vec!["react".to_string()], "approval record lived in node_modules and is gone");
    }
}
