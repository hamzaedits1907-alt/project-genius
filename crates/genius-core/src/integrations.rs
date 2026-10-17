//! Integration framework: a registry of external services, each declaring its identity,
//! authentication method, permissions, capabilities, credential storage, health check and
//! disconnect behaviour. Connection state is derived from real checks and stored metadata —
//! the agent discovers integrations only from this registry and never assumes access.
//!
//! Secrets (tokens, keys) live in the OS credential store via `SecretStore`; only non-secret
//! metadata (selected project, last check result) is stored in SQLite.

use crate::app::Genius;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub auth_method: &'static str,
    pub permissions: &'static [&'static str],
    pub capabilities: &'static [&'static str],
    pub credential_storage: &'static str,
    pub disconnect: &'static str,
    pub docs: &'static str,
}

pub const REGISTRY: &[IntegrationSpec] = &[
    IntegrationSpec {
        id: "github",
        name: "GitHub",
        auth_method: "GitHub CLI (gh) OAuth device flow, or a fine-grained personal access token piped to gh",
        permissions: &["repo (read/write the repositories you authorise)", "read:org (list organisations)"],
        capabilities: &["list repositories", "create repository", "clone", "push and verify", "publish project"],
        credential_storage: "kept by gh in the OS credential store; Genius never stores the token",
        disconnect: "gh auth logout (token revoked locally; revoke the OAuth app at github.com/settings/applications)",
        docs: "https://cli.github.com/manual/gh_auth_login",
    },
    IntegrationSpec {
        id: "supabase",
        name: "Supabase",
        auth_method: "Supabase personal access token (Management API)",
        permissions: &["Management API access for the projects your account can see"],
        capabilities: &[
            "list projects",
            "select project",
            "read project URL and publishable (anon) key",
            "write client env config (.env.local, gitignored)",
            "generate migrations as reviewable changesets",
            "apply SQL migrations remotely (only after your approval)",
            "health check",
        ],
        credential_storage: "access token in the OS credential store; the service-role key is never fetched or stored",
        disconnect: "removes the token from the OS store; revoke it at supabase.com/dashboard/account/tokens",
        docs: "https://supabase.com/docs/reference/api/introduction",
    },
];

impl Genius {
    /// Status of every registered integration from stored metadata (no network).
    pub fn integration_statuses(&self) -> Vec<Value> {
        REGISTRY
            .iter()
            .map(|spec| {
                let meta = self.db.get_integration(spec.id).ok().flatten().unwrap_or(Value::Null);
                let has_secret = match spec.id {
                    "supabase" => self.secrets.get(crate::supabase::TOKEN_NAME).ok().flatten().is_some(),
                    _ => false,
                };
                let status = match spec.id {
                    "github" => meta.get("status").and_then(|s| s.as_str()).unwrap_or("unknown").to_string(),
                    _ if has_secret => meta.get("status").and_then(|s| s.as_str()).unwrap_or("credentials_stored").to_string(),
                    _ => "not_connected".to_string(),
                };
                json!({ "id": spec.id, "name": spec.name, "status": status, "meta": meta, "spec": spec })
            })
            .collect()
    }
}
