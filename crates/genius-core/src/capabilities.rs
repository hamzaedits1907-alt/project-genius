//! Genius's self-model: a structured description of what it can do *right now*, derived only
//! from runtime state (settings, stored key, model probes, detected tools, integration
//! status, approval policies, running tasks, recent operation outcomes).
//!
//! Nothing here is hardcoded as "available": every capability carries a status of
//! `available`, `unavailable` or `unverified` and the evidence for it. The agent answers
//! "what can you do?" questions from this report (via its `capabilities` tool), and the UI
//! shows the same report.

use crate::app::Genius;
use crate::error::Result;
use crate::events::EventStatus;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
    pub label: String,
    /// available | unavailable | unverified
    pub status: String,
    pub evidence: String,
    /// What the user can do to enable it, when it is not available.
    #[serde(default)]
    pub remedy: String,
}

fn cap(id: &str, label: &str, status: &str, evidence: impl Into<String>, remedy: impl Into<String>) -> Capability {
    Capability { id: id.into(), label: label.into(), status: status.into(), evidence: evidence.into(), remedy: remedy.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityReport {
    pub generated_at: String,
    pub provider: Value,
    pub capabilities: Vec<Capability>,
    pub agent_tools: Vec<Value>,
    pub integrations: Vec<Value>,
    pub permissions: Value,
    pub project: Option<Value>,
    pub limitations: Vec<String>,
}

impl CapabilityReport {
    /// Compact text for model prompts.
    pub fn render(&self) -> String {
        let mut s = format!("GENIUS RUNTIME SELF-DESCRIPTION (generated {} from live state; authoritative):\n", self.generated_at);
        s.push_str(&format!("provider: {}\n", self.provider));
        for c in &self.capabilities {
            s.push_str(&format!("- [{}] {}: {}{}\n", c.status, c.label, c.evidence, if c.remedy.is_empty() { String::new() } else { format!(" (to enable: {})", c.remedy) }));
        }
        s.push_str(&format!("integrations: {}\n", Value::Array(self.integrations.clone())));
        s.push_str(&format!("approval policies: {}\n", self.permissions));
        if let Some(p) = &self.project {
            s.push_str(&format!("project state: {p}\n"));
        }
        s.push_str(&format!("known limitations: {}\n", self.limitations.join("; ")));
        s
    }
}

impl Genius {
    /// Fast, synchronous part of the self-model (no process spawning or network).
    pub fn capability_report_quick(&self, project_id: Option<&str>) -> Result<CapabilityReport> {
        self.build_capability_report(project_id, None, None)
    }

    /// Full self-model including detected local tools and GitHub status (spawns `--version` probes).
    pub async fn capability_report(&self, project_id: Option<&str>) -> Result<CapabilityReport> {
        let tools = crate::process::detect_tools().await;
        let gh = self.github_status().await;
        self.build_capability_report(project_id, Some(tools), Some(serde_json::to_value(gh)?))
    }

    fn build_capability_report(&self, project_id: Option<&str>, tools: Option<Vec<crate::process::ToolStatus>>, gh: Option<Value>) -> Result<CapabilityReport> {
        let s = self.settings()?;
        let key = self.key_status();
        let probes = self.probes();
        let probe = s.model.as_ref().and_then(|m| probes.get(m).cloned());
        let vision = s.model.as_ref().and_then(|m| self.vision_probe(m));
        let mut caps = Vec::new();

        caps.push(if key.stored {
            cap("provider_key", "NVIDIA API key", "available", format!("stored in {} ({})", key.backend, key.hint.clone().unwrap_or_default()), "")
        } else {
            cap("provider_key", "NVIDIA API key", "unavailable", "no key is stored", "Settings → AI provider → paste your NVIDIA API key")
        });
        caps.push(match (&s.model, &probe) {
            (None, _) => cap("chat", "Chat with a model", "unavailable", "no model selected", "Settings → AI provider → Refresh models and pick one"),
            (Some(m), Some(p)) if p.chat => cap("chat", "Chat with a model", if key.stored { "available" } else { "unavailable" }, format!("{m} answered a verification probe ({} ms)", p.latency_ms), if key.stored { "" } else { "save an API key" }),
            (Some(m), _) => cap("chat", "Chat with a model", if key.stored { "unverified" } else { "unavailable" }, format!("{m} selected but not verified with a real request yet"), "Settings → AI provider → Verify"),
        });
        caps.push(match &probe {
            Some(p) if p.streaming => cap("streaming", "Streaming responses", "available", "verification probe streamed tokens", ""),
            Some(_) => cap("streaming", "Streaming responses", "unavailable", "probe did not stream; non-streaming fallback is used", ""),
            None => cap("streaming", "Streaming responses", "unverified", "model not probed", "Verify the model"),
        });
        caps.push(match vision {
            Some(true) => cap("vision", "Image input (screenshots)", "available", "the model accepted an image in a verification request", ""),
            Some(false) => cap("vision", "Image input (screenshots)", "unavailable", "the model rejected image input in a verification request", "choose a vision-capable model (e.g. one with 'vision' or 'vl' in its id) and verify it"),
            None => cap("vision", "Image input (screenshots)", "unverified", "image input has not been tested with this model", "Settings → AI provider → Test image input"),
        });
        caps.push(cap(
            "native_tools",
            "Provider-native function calling",
            "unavailable",
            "not used: support differs per model and is unverified; Genius uses its own validated text tool protocol, which works with any chat model",
            "",
        ));
        caps.push(cap("project_tools", "Read, search and inspect project files", if project_id.is_some() { "available" } else { "unavailable" }, if project_id.is_some() { "sandboxed to the project folder; secrets and .env contents are never read" } else { "no project is open" }, if project_id.is_some() { "" } else { "open a project" }));
        caps.push(cap("edits", "Propose and apply code changes", if project_id.is_some() { "available" } else { "unavailable" }, match s.approvals.apply_ai_edits {
            crate::settings::Policy::Ask => "changes are proposed as reviewable diffs; you apply or reject them",
            crate::settings::Policy::Auto => "changes are applied automatically by your settings (still revertible in History)",
        }, ""));
        if let Some(tools) = &tools {
            let node = tools.iter().any(|t| t.name == "npm" && t.found);
            caps.push(if node {
                cap("checks", "Run builds and tests", "available", "npm found; only the project's install/build/test/typecheck scripts can run, with approvals for untrusted scripts", "")
            } else {
                cap("checks", "Run builds and tests", "unavailable", "npm was not found on PATH", "install Node.js 20+ and restart Genius")
            });
            let git = tools.iter().any(|t| t.name == "git" && t.found);
            caps.push(cap("git", "Git source control", if git { "available" } else { "unavailable" }, if git { "git found" } else { "git was not found" }, if git { "" } else { "install Git from https://git-scm.com" }));
        } else {
            caps.push(cap("checks", "Run builds and tests", "unverified", "tool detection not run in this quick report", ""));
        }
        caps.push(cap("shell", "Arbitrary shell commands", "unavailable", "deliberately not provided: only fixed, allowlisted operations run", ""));
        caps.push(if s.internet_access {
            cap("web", "Search the web and read pages", "available", "internet access is on: the agent can use web_search (DuckDuckGo, no key) and fetch_url (public http/https pages only; local and private network addresses are blocked; pages are treated as untrusted data). Results depend on your network.", "")
        } else {
            cap("web", "Search the web and read pages", "unavailable", "internet access is turned off in Settings", "Settings → AI provider → turn on internet access")
        });
        caps.push(cap(
            "subagents",
            "Specialist subagents",
            "available",
            "the main agent can delegate bounded, read-only analyses to: architect (implementation plan), designer (visual design direction), reviewer (code review), ui (UI/accessibility audit), tester (test cases); multi-file proposals are reviewed by the reviewer automatically",
            "",
        ));
        caps.push(cap("large_builds", "Build whole apps and multi-page websites", if project_id.is_some() { "available" } else { "unavailable" }, "the agent writes files in batches (write_files) and proposes them as one reviewable changeset; it can ask clarifying questions and shows a live progress checklist", if project_id.is_some() { "" } else { "open a project" }));

        let mut integrations = self.integration_statuses();
        if let Some(gh) = gh {
            let connected = gh.get("connected").and_then(|v| v.as_bool()).unwrap_or(false);
            integrations.retain(|i| i.get("id").and_then(|v| v.as_str()) != Some("github"));
            integrations.insert(0, json!({
                "id": "github",
                "status": if connected { "connected" } else { "not_connected" },
                "detail": gh.get("detail").cloned().unwrap_or(Value::Null),
                "login": gh.get("login").cloned().unwrap_or(Value::Null),
                "scopes": gh.get("scopes").cloned().unwrap_or(Value::Null),
            }));
        }

        let project = match project_id {
            Some(pid) => {
                let p = self.db.get_project(pid)?;
                let tasks = self.active_tasks(Some(pid));
                let recent: Vec<Value> = self
                    .bus
                    .list(Some(pid), 40)?
                    .into_iter()
                    .filter(|e| e.status.is_terminal() || e.status == EventStatus::AwaitingApproval)
                    .rev()
                    .take(12)
                    .map(|e| json!({ "kind": e.kind, "status": e.status, "title": e.title, "at": e.updated_ts }))
                    .collect();
                let unresolved: Vec<Value> = ["lastBuild", "lastTest", "lastTypecheck", "lastInstall"]
                    .iter()
                    .filter_map(|k| p.status.get(*k).filter(|v| v.get("status").and_then(|s| s.as_str()) == Some("failed")).map(|v| json!({ "check": k, "result": v })))
                    .collect();
                Some(json!({
                    "name": p.name,
                    "path": p.path,
                    "lastBuild": p.status.get("lastBuild"),
                    "lastTest": p.status.get("lastTest"),
                    "activeTasks": tasks.iter().map(|t| json!({ "title": t.title, "kind": t.kind })).collect::<Vec<_>>(),
                    "recentOperations": recent,
                    "unresolvedFailures": unresolved,
                    "memories": self.db.list_memories(pid)?.len(),
                    "references": self.db.count_attachments(pid)?,
                }))
            }
            None => None,
        };

        Ok(CapabilityReport {
            generated_at: crate::db::now(),
            provider: json!({ "name": "NVIDIA (OpenAI-compatible API)", "baseUrl": s.base_url, "model": s.model, "maxTokens": s.max_tokens, "keyStored": key.stored }),
            capabilities: caps,
            agent_tools: crate::agent::tool_specs().iter().map(|t| json!({ "name": t.name, "description": t.description, "readOnly": t.read_only })).collect(),
            integrations,
            permissions: serde_json::to_value(&s.approvals)?,
            project,
            limitations: vec![
                "one project type is fully supported (Vite + React + TypeScript web apps)".into(),
                "model capabilities other than chat/streaming/image input are not assumed".into(),
                "the agent cannot run arbitrary commands; it can only use its listed tools".into(),
                if s.internet_access { "web access is read-only search and page fetching (no logins, forms or JavaScript execution)".into() } else { "internet access is turned off, so the agent cannot search or browse the web".into() },
                "live NVIDIA, GitHub and Supabase calls depend on your network and credentials".into(),
            ],
        })
    }
}
