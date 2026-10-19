//! One provider diagnostics workflow: staged, honest checks of the selected provider, each with
//! a plain explanation and a next step. Replaces scattered "connection test" / probe status
//! indicators with a single report the Settings screen shows.
//!
//! What it never does: show, log or return the API key (only the credential-store backend is
//! named); guess a provider's billing state (Genius cannot see it); claim a capability it did not
//! observe (image input is reported as "declared by model family", not as tested).

use crate::agent::Action;
use crate::app::Genius;
use crate::error::Result;
use crate::nvidia::{ChatMessage, ChatOptions, ProviderError};
use crate::provider::ProviderKind;
use crate::redact::redact;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Ok,
    Warn,
    Fail,
    /// Not run because an earlier step failed.
    Skipped,
    Info,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagStep {
    pub id: &'static str,
    pub title: &'static str,
    pub status: StepStatus,
    pub detail: String,
    /// What the user can do about it (set for warnings and failures).
    pub hint: Option<String>,
    pub ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagReport {
    pub provider: &'static str,
    pub label: &'static str,
    /// "live" or "contract-tests": how well Genius's integration with this provider has been verified.
    pub integration_verified: &'static str,
    pub host: String,
    pub model: Option<String>,
    /// True when no step failed.
    pub ok: bool,
    pub verdict: String,
    pub steps: Vec<DiagStep>,
    pub checked_at: String,
}

fn step(id: &'static str, title: &'static str, status: StepStatus, detail: impl Into<String>, hint: Option<String>, ms: Option<u64>) -> DiagStep {
    DiagStep { id, title, status, detail: redact(&detail.into()), hint: hint.map(|h| redact(&h)), ms }
}

/// Host part of a URL, for display and comparison. Never includes credentials or the path.
pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    authority.rsplit('@').next().unwrap_or("").to_string()
}

/// The next step for a provider failure, in plain words.
pub fn hint_for(e: &ProviderError, kind: ProviderKind) -> String {
    let label = kind.label();
    match e {
        ProviderError::MissingKey => format!("Add your {label} API key in Settings → AI provider. Each provider has its own key."),
        ProviderError::NoModel => "Choose a model in Settings → AI provider.".into(),
        ProviderError::Auth { .. } => format!("{label} did not accept this key. Check that it is a {label} key (not another provider's), that it is active, and that it was copied completely without spaces."),
        ProviderError::ModelUnavailable { model, .. } => format!("This key cannot use '{model}'. Pick another model from the list in Settings → AI provider."),
        ProviderError::RateLimited { retry_after_secs, .. } => format!("Too many requests. {}Genius retries automatically; lower the number of concurrent tasks in Settings → Limits if it keeps happening.", retry_after_secs.map(|s| format!("The provider asks you to wait about {s} s. ")).unwrap_or_default()),
        ProviderError::Quota { .. } => format!("The {label} account reports no remaining credit or quota. Genius cannot see your balance: check billing in the {label} dashboard."),
        ProviderError::BadRequest { .. } => "The provider refused the request. The selected model may not support what Genius sent (for example an image or a long input); try another model.".into(),
        ProviderError::Server { .. } => format!("{label} reports a server problem. This is usually temporary: try again shortly or check the provider's status page."),
        ProviderError::Network(_) => "Genius could not reach the server. Check your internet connection, proxy or firewall, and that the base URL is correct.".into(),
        ProviderError::Timeout(_) => "The server did not answer in time. Try again; if it persists, the model may be overloaded or the base URL wrong.".into(),
        ProviderError::Malformed(_) => format!("The answer was not in the format {label} uses. Check that the base URL belongs to the selected provider."),
        ProviderError::Cancelled => "The check was cancelled.".into(),
    }
}

fn fail(id: &'static str, title: &'static str, e: &ProviderError, kind: ProviderKind, started: Instant) -> DiagStep {
    step(id, title, StepStatus::Fail, e.to_string(), Some(hint_for(e, kind)), Some(started.elapsed().as_millis() as u64))
}

fn skipped(id: &'static str, title: &'static str, why: &str) -> DiagStep {
    step(id, title, StepStatus::Skipped, why, None, None)
}

/// Does the base URL look like the provider that is selected?
pub fn base_url_check(kind: ProviderKind, base_url: &str) -> DiagStep {
    let title = "Base URL";
    let host = host_of(base_url);
    if let Err(e) = crate::nvidia::normalize_base_url(base_url) {
        return step("base_url", title, StepStatus::Fail, e, Some(format!("Use the https address of the API, for example {}.", kind.default_base_url())), None);
    }
    match kind.expected_host() {
        Some(expected) if !host.ends_with(expected) && !host.starts_with("127.0.0.1") && !host.starts_with("localhost") => step(
            "base_url",
            title,
            StepStatus::Warn,
            format!("{host} does not look like {}.", kind.label()),
            Some(format!("If you did not mean to use a gateway or proxy, reset the base URL to {}.", kind.default_base_url())),
            None,
        ),
        _ => step("base_url", title, StepStatus::Ok, host, None, None),
    }
}

