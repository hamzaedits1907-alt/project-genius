//! Supabase integration via the documented Management API (https://api.supabase.com/v1),
//! authenticated with a personal access token stored in the OS credential store.
//!
//! Least privilege and secret hygiene:
//! * Only the project URL and the **publishable (anon) key** are written to the app, into
//!   `.env.local` (gitignored by Genius). The service-role key is never stored, logged, sent
//!   to the model, or written anywhere: it is dropped as soon as the key list is parsed.
//! * Code changes (client library, migrations, generated types) are reviewable changesets.
//! * Remote SQL (migrations) runs only after an explicit approval showing the exact SQL —
//!   regardless of approval policy settings.

use crate::app::Genius;
use crate::changes::ChangeStore;
use crate::db::now;
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

pub const TOKEN_NAME: &str = "supabase-access-token";
const DEFAULT_API: &str = "https://api.supabase.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SupabaseProject {
    pub reference: String,
    pub name: String,
    pub region: String,
    pub status: String,
    pub organization_id: String,
}

fn parse_projects(v: &Value) -> Vec<SupabaseProject> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    Some(SupabaseProject {
                        reference: p.get("id").or_else(|| p.get("ref")).and_then(|x| x.as_str())?.to_string(),
                        name: p.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        region: p.get("region").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        status: p.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                        organization_id: p.get("organization_id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Pick the publishable key from the api-keys response; everything else is discarded.
pub fn publishable_key(v: &Value) -> Option<String> {
    let keys = v.as_array()?;
    let pick = |names: &[&str]| keys.iter().find(|k| k.get("name").and_then(|n| n.as_str()).is_some_and(|n| names.contains(&n))).and_then(|k| k.get("api_key").and_then(|x| x.as_str())).map(String::from);
    pick(&["anon", "publishable"]).or_else(|| {
        keys.iter().find(|k| k.get("type").and_then(|t| t.as_str()) == Some("publishable")).and_then(|k| k.get("api_key").and_then(|x| x.as_str())).map(String::from)
    })
}

fn valid_ref(r: &str) -> Result<()> {
    if r.len() >= 8 && r.len() <= 40 && r.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        Ok(())
    } else {
        Err(GeniusError::Validation(format!("'{r}' is not a valid Supabase project reference")))
    }
}

fn migration_name(name: &str) -> String {
    let s: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "migration".into() } else { s.chars().take(60).collect() }
}

impl Genius {
    fn supabase_api(&self) -> Result<String> {
        let base = self.db.get_setting("supabase_api_base")?.and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| DEFAULT_API.into());
        let b = base.trim_end_matches('/').to_string();
        if b.starts_with("https://") || b.starts_with("http://127.0.0.1") || b.starts_with("http://localhost") {
            Ok(b)
        } else {
            Err(GeniusError::Validation("Supabase API base must be https".into()))
        }
    }

    fn supabase_token(&self) -> Result<String> {
        self.secrets.get(TOKEN_NAME)?.ok_or_else(|| GeniusError::Validation("Supabase is not connected. Settings → Integrations → Supabase → paste a personal access token.".into()))
    }

    async fn supabase_request(&self, method: reqwest::Method, path: &str, token: &str, body: Option<Value>) -> Result<Value> {
        let url = format!("{}{path}", self.supabase_api()?);
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().map_err(|e| GeniusError::Internal(e.to_string()))?;
        let mut req = client.request(method, &url).bearer_auth(token).header("accept", "application/json");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|e| GeniusError::Integration(crate::redact::redact(&e.to_string())))?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(|e| GeniusError::Integration(e.to_string()))?;
        match status {
            200..=299 => Ok(serde_json::from_str(&text).unwrap_or(Value::String(text))),
            401 => Err(GeniusError::Integration("Supabase rejected the access token (401). Create a new token at supabase.com/dashboard/account/tokens.".into())),
            403 => Err(GeniusError::Integration("The Supabase token is not allowed to do this (403). Check that your account has access to this project.".into())),
            404 => Err(GeniusError::NotFound(format!("Supabase resource not found: {path}"))),
            429 => Err(GeniusError::Integration("Supabase rate limit reached; try again in a minute.".into())),
            _ => Err(GeniusError::Integration(format!("Supabase API error {status}: {}", crate::redact::redact(&text.chars().take(300).collect::<String>())))),
        }
    }

    /// Validate the token with a real API call before storing it.
    pub async fn supabase_connect(&self, token: &str) -> Result<Value> {
        let token = token.trim();
        if token.len() < 20 || token.contains(char::is_whitespace) {
            return Err(GeniusError::Validation("That does not look like a Supabase access token (they start with sbp_).".into()));
        }
        crate::redact::register_secret(token);
        let mut ev = self.bus.start(None, None, EventKind::Task, EventStatus::Running, "Connecting Supabase", json!({}));
        let res = self.supabase_request(reqwest::Method::GET, "/v1/projects", token, None).await;
        match res {
            Ok(v) => {
                let projects = parse_projects(&v);
                self.secrets.set(TOKEN_NAME, token)?;
                let meta = json!({ "status": "connected", "checkedAt": now(), "projects": projects.len(), "storage": self.secrets.backend_name() });
                self.db.set_integration("supabase", &meta)?;
                ev.update(EventStatus::Completed, json!({ "projects": projects.len() }));
                Ok(json!({ "connected": true, "projects": projects }))
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    pub fn supabase_disconnect(&self) -> Result<Value> {
        if let Some(t) = self.secrets.get(TOKEN_NAME)? {
            crate::redact::unregister_secret(&t);
        }
        self.secrets.delete(TOKEN_NAME)?;
        self.db.delete_integration("supabase")?;
        self.bus.record(None, None, EventKind::Task, EventStatus::Completed, "Disconnected Supabase (token removed from the credential store)", json!({}));
        Ok(json!({ "connected": false, "note": "Also revoke the token at supabase.com/dashboard/account/tokens if you no longer need it." }))
    }

    pub async fn supabase_projects(&self) -> Result<Vec<SupabaseProject>> {
        let token = self.supabase_token()?;
        let v = self.supabase_request(reqwest::Method::GET, "/v1/projects", &token, None).await?;
        Ok(parse_projects(&v))
    }

    /// Link a Genius project to a Supabase project (non-secret metadata only).
    pub async fn supabase_link(&self, project_id: &str, reference: &str) -> Result<Value> {
        valid_ref(reference)?;
        let token = self.supabase_token()?;
        let p = self.supabase_request(reqwest::Method::GET, &format!("/v1/projects/{reference}"), &token, None).await?;
        let link = json!({ "ref": reference, "name": p.get("name"), "region": p.get("region"), "status": p.get("status"), "url": format!("https://{reference}.supabase.co"), "linkedAt": now() });
        self.db.patch_project_status(project_id, &json!({ "supabase": link }))?;
        self.bus.record(Some(project_id), None, EventKind::Task, EventStatus::Completed, format!("Linked Supabase project {reference}"), link.clone());
        Ok(link)
    }

    fn supabase_link_of(&self, project_id: &str) -> Result<String> {
        let p = self.db.get_project(project_id)?;
        p.status.get("supabase").and_then(|s| s.get("ref")).and_then(|r| r.as_str()).map(String::from).ok_or_else(|| GeniusError::Validation("This project is not linked to a Supabase project yet.".into()))
    }

    /// Health and configuration check: remote project status, env config presence, client
    /// library presence, gitignore protection. Read-only.
    pub async fn supabase_status(&self, project_id: &str) -> Result<Value> {
        let connected = self.secrets.get(TOKEN_NAME)?.is_some();
        let p = self.db.get_project(project_id)?;
        let link = p.status.get("supabase").cloned();
        let ws = self.workspace(project_id)?;
        let env = ws.read_text_opt(".env.local")?.unwrap_or_default();
        let env_ok = env.contains("VITE_SUPABASE_URL=") && env.contains("VITE_SUPABASE_ANON_KEY=");
        let pkg = ws.read_text_opt("package.json")?.unwrap_or_default();
        let client_dep = pkg.contains("\"@supabase/supabase-js\"");
        let gitignored = self.gitignore_missing(project_id)?.iter().all(|m| m != ".env.*" && m != ".env");
        let leaked_service_key = ws.search("service_role", 5)?.into_iter().map(|h| h.path).collect::<Vec<_>>();
        let mut remote = Value::Null;
        if let (true, Some(r)) = (connected, link.as_ref().and_then(|l| l.get("ref")).and_then(|r| r.as_str())) {
            remote = match self.supabase_request(reqwest::Method::GET, &format!("/v1/projects/{r}"), &self.supabase_token()?, None).await {
                Ok(v) => json!({ "ok": true, "status": v.get("status") }),
                Err(e) => json!({ "ok": false, "error": e.to_string() }),
            };
        }
        Ok(json!({
            "connected": connected,
            "linked": link,
            "remote": remote,
            "envConfigured": env_ok,
            "clientLibrary": client_dep,
            "envGitignored": gitignored,
            "serviceRoleMentions": leaked_service_key,
            "manualSteps": [
                "Enable the auth providers you need (Email, OAuth) in the Supabase dashboard → Authentication → Providers.",
                "Set the Site URL / redirect URLs for local development (http://localhost:5173) in Authentication → URL Configuration."
            ],
        }))
    }

    /// Write VITE_SUPABASE_URL and the publishable key to .env.local (gitignored first), and
    /// propose the client library + `src/lib/supabase.ts` as a reviewable changeset.
    pub async fn supabase_configure(&self, project_id: &str) -> Result<Value> {
        let reference = self.supabase_link_of(project_id)?;
        let token = self.supabase_token()?;
        let keys = self.supabase_request(reqwest::Method::GET, &format!("/v1/projects/{reference}/api-keys"), &token, None).await?;
        let anon = publishable_key(&keys).ok_or_else(|| GeniusError::Validation("Supabase did not return a publishable (anon) key for this project.".into()));
        drop(keys); // contains the service-role key: never kept
        let anon = anon?;
        crate::redact::register_secret(&anon);
        let added = self.ensure_gitignore(project_id)?;
        let ws = self.workspace(project_id)?;
        let mut env = ws.read_text_opt(".env.local")?.unwrap_or_default();
        env = env.lines().filter(|l| !l.starts_with("VITE_SUPABASE_URL=") && !l.starts_with("VITE_SUPABASE_ANON_KEY=")).map(|l| format!("{l}\n")).collect();
        env.push_str(&format!("VITE_SUPABASE_URL=https://{reference}.supabase.co\nVITE_SUPABASE_ANON_KEY={anon}\n"));
        ws.write_text(".env.local", &env)?;
        self.bus.record(Some(project_id), None, EventKind::FileWrite, EventStatus::Completed, "Wrote Supabase client configuration to .env.local (gitignored)", json!({ "variables": ["VITE_SUPABASE_URL", "VITE_SUPABASE_ANON_KEY"], "gitignoreAdded": added }));

        // Client library + typed client module as a proposal.
        let mut writes = Vec::new();
        if let Some(pkg) = ws.read_text_opt("package.json")? {
            let mut v: Value = serde_json::from_str(&pkg).map_err(|e| GeniusError::Validation(format!("package.json: {e}")))?;
            if v.get("dependencies").and_then(|d| d.get("@supabase/supabase-js")).is_none() {
                v["dependencies"]["@supabase/supabase-js"] = json!("^2.45.0");
                writes.push(("package.json".to_string(), format!("{}\n", serde_json::to_string_pretty(&v)?)));
            }
        }
        if !ws.exists("src/lib/supabase.ts") {
            writes.push(("src/lib/supabase.ts".into(), SUPABASE_CLIENT.into()));
        }
        if !ws.exists("src/vite-env.d.ts") || !ws.read_text("src/vite-env.d.ts").unwrap_or_default().contains("VITE_SUPABASE_URL") {
            let mut d = ws.read_text_opt("src/vite-env.d.ts")?.unwrap_or_else(|| "/// <reference types=\"vite/client\" />\n".into());
            d.push_str("\ninterface ImportMetaEnv {\n  readonly VITE_SUPABASE_URL: string;\n  readonly VITE_SUPABASE_ANON_KEY: string;\n}\n");
            writes.push(("src/vite-env.d.ts".into(), d));
        }
        let changes = crate::changes::compute_changes(&ws, &writes, &[])?;
        let cs = if changes.is_empty() {
            None
        } else {
            let cs = ChangeStore::new(&self.db).create(project_id, None, "change", "Add Supabase client", "Adds @supabase/supabase-js and a typed client in src/lib/supabase.ts that reads the public URL and anon key from Vite env variables. The service-role key is never used in browser code.", &changes)?;
            self.bus.record(Some(project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, "Review proposed changes: Add Supabase client", json!({ "changesetId": cs.id, "files": cs.files }));
            Some(cs.id)
        };
        Ok(json!({ "envWritten": ".env.local", "variables": ["VITE_SUPABASE_URL", "VITE_SUPABASE_ANON_KEY"], "changesetId": cs, "gitignoreAdded": added }))
    }

    /// Propose a migration file as a reviewable changeset (nothing runs remotely).
    pub fn supabase_propose_migration(&self, project_id: &str, name: &str, sql: &str) -> Result<Value> {
        if sql.trim().is_empty() {
            return Err(GeniusError::Validation("the migration SQL is empty".into()));
        }
        let ws = self.workspace(project_id)?;
        let path = format!("supabase/migrations/{}_{}.sql", chrono::Utc::now().format("%Y%m%d%H%M%S"), migration_name(name));
        let changes = crate::changes::compute_changes(&ws, &[(path.clone(), format!("{}\n", sql.trim_end()))], &[])?;
        let cs = ChangeStore::new(&self.db).create(project_id, None, "change", &format!("Migration: {name}"), "New SQL migration file. Review it, apply it to the project, then apply it to the remote database separately.", &changes)?;
        self.bus.record(Some(project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, format!("Review proposed migration: {name}"), json!({ "changesetId": cs.id, "files": cs.files }));
        Ok(json!({ "changesetId": cs.id, "path": path }))
    }

    pub fn supabase_migrations(&self, project_id: &str) -> Result<Vec<Value>> {
        let ws = self.workspace(project_id)?;
        let applied = self.db.get_project(project_id)?.status.get("supabaseApplied").cloned().unwrap_or(json!({}));
        Ok(ws
            .list_files()?
            .into_iter()
            .filter(|f| f.path.starts_with("supabase/migrations/") && f.path.ends_with(".sql"))
            .map(|f| json!({ "path": f.path, "size": f.size, "appliedRemote": applied.get(&f.path) }))
            .collect())
    }

    /// Run a migration file against the linked remote database — always behind an explicit
    /// approval that shows the target project and the exact SQL.
    pub async fn supabase_apply_migration(&self, project_id: &str, path: &str, cancel: &CancellationToken) -> Result<Value> {
        if !path.starts_with("supabase/migrations/") || !path.ends_with(".sql") {
            return Err(GeniusError::Validation("only files in supabase/migrations/*.sql can be applied".into()));
        }
        let reference = self.supabase_link_of(project_id)?;
        let token = self.supabase_token()?;
        let ws = self.workspace(project_id)?;
        let sql = ws.read_text(path)?;
        self.approvals
            .request(&self.bus, Some(project_id), None, "remote_database", crate::settings::Policy::Ask, &format!("Run migration on Supabase project {reference}"), json!({ "project": reference, "file": path, "sql": sql, "warning": "This changes the REMOTE database. It cannot be undone automatically." }), cancel)
            .await?;
        let mut ev = self.bus.start(Some(project_id), None, EventKind::Command, EventStatus::Running, format!("Applying {path} to Supabase {reference}"), json!({ "file": path }));
        match self.supabase_request(reqwest::Method::POST, &format!("/v1/projects/{reference}/database/query"), &token, Some(json!({ "query": sql }))).await {
            Ok(v) => {
                self.db.patch_project_status(project_id, &json!({ "supabaseApplied": { path: now() } }))?;
                ev.update(EventStatus::Completed, json!({ "result": crate::events::redact_value(v.clone()) }));
                Ok(json!({ "applied": true, "file": path, "result": v }))
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    /// Generate TypeScript types from the linked database and propose them as a changeset.
    pub async fn supabase_generate_types(&self, project_id: &str) -> Result<Value> {
        let reference = self.supabase_link_of(project_id)?;
        let token = self.supabase_token()?;
        let v = self.supabase_request(reqwest::Method::GET, &format!("/v1/projects/{reference}/types/typescript"), &token, None).await?;
        let types = v.get("types").and_then(|t| t.as_str()).ok_or_else(|| GeniusError::Validation("Supabase returned no types".into()))?;
        let ws = self.workspace(project_id)?;
        let changes = crate::changes::compute_changes(&ws, &[("src/types/supabase.ts".into(), types.to_string())], &[])?;
        if changes.is_empty() {
            return Ok(json!({ "changesetId": null, "note": "types are already up to date" }));
        }
        let cs = ChangeStore::new(&self.db).create(project_id, None, "change", "Update Supabase database types", "Generated from the linked database schema by the Supabase Management API.", &changes)?;
        self.bus.record(Some(project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, "Review proposed changes: Update Supabase database types", json!({ "changesetId": cs.id, "files": cs.files }));
        Ok(json!({ "changesetId": cs.id }))
    }
}

const SUPABASE_CLIENT: &str = r#"import { createClient } from '@supabase/supabase-js';

// Public client configuration (safe in the browser). Never put the service-role key here:
// it bypasses Row Level Security and must only be used on a server.
const url = import.meta.env.VITE_SUPABASE_URL;
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;

if (!url || !anonKey) {
  throw new Error('Supabase is not configured: set VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY in .env.local');
}

export const supabase = createClient(url, anonKey);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_publishable_key_is_selected() {
        let v = json!([{ "name": "service_role", "api_key": "SERVICE" }, { "name": "anon", "api_key": "ANON" }]);
        assert_eq!(publishable_key(&v).as_deref(), Some("ANON"));
        let v = json!([{ "name": "service_role", "api_key": "SERVICE" }]);
        assert_eq!(publishable_key(&v), None, "never falls back to the service-role key");
    }

    #[test]
    fn refs_and_names_are_validated() {
        assert!(valid_ref("abcdefghijklmnopqrst").is_ok());
        assert!(valid_ref("../etc").is_err());
        assert!(valid_ref("ABC").is_err());
        assert_eq!(migration_name("Create profiles!"), "create_profiles");
        assert!(!SUPABASE_CLIENT.contains("SERVICE_ROLE_KEY"));
    }
}