impl Genius {
    /// Run the provider diagnostics. `deep` adds one real model request that checks Genius's tool-call format.
    pub async fn diagnose_provider(&self, deep: bool) -> Result<DiagReport> {
        let (mut client, s) = self.client()?;
        client.max_retries = 0; // report problems promptly instead of waiting out retries
        client.idle_timeout = std::time::Duration::from_secs(45);
        let kind = client.kind();
        let mut steps = Vec::new();
        let caps = kind.capabilities();
        steps.push(step(
            "provider",
            "Provider",
            StepStatus::Info,
            format!("{} · {} wire · streaming and token usage supported", kind.label(), caps.wire),
            if caps.verification == "live" { None } else { Some("Genius's integration with this provider is covered by contract tests only; it has not been run against the live service yet. Report anything that behaves oddly.".into()) },
            None,
        ));

        // Credentials: stored or not. The value itself is never read into the report.
        let ks = self.key_status();
        if let Some(err) = &ks.error {
            steps.push(step("credentials", "API key", StepStatus::Fail, format!("The system credential store could not be read: {err}"), Some("Check that the operating system's credential store is available.".into()), None));
        } else if ks.stored {
            steps.push(step("credentials", "API key", StepStatus::Ok, format!("Saved in the {} (value not shown).", ks.backend), None, None));
        } else {
            steps.push(step("credentials", "API key", StepStatus::Fail, format!("No {} key is saved.", kind.label()), Some(hint_for(&ProviderError::MissingKey, kind)), None));
        }
        let have_key = ks.stored && ks.error.is_none();

        steps.push(base_url_check(kind, &s.base_url));
        let url_ok = steps.last().is_some_and(|x| x.status != StepStatus::Fail);

        // Connectivity + authentication + model access, in one request.
        let mut models = None;
        if have_key && url_ok {
            let t = Instant::now();
            match client.list_models().await {
                Ok(list) => {
                    steps.push(step("models", "Reach the provider and list models", StepStatus::Ok, format!("{} models available to this key.", list.len()), None, Some(t.elapsed().as_millis() as u64)));
                    models = Some(list);
                }
                Err(e) => steps.push(fail("models", "Reach the provider and list models", &e, kind, t)),
            }
        } else {
            steps.push(skipped("models", "Reach the provider and list models", if have_key { "Fix the base URL first." } else { "Needs an API key." }));
        }

        // The model Genius would use.
        let mut model: Option<String> = None;
        match &models {
            Some(list) => {
                let picked = if caps.auto_routing && s.is_auto() { self.resolve_models(crate::router::ModelRole::Agent, s.effort).await.ok().and_then(|c| c.into_iter().next()) } else { s.model.clone().filter(|m| !m.trim().is_empty()) };
                match picked {
                    None => steps.push(step("model", "Model", StepStatus::Fail, "No model is selected.", Some(hint_for(&ProviderError::NoModel, kind)), None)),
                    Some(m) if list.iter().any(|x| x.id == m) => {
                        steps.push(step("model", "Model", StepStatus::Ok, format!("{m} is available to this key{}.", if caps.auto_routing && s.is_auto() { " (chosen automatically)" } else { "" }), None, None));
                        model = Some(m);
                    }
                    Some(m) => {
                        let near: Vec<&str> = list.iter().map(|x| x.id.as_str()).filter(|id| id.contains(m.split(['/', ':']).next_back().unwrap_or(&m))).take(3).collect();
                        let hint = format!("Pick one of the models your key can use.{}", if near.is_empty() { String::new() } else { format!(" Similar: {}.", near.join(", ")) });
                        steps.push(step("model", "Model", StepStatus::Fail, format!("'{m}' is not in the list of models available to this key."), Some(hint), None));
                    }
                }
            }
            None => steps.push(skipped("model", "Model", "Needs the model list.")),
        }

        // A real, tiny streamed request: the only proof that the key authorises inference.
        let mut chat_ok = false;
        if let Some(m) = &model {
            let t = Instant::now();
            match client.probe_model(m, &CancellationToken::new()).await {
                Ok(p) => {
                    chat_ok = true;
                    let detail = format!("{m} answered in {} ms; streaming {}.", p.latency_ms, if p.streaming { "works" } else { "is not supported (Genius falls back to waiting for the full reply)" });
                    self.record_probe(&p);
                    let _ = self.store_probe(p);
                    steps.push(step("chat", "Send a test message", StepStatus::Ok, detail, None, Some(t.elapsed().as_millis() as u64)));
                }
                Err(e) => steps.push(fail("chat", "Send a test message", &e, kind, t)),
            }
            let vision = self.is_vision_capable(m);
            steps.push(step(
                "vision",
                "Image input",
                StepStatus::Info,
                if vision { format!("{m} is expected to accept images (declared by model family; not tested with a real image).") } else { format!("{m} is not known to accept images. Genius hands images to a vision model when one is available, otherwise it says it cannot see them.") },
                None,
                None,
            ));
        } else {
            steps.push(skipped("chat", "Send a test message", "Needs a working model."));
        }

        // Deep: can this model follow Genius's tool-call format?
        if deep {
            match (&model, chat_ok) {
                (Some(m), true) => {
                    let t = Instant::now();
                    let prompt = vec![ChatMessage::user("Reply with exactly this and nothing else:\n<tool>{\"name\": \"runtime_info\", \"args\": {}}</tool>")];
                    let opts = ChatOptions { temperature: 0.0, top_p: 1.0, max_tokens: 256, stream: true, thinking: Some(false), reasoning_effort: None };
                    match client.chat(m, &prompt, &opts, &CancellationToken::new(), |_| {}).await {
                        Ok(r) => {
                            let calls = crate::agent::parse_turn(&r.content).calls;
                            let ok = calls.iter().any(|c| matches!(c, Action::Tool { name, .. } if name == "runtime_info"));
                            let (status, hint) = if ok { (StepStatus::Ok, None) } else { (StepStatus::Warn, Some("This model did not produce a valid tool call. Chat will work, but tasks that need tools may be unreliable: try a larger or instruction-tuned model.".to_string())) };
                            steps.push(step("tools", "Tool-call format", status, if ok { "The model produced a valid Genius tool call.".to_string() } else { format!("Expected a runtime_info call, got: {}", r.content.chars().take(120).collect::<String>()) }, hint, Some(t.elapsed().as_millis() as u64)));
                        }
                        Err(e) => steps.push(fail("tools", "Tool-call format", &e, kind, t)),
                    }
                }
                _ => steps.push(skipped("tools", "Tool-call format", "Needs a working model.")),
            }
        }

        let failed: Vec<&DiagStep> = steps.iter().filter(|x| x.status == StepStatus::Fail).collect();
        let warned = steps.iter().filter(|x| x.status == StepStatus::Warn).count();
        let verdict = match (failed.first(), warned) {
            (Some(f), _) => format!("Not ready: {} — {}", f.title, f.hint.clone().unwrap_or_else(|| f.detail.clone())),
            (None, 0) => format!("{} is ready.", kind.label()),
            (None, n) => format!("{} works, with {n} warning(s) to look at.", kind.label()),
        };
        let report = DiagReport { provider: kind.id(), label: kind.label(), integration_verified: caps.verification, host: host_of(client.base_url()), model, ok: failed.is_empty(), verdict, steps, checked_at: chrono::Utc::now().to_rfc3339() };
        self.bus.record(None, None, crate::events::EventKind::Ai, if report.ok { crate::events::EventStatus::Completed } else { crate::events::EventStatus::Failed }, format!("Provider diagnostics: {}", report.verdict), json!({ "provider": report.provider, "deep": deep, "steps": report.steps.iter().map(|x| json!({ "id": x.id, "status": x.status })).collect::<Vec<Value>>() }));
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_extracted_without_credentials_or_paths() {
        assert_eq!(host_of("https://api.anthropic.com/v1"), "api.anthropic.com");
        assert_eq!(host_of("https://user:pass@gateway.example.com:8443/x?y=1"), "gateway.example.com:8443");
        assert_eq!(host_of("http://127.0.0.1:9/v1"), "127.0.0.1:9");
    }

    #[test]
    fn base_url_warns_when_it_does_not_match_the_provider() {
        assert_eq!(base_url_check(ProviderKind::Anthropic, "https://api.anthropic.com/v1").status, StepStatus::Ok);
        let w = base_url_check(ProviderKind::Anthropic, "https://integrate.api.nvidia.com/v1");
        assert_eq!(w.status, StepStatus::Warn);
        assert!(w.hint.unwrap().contains("https://api.anthropic.com/v1"));
        assert_eq!(base_url_check(ProviderKind::Custom, "https://my-gateway.example.com/v1").status, StepStatus::Ok, "custom endpoints may be anywhere");
        assert_eq!(base_url_check(ProviderKind::OpenAi, "http://api.openai.com/v1").status, StepStatus::Fail, "plain http to a remote host is refused");
    }

    #[test]
    fn every_error_kind_has_a_next_step() {
        let all = [
            ProviderError::MissingKey,
            ProviderError::NoModel,
            ProviderError::Auth { status: 401, message: String::new() },
            ProviderError::ModelUnavailable { model: "m".into(), status: 404, message: String::new() },
            ProviderError::RateLimited { retry_after_secs: Some(7), message: String::new() },
            ProviderError::Quota { status: 402, message: String::new() },
            ProviderError::BadRequest { status: 400, message: String::new() },
            ProviderError::Server { status: 500, message: String::new() },
            ProviderError::Network("x".into()),
            ProviderError::Timeout("x".into()),
            ProviderError::Malformed("x".into()),
            ProviderError::Cancelled,
        ];
        for e in &all {
            assert!(hint_for(e, ProviderKind::Gemini).len() > 20, "{e:?}");
        }
        assert!(hint_for(&all[4], ProviderKind::Gemini).contains("7 s"));
        assert!(hint_for(&all[5], ProviderKind::Gemini).contains("cannot see your balance"), "Genius never pretends to know the user's billing state");
    }
}
