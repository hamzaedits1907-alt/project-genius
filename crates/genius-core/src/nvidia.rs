//! NVIDIA API provider (hosted `integrate.api.nvidia.com` or a self-hosted NIM).
//!
//! NVIDIA's hosted LLM APIs are OpenAI-compatible:
//! * `POST {base}/chat/completions` — chat completions, SSE streaming with `stream: true`
//! * `GET  {base}/models`           — model listing (`{ "object": "list", "data": [{ "id", "owned_by", ... }] }`)
//!
//! Default base: `https://integrate.api.nvidia.com/v1`. The listing does not describe
//! per-model capabilities, so Genius never assumes them: any category shown in the UI
//! is labelled as inferred from the model id, and chat/streaming support is only marked
//! as *verified* after a real request succeeds (see [`NvidiaClient::probe_model`]).

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::provider::ProviderKind;
use crate::redact::redact;

pub const DEFAULT_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderError {
    #[error("No API key is saved for the selected provider. Add one in Settings → AI Provider.")]
    MissingKey,
    #[error("No model is selected. Choose a model in Settings → AI Provider.")]
    NoModel,
    #[error("The provider rejected the API key (HTTP {status}). Check that the key is correct, active and belongs to the selected provider. {message}")]
    Auth { status: u16, message: String },
    #[error("The model '{model}' is not available to this key (HTTP {status}). {message}")]
    ModelUnavailable { model: String, status: u16, message: String },
    #[error("Rate limited by the provider (HTTP 429). {message}")]
    RateLimited { retry_after_secs: Option<u64>, message: String },
    #[error("The provider reports the account has no remaining credits or quota (HTTP {status}). {message}")]
    Quota { status: u16, message: String },
    #[error("The request was rejected (HTTP {status}); the selected model may not support a parameter Genius sent. {message}")]
    BadRequest { status: u16, message: String },
    #[error("Provider server error (HTTP {status}). Try again shortly. {message}")]
    Server { status: u16, message: String },
    #[error("Network error contacting the provider: {0}")]
    Network(String),
    #[error("The request to the provider timed out: {0}")]
    Timeout(String),
    #[error("Unexpected response from the provider: {0}")]
    Malformed(String),
    #[error("Request cancelled.")]
    Cancelled,
}

impl ProviderError {
    pub fn kind(&self) -> &'static str {
        match self {
            ProviderError::MissingKey => "provider_missing_key",
            ProviderError::NoModel => "provider_no_model",
            ProviderError::Auth { .. } => "provider_auth",
            ProviderError::ModelUnavailable { .. } => "provider_model_unavailable",
            ProviderError::RateLimited { .. } => "provider_rate_limited",
            ProviderError::Quota { .. } => "provider_quota",
            ProviderError::BadRequest { .. } => "provider_bad_request",
            ProviderError::Server { .. } => "provider_server",
            ProviderError::Network(_) => "provider_network",
            ProviderError::Timeout(_) => "provider_timeout",
            ProviderError::Malformed(_) => "provider_malformed",
            ProviderError::Cancelled => "cancelled",
        }
    }

    fn retryable(&self) -> bool {
        matches!(
            self,
            ProviderError::RateLimited { .. } | ProviderError::Server { status: 502..=504 | 529, .. } | ProviderError::Network(_)
        )
    }
}

pub type PResult<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub owner: Option<String>,
    pub created: Option<i64>,
    /// Category inferred from the model id. Never treated as authoritative.
    pub inferred_category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    /// Image data URLs (`data:image/png;base64,...`). Only sent to models whose image input
    /// was verified; serialized as OpenAI-compatible content parts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

impl ChatMessage {
    pub fn system(c: impl Into<String>) -> Self {
        Self { role: "system".into(), content: c.into(), images: vec![] }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self { role: "user".into(), content: c.into(), images: vec![] }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: c.into(), images: vec![] }
    }

    /// Wire format: plain string content, or content parts when images are attached.
    pub fn to_wire(&self) -> Value {
        if self.images.is_empty() {
            return json!({ "role": self.role, "content": self.content });
        }
        let mut parts = vec![json!({ "type": "text", "text": self.content })];
        parts.extend(self.images.iter().map(|u| json!({ "type": "image_url", "image_url": { "url": u } })));
        json!({ "role": self.role, "content": parts })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOptions {
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub stream: bool,
    /// Reasoning on/off for model families with a known switch (see [`thinking_style`]);
    /// `None` leaves the model's default.
    #[serde(default)]
    pub thinking: Option<bool>,
    /// "low" | "medium" | "high" for models that take `reasoning_effort` (gpt-oss).
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self { temperature: 0.2, top_p: 0.95, max_tokens: 8192, stream: true, thinking: None, reasoning_effort: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    pub content: String,
    pub finish_reason: Option<String>,
    pub streamed: bool,
    pub usage: Option<Value>,
    /// True when the model emitted reasoning (separate `reasoning_content` or inline `<think>`).
    pub had_reasoning: bool,
    /// The model id Genius **requested** for this reply (what was sent in the request body).
    pub model: String,
    /// The model id the provider **reported** in its response, when it reported one. It can differ
    /// from [`model`](Self::model) (aliases, routing, dated snapshots); neither is ever invented.
    #[serde(default)]
    pub reported_model: Option<String>,
    /// Milliseconds from sending the request to the first content or reasoning token.
    #[serde(default)]
    pub first_token_ms: Option<u64>,
    /// Milliseconds the whole call took, including retries.
    #[serde(default)]
    pub total_ms: u64,
    /// Reasoning text (never part of `content`), clipped to [`MAX_REASONING_CHARS`].
    pub reasoning: String,
}

pub const MAX_REASONING_CHARS: usize = 20_000;

/// Optional request parameters a model rejected, remembered per model so they are not sent
/// again. Shared between clients (a new client is created per call).
pub type ParamMemory = Arc<Mutex<HashMap<String, HashSet<String>>>>;

/// Optional body parameters that are dropped (and remembered) when a server rejects them.
const OPTIONAL_PARAMS: [&str; 5] = ["stream_options", "chat_template_kwargs", "reasoning_effort", "temperature", "top_p"];

#[derive(Clone)]
pub struct NvidiaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    pub idle_timeout: Duration,
    /// How long the provider may take to START answering (response headers, then the first data): a request that is
    /// accepted and then never answered used to hang for minutes (or forever before the first byte).
    pub start_timeout: Duration,
    pub max_retries: u32,
    params: ParamMemory,
    kind: ProviderKind,
}

impl NvidiaClient {
    pub fn new(base_url: &str, api_key: Option<String>) -> PResult<Self> {
        let base = normalize_base_url(base_url).map_err(ProviderError::Malformed)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("ProjectGenius/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok(Self { http, base_url: base, api_key, idle_timeout: Duration::from_secs(120), start_timeout: Duration::from_secs(90), max_retries: 3, params: ParamMemory::default(), kind: ProviderKind::Nvidia })
    }

    /// Talk to another provider (default: NVIDIA / OpenAI chat-completions).
    pub fn with_kind(mut self, kind: ProviderKind) -> Self {
        self.base_url = crate::provider::normalize_root(kind, &self.base_url);
        self.kind = kind;
        self
    }

    pub fn kind(&self) -> ProviderKind {
        self.kind
    }

    /// Share the memory of rejected optional parameters with other clients.
    pub fn with_param_memory(mut self, params: ParamMemory) -> Self {
        self.params = params;
        self
    }

    /// Optional parameters this model is known to reject.
    pub fn rejected_params(&self, model: &str) -> HashSet<String> {
        self.params.lock().unwrap().get(model).cloned().unwrap_or_default()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn key(&self) -> PResult<&str> {
        self.api_key.as_deref().ok_or(ProviderError::MissingKey)
    }

    /// GET /models. Authenticated when a key is available.
    pub async fn list_models(&self) -> PResult<Vec<ModelInfo>> {
        let mut req = self.http.get(crate::provider::models_endpoint(self.kind, &self.base_url)).timeout(Duration::from_secs(30));
        if let Some(k) = &self.api_key {
            req = apply_auth(req, self.kind, k);
        }
        let resp = req.send().await.map_err(net_err)?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(net_err)?;
        if !(200..300).contains(&status) {
            return Err(crate::provider::classify_error(self.kind, status, &body, None, None));
        }
        crate::provider::parse_models(self.kind, &body)
    }

    /// Verify that the key is accepted. Uses the model listing when the key is honoured by it;
    /// if a model is given, also performs a 1-token chat completion, which is the only way to
    /// prove the key authorizes inference.
    pub async fn test_connection(&self, model: Option<&str>) -> PResult<Value> {
        self.key()?;
        let models = self.list_models().await?;
        let mut out = json!({ "modelsListed": models.len() });
        if let Some(m) = model {
            let probe = self.probe_model(m, &CancellationToken::new()).await?;
            out["probe"] = serde_json::to_value(probe).unwrap_or(Value::Null);
        }
        Ok(out)
    }

    /// Send a tiny streaming request to verify the model accepts chat completions and streaming.
    pub async fn probe_model(&self, model: &str, cancel: &CancellationToken) -> PResult<ProbeResult> {
        let started = std::time::Instant::now();
        let msgs = vec![ChatMessage::user("Reply with the single word: ready")];
        // Moderate sampling: some hosted models reject temperature 0. Thinking off so a reasoning
        // model does not spend the tiny budget on reasoning.
        let opts = ChatOptions { temperature: 0.2, top_p: 0.7, max_tokens: 16, stream: true, thinking: Some(false), reasoning_effort: None };
        let res = self.chat(model, &msgs, &opts, cancel, |_| {}).await?;
        Ok(ProbeResult {
            model: model.into(),
            chat: true,
            streaming: res.streamed,
            latency_ms: started.elapsed().as_millis() as u64,
            sample: res.content.chars().take(40).collect(),
        })
    }

    /// Chat completion with streaming (falls back to non-streaming when the model rejects it).
    /// `on_delta` receives content text as it arrives.
    pub async fn chat(&self, model: &str, messages: &[ChatMessage], opts: &ChatOptions, cancel: &CancellationToken, on_delta: impl FnMut(&str)) -> PResult<ChatResult> {
        self.chat_with_reasoning(model, messages, opts, cancel, on_delta, |_| {}).await
    }

    /// [`chat`](Self::chat) that also streams reasoning (separate `reasoning_content` deltas and
    /// inline `<think>` sections, which never reach `on_delta`) to `on_reasoning`.
    pub async fn chat_with_reasoning(
        &self,
        model: &str,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        cancel: &CancellationToken,
        mut on_delta: impl FnMut(&str),
        mut on_reasoning: impl FnMut(&str),
    ) -> PResult<ChatResult> {
        if model.trim().is_empty() {
            return Err(ProviderError::NoModel);
        }
        let mut attempt = 0;
        let mut stream = opts.stream;
        let mut opts = opts.clone();
        let started = std::time::Instant::now();
        loop {
            attempt += 1;
            let body = crate::provider::build_body(self.kind, model, messages, &opts, stream, &self.rejected_params(model));
            // 0: no token yet (the clock is shared by both callbacks; atomics keep the future `Send`)
            let first_token = std::sync::atomic::AtomicU64::new(0);
            let mark = || {
                let _ = first_token.compare_exchange(0, (started.elapsed().as_millis() as u64).max(1), std::sync::atomic::Ordering::Relaxed, std::sync::atomic::Ordering::Relaxed);
            };
            let res = {
                let mut on_delta = |d: &str| {
                    mark();
                    on_delta(d)
                };
                let mut on_reasoning = |d: &str| {
                    mark();
                    on_reasoning(d)
                };
                self.chat_once(model, &body, stream, cancel, &mut on_delta, &mut on_reasoning).await
            };
            match res {
                Ok(mut r) => {
                    r.model = model.to_string();
                    r.first_token_ms = Some(first_token.load(std::sync::atomic::Ordering::Relaxed)).filter(|t| *t > 0);
                    r.total_ms = started.elapsed().as_millis() as u64;
                    if r.reasoning.len() > MAX_REASONING_CHARS {
                        r.reasoning = r.reasoning.chars().take(MAX_REASONING_CHARS).collect();
                    }
                    return Ok(r);
                }
                Err(ProviderError::BadRequest { ref message, .. }) if rejected_param(message, &body).is_some() => {
                    // An optional parameter is not supported by this model: drop it for good.
                    let p = rejected_param(message, &body).unwrap_or_default();
                    log::info!("{model} rejected optional parameter {p}; retrying without it");
                    self.params.lock().unwrap().entry(model.to_string()).or_default().insert(p.to_string());
                }
                Err(ProviderError::BadRequest { ref message, .. })
                    if opts.max_tokens > 1024 && is_token_limit_error(message) =>
                {
                    // The model has a smaller output limit than requested: retry with less.
                    opts.max_tokens = (opts.max_tokens / 2).max(1024);
                }
                Err(ProviderError::BadRequest { ref message, .. })
                    if stream && message.to_lowercase().contains("stream") =>
                {
                    // Model does not support streaming: retry once without it.
                    stream = false;
                }
                Err(e) if e.retryable() && attempt <= self.max_retries => {
                    let wait = match &e {
                        ProviderError::RateLimited { retry_after_secs: Some(s), .. } => (*s).min(60),
                        _ => 2u64.pow(attempt),
                    };
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                        _ = tokio::time::sleep(Duration::from_secs(wait)) => {}
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn chat_once(
        &self,
        model: &str,
        body: &Value,
        stream: bool,
        cancel: &CancellationToken,
        on_delta: &mut impl FnMut(&str),
        on_reasoning: &mut impl FnMut(&str),
    ) -> PResult<ChatResult> {
        let key = self.key()?;
        let mut req = apply_auth(self.http.post(crate::provider::chat_endpoint(self.kind, &self.base_url, model, stream)), self.kind, key)
            .header("Accept", if stream { "text/event-stream" } else { "application/json" })
            .json(body);
        if !stream {
            req = req.timeout(Duration::from_secs(600));
        }
        let send = req.send();
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
            _ = tokio::time::sleep(self.start_timeout) => {
                return Err(ProviderError::Timeout(format!("the provider did not start answering within {}s", self.start_timeout.as_secs())))
            }
            r = send => r.map_err(net_err)?,
        };
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::provider::classify_error(self.kind, status, &text, Some(model), retry_after));
        }
        let is_sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream"))
            .unwrap_or(false);

        if self.kind.wire() != crate::provider::Wire::OpenAiChat {
            return self.read_native(resp, is_sse, cancel, on_delta, on_reasoning).await;
        }
        if !is_sse {
            let text = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                t = resp.text() => t.map_err(net_err)?,
            };
            if stream && text.trim_start().starts_with("data:") {
                // Server sent SSE with a wrong content-type: parse it as a stream anyway.
                return parse_sse_body(&text, on_delta, on_reasoning);
            }
            let mut r = parse_completion(&text)?;
            let mut filter = ThinkFilter::default();
            r.content = filter.push(&r.content) + &filter.finish();
            r.reasoning.push_str(&filter.take_thought());
            r.had_reasoning |= filter.saw_think;
            if !r.reasoning.is_empty() {
                on_reasoning(&r.reasoning);
            }
            if !r.content.is_empty() {
                on_delta(&r.content);
            }
            return Ok(r);
        }

        let mut result = ChatResult { streamed: true, ..Default::default() };
        let mut sse = SseParser::default();
        let mut filter = ThinkFilter::default();
        let mut bytes = resp.bytes_stream();
        let mut got_data = false;
        loop {
            let wait = if got_data { self.idle_timeout } else { self.start_timeout };
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                _ = tokio::time::sleep(wait) => {
                    return Err(ProviderError::Timeout(format!("no data received for {}s while streaming", wait.as_secs())))
                }
                n = bytes.next() => n,
            };
            got_data = true;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(net_err)?;
            for data in sse.push(&String::from_utf8_lossy(&chunk)) {
                if data.trim() == "[DONE]" {
                    let tail = filter.finish();
                    let thought = filter.take_thought();
                    if !thought.is_empty() {
                        on_reasoning(&thought);
                        result.reasoning.push_str(&thought);
                    }
                    if !tail.is_empty() {
                        on_delta(&tail);
                        result.content.push_str(&tail);
                    }
                    return Ok(result);
                }
                let v: Value = serde_json::from_str(&data)
                    .map_err(|e| ProviderError::Malformed(format!("invalid stream chunk: {e}")))?;
                if let Some(err) = v.get("error") {
                    return Err(ProviderError::Server { status: 200, message: redact(&err.to_string()) });
                }
                if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
                    result.usage = Some(u.clone());
                }
                if result.reported_model.is_none() {
                    result.reported_model = reported_model(&v);
                }
                let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else { continue };
                let delta = &choice["delta"];
                if let Some(r) = reasoning_of(delta) {
                    result.had_reasoning = true;
                    on_reasoning(r);
                    result.reasoning.push_str(r);
                }
                if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                    let visible = filter.push(text);
                    if filter.saw_think {
                        result.had_reasoning = true;
                    }
                    let thought = filter.take_thought();
                    if !thought.is_empty() {
                        on_reasoning(&thought);
                        result.reasoning.push_str(&thought);
                    }
                    if !visible.is_empty() {
                        on_delta(&visible);
                        result.content.push_str(&visible);
                    }
                }
                if let Some(fr) = choice.get("finish_reason").and_then(|f| f.as_str()) {
                    result.finish_reason = Some(fr.to_string());
                }
            }
        }
        // Stream ended without [DONE]: accept if we got a finish_reason, otherwise report truncation.
        let tail = filter.finish();
        let thought = filter.take_thought();
        if !thought.is_empty() {
            on_reasoning(&thought);
            result.reasoning.push_str(&thought);
        }
        if !tail.is_empty() {
            on_delta(&tail);
            result.content.push_str(&tail);
        }
        if result.finish_reason.is_none() && result.content.is_empty() && result.reasoning.is_empty() {
            return Err(ProviderError::Malformed("stream ended without any content".into()));
        }
        Ok(result)
    }
}

impl NvidiaClient {
    /// Read the reply of a native-wire provider (Anthropic Messages, Gemini): streamed events or one JSON body.
    async fn read_native(&self, resp: reqwest::Response, is_sse: bool, cancel: &CancellationToken, on_delta: &mut impl FnMut(&str), on_reasoning: &mut impl FnMut(&str)) -> PResult<ChatResult> {
        let mut state = crate::provider::StreamState::new(self.kind);
        let mut sse = SseParser::default();
        if !is_sse {
            let text = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                t = resp.text() => t.map_err(net_err)?,
            };
            if !text.trim_start().starts_with("data:") {
                let r = crate::provider::parse_completion(self.kind, &text)?;
                if !r.reasoning.is_empty() {
                    on_reasoning(&r.reasoning);
                }
                if !r.content.is_empty() {
                    on_delta(&r.content);
                }
                return Ok(r);
            }
            // SSE sent with a wrong content type: parse the buffered body as a stream.
            let mut events = sse.push(&text);
            events.extend(sse.push("\n\n"));
            for data in events {
                state.feed(&data, &mut *on_delta, &mut *on_reasoning)?;
            }
            return state.finish();
        }
        let mut bytes = resp.bytes_stream();
        let mut got_data = false;
        loop {
            let wait = if got_data { self.idle_timeout } else { self.start_timeout };
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                _ = tokio::time::sleep(wait) => {
                    return Err(ProviderError::Timeout(format!("no data received for {}s while streaming", wait.as_secs())))
                }
                n = bytes.next() => n,
            };
            got_data = true;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(net_err)?;
            for data in sse.push(&String::from_utf8_lossy(&chunk)) {
                state.feed(&data, &mut *on_delta, &mut *on_reasoning)?;
                if state.done {
                    return state.finish();
                }
            }
        }
        state.finish()
    }
}

/// Add the provider's authentication header(s); the values are marked sensitive so they stay out of debug output.
fn apply_auth(mut req: reqwest::RequestBuilder, kind: ProviderKind, key: &str) -> reqwest::RequestBuilder {
    for (name, value) in crate::provider::auth_headers(kind, key) {
        match reqwest::header::HeaderValue::from_str(&value) {
            Ok(mut v) => {
                v.set_sensitive(true);
                req = req.header(name, v);
            }
            Err(_) => req = req.header(name, "invalid-key-characters"), // a key with control characters is refused by the provider as unauthorised
        }
    }
    req
}

/// Reasoning text of a stream delta or message (`reasoning_content`, or `reasoning` on some servers).
fn reasoning_of(v: &Value) -> Option<&str> {
    v.get("reasoning_content").or_else(|| v.get("reasoning")).and_then(|r| r.as_str()).filter(|r| !r.is_empty())
}

/// Parse a fully-buffered SSE body (used when a server mislabels its content type).
fn parse_sse_body(text: &str, on_delta: &mut impl FnMut(&str), on_reasoning: &mut impl FnMut(&str)) -> PResult<ChatResult> {
    let mut result = ChatResult { streamed: true, ..Default::default() };
    let mut sse = SseParser::default();
    let mut filter = ThinkFilter::default();
    let mut events = sse.push(text);
    events.extend(sse.push("\n\n"));
    for data in events {
        if data.trim() == "[DONE]" {
            break;
        }
        let v: Value = serde_json::from_str(&data).map_err(|e| ProviderError::Malformed(format!("invalid stream chunk: {e}")))?;
        if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
            result.usage = Some(u.clone());
        }
        if result.reported_model.is_none() {
            result.reported_model = reported_model(&v);
        }
        if let Some(r) = v.pointer("/choices/0/delta").and_then(reasoning_of) {
            on_reasoning(r);
            result.reasoning.push_str(r);
            result.had_reasoning = true;
        }
        if let Some(c) = v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()) {
            let visible = filter.push(c);
            if !visible.is_empty() {
                on_delta(&visible);
                result.content.push_str(&visible);
            }
        }
        if let Some(fr) = v.pointer("/choices/0/finish_reason").and_then(|f| f.as_str()) {
            result.finish_reason = Some(fr.into());
        }
    }
    let tail = filter.finish();
    on_delta(&tail);
    result.content.push_str(&tail);
    let thought = filter.take_thought();
    if !thought.is_empty() {
        on_reasoning(&thought);
        result.reasoning.push_str(&thought);
    }
    result.had_reasoning |= filter.saw_think;
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub model: String,
    pub chat: bool,
    pub streaming: bool,
    pub latency_ms: u64,
    pub sample: String,
}

fn is_token_limit_error(message: &str) -> bool {
    let m = message.to_lowercase();
    (m.contains("max_tokens") || m.contains("max tokens") || m.contains("maximum context") || m.contains("context length"))
        && !m.contains("stream")
}

/// The optional parameter a 400 response complains about, if it is one Genius sent.
fn rejected_param(message: &str, body: &Value) -> Option<&'static str> {
    let m = message.to_lowercase();
    let sent: Vec<&'static str> = OPTIONAL_PARAMS.iter().copied().filter(|p| body.get(*p).is_some()).collect();
    if let Some(p) = sent.iter().find(|p| m.contains(**p)) {
        return Some(p);
    }
    if m.contains("enable_thinking") || m.contains("thinking") || m.contains("template") {
        if let Some(p) = sent.iter().find(|p| **p == "chat_template_kwargs") {
            return Some(p);
        }
    }
    if m.contains("include_usage") {
        return sent.iter().find(|p| **p == "stream_options").copied();
    }
    // Generic "unknown field" errors that do not name it: drop the least important first.
    let generic = ["extra inputs are not permitted", "extra_forbidden", "unrecognized", "unknown parameter", "unsupported parameter", "unexpected keyword", "unknown field", "additional properties"];
    if generic.iter().any(|g| m.contains(g)) {
        return sent.first().copied();
    }
    None
}

/// How a model family's reasoning is switched on or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingStyle {
    /// No known switch: nothing is sent.
    None,
    /// `chat_template_kwargs: {"enable_thinking": bool}` (Nemotron 3, Qwen3, GLM 4.5+).
    EnableThinking,
    /// `chat_template_kwargs: {"thinking": bool}` (DeepSeek V3.1+).
    DeepSeekThinking,
    /// System prompt "detailed thinking on|off" (Llama-Nemotron Super/Ultra v1).
    SystemDetailed,
    /// System prompt "/think" | "/no_think" (Llama-Nemotron v1.5, Nemotron Nano v2).
    SystemSlash,
    /// `reasoning_effort: "low"|"medium"|"high"` (gpt-oss).
    ReasoningEffort,
}

/// Version number right after `prefix` in `id` (e.g. "deepseek-v" in "deepseek-v3.1" → 3.1).
pub fn version_after(id: &str, prefix: &str) -> Option<f32> {
    let rest = &id[id.find(prefix)? + prefix.len()..];
    let rest = rest.trim_start_matches(['-', '_']);
    let v: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    v.trim_end_matches('.').parse().ok()
}

pub fn thinking_style(model: &str) -> ThinkingStyle {
    let l = model.to_lowercase();
    let vision = l.contains("-vl") || l.contains("vision");
    if l.contains("gpt-oss") {
        ThinkingStyle::ReasoningEffort
    } else if l.contains("llama") && l.contains("nemotron") {
        if l.contains("v1.5") || l.contains("v1_5") { ThinkingStyle::SystemSlash } else { ThinkingStyle::SystemDetailed }
    } else if l.contains("nemotron-nano") && l.contains("-v2") && !vision {
        ThinkingStyle::SystemSlash
    } else if version_after(&l, "nemotron-").is_some_and(|v| v >= 3.0) && !vision && !l.contains("340b") {
        ThinkingStyle::EnableThinking
    } else if version_after(&l, "deepseek-v").is_some_and(|v| v >= 3.1) {
        ThinkingStyle::DeepSeekThinking
    } else if l.contains("qwen3") && !l.contains("coder") && !vision {
        ThinkingStyle::EnableThinking
    } else if version_after(&l, "glm").is_some_and(|v| v >= 4.5) {
        ThinkingStyle::EnableThinking
    } else {
        ThinkingStyle::None
    }
}

pub fn build_chat_body(model: &str, messages: &[ChatMessage], opts: &ChatOptions, stream: bool) -> Value {
    build_chat_body_with(model, messages, opts, stream, &HashSet::new())
}

/// Request body; optional parameters listed in `skip` (rejected by this model before) are left out.
pub fn build_chat_body_with(model: &str, messages: &[ChatMessage], opts: &ChatOptions, stream: bool, skip: &HashSet<String>) -> Value {
    let mut wire: Vec<Value> = messages.iter().map(ChatMessage::to_wire).collect();
    let style = thinking_style(model);
    let flag = match (style, opts.thinking) {
        (ThinkingStyle::SystemDetailed, Some(on)) => Some(if on { "detailed thinking on" } else { "detailed thinking off" }),
        (ThinkingStyle::SystemSlash, Some(on)) => Some(if on { "/think" } else { "/no_think" }),
        _ => None,
    };
    if let Some(flag) = flag {
        match wire.first_mut() {
            Some(first) if first["role"] == "system" && first["content"].is_string() => {
                first["content"] = json!(format!("{flag}\n\n{}", first["content"].as_str().unwrap_or("")));
            }
            _ => wire.insert(0, json!({ "role": "system", "content": flag })),
        }
    }
    let mut body = json!({
        "model": model,
        "messages": wire,
        "max_tokens": opts.max_tokens,
        "stream": stream,
    });
    let allowed = |p: &str| !skip.contains(p);
    // Sampling parameters become optional once a model has rejected them (some hosted models refuse a custom temperature).
    if allowed("temperature") {
        body["temperature"] = json!(opts.temperature);
    }
    if allowed("top_p") {
        body["top_p"] = json!(opts.top_p);
    }
    if stream && allowed("stream_options") {
        body["stream_options"] = json!({ "include_usage": true });
    }
    match (style, opts.thinking) {
        (ThinkingStyle::EnableThinking, Some(on)) if allowed("chat_template_kwargs") => body["chat_template_kwargs"] = json!({ "enable_thinking": on }),
        (ThinkingStyle::DeepSeekThinking, Some(on)) if allowed("chat_template_kwargs") => body["chat_template_kwargs"] = json!({ "thinking": on }),
        (ThinkingStyle::ReasoningEffort, t) if allowed("reasoning_effort") => {
            let level = opts.reasoning_effort.clone().or_else(|| t.map(|on| if on { "high" } else { "low" }.to_string()));
            if let Some(level) = level {
                body["reasoning_effort"] = json!(level);
            }
        }
        _ => {}
    }
    body
}

pub fn normalize_base_url(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix("/chat/completions").unwrap_or(u);
    if !(u.starts_with("https://") || u.starts_with("http://127.0.0.1") || u.starts_with("http://localhost")) {
        return Err("Base URL must use https:// (plain http is only allowed for localhost NIM deployments).".into());
    }
    Ok(u.to_string())
}

fn net_err(e: reqwest::Error) -> ProviderError {
    if e.is_timeout() {
        ProviderError::Timeout(redact(&e.to_string()))
    } else {
        ProviderError::Network(redact(&e.to_string()))
    }
}

fn extract_message(body: &str) -> String {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("detail"))
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
                .or_else(|| v.get("title"))
                .map(|m| m.as_str().map(String::from).unwrap_or_else(|| m.to_string()))
        })
        .unwrap_or_else(|| body.chars().take(300).collect());
    redact(msg.trim())
}

pub fn classify(status: u16, body: &str, model: Option<&str>, retry_after: Option<u64>) -> ProviderError {
    let message = extract_message(body);
    let lower = message.to_lowercase();
    match status {
        401 => ProviderError::Auth { status, message },
        402 => ProviderError::Quota { status, message },
        403 if lower.contains("credit") || lower.contains("quota") => ProviderError::Quota { status, message },
        403 => ProviderError::Auth { status, message },
        404 | 410 => ProviderError::ModelUnavailable { model: model.unwrap_or("?").into(), status, message },
        429 => ProviderError::RateLimited { retry_after_secs: retry_after, message },
        400 | 422 if model.is_some() && (lower.contains("model") && (lower.contains("not found") || lower.contains("does not exist") || lower.contains("unknown"))) => {
            ProviderError::ModelUnavailable { model: model.unwrap_or("?").into(), status, message }
        }
        400..=499 => ProviderError::BadRequest { status, message },
        _ => ProviderError::Server { status, message },
    }
}

pub fn parse_models(body: &str) -> PResult<Vec<ModelInfo>> {
    let v: Value = serde_json::from_str(body).map_err(|e| ProviderError::Malformed(format!("model list is not JSON: {e}")))?;
    let arr = v
        .get("data")
        .and_then(|d| d.as_array())
        .or_else(|| v.as_array())
        .ok_or_else(|| ProviderError::Malformed("model list has no 'data' array".into()))?;
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?.trim().to_string();
            if id.is_empty() {
                return None;
            }
            Some(ModelInfo {
                inferred_category: infer_category(&id).into(),
                owner: m.get("owned_by").and_then(|o| o.as_str()).map(String::from)
                    .or_else(|| id.split_once('/').map(|(o, _)| o.to_string())),
                created: m.get("created").and_then(|c| c.as_i64()),
                id,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

/// Heuristic, display-only grouping. The UI labels it "inferred".
pub fn infer_category(id: &str) -> &'static str {
    let l = id.to_lowercase();
    if l.contains("embed") || l.contains("e5-") || l.contains("arctic-embed") {
        "embedding"
    } else if l.contains("rerank") {
        "reranking"
    } else if l.contains("guard") || l.contains("safety") || l.contains("shield") || l.contains("content-safety") {
        "safety"
    } else if l.contains("clip") || l.contains("parse") || l.contains("ocr") || l.contains("paddle") || l.contains("deplot") {
        "vision-utility"
    } else if l.contains("whisper") || l.contains("asr") || l.contains("tts") || l.contains("parakeet") {
        "speech"
    } else if l.contains("code") || l.contains("coder") || l.contains("starcoder") || l.contains("codestral") {
        "code"
    } else {
        "chat"
    }
}

pub fn parse_completion(body: &str) -> PResult<ChatResult> {
    let v: Value = serde_json::from_str(body).map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| ProviderError::Malformed("response has no choices".into()))?;
    let message = choice.get("message").ok_or_else(|| ProviderError::Malformed("response has no message".into()))?;
    // `content` may be null when a reasoning model spent its whole budget thinking.
    let content = match message.get("content") {
        Some(Value::String(c)) => c.clone(),
        Some(Value::Null) | None if reasoning_of(message).is_some() => String::new(),
        _ => return Err(ProviderError::Malformed("response has no message content".into())),
    };
    let reasoning = choice.get("message").and_then(reasoning_of).unwrap_or("").to_string();
    Ok(ChatResult {
        content,
        finish_reason: choice.get("finish_reason").and_then(|f| f.as_str()).map(String::from),
        streamed: false,
        usage: v.get("usage").cloned().filter(|u| !u.is_null()),
        had_reasoning: !reasoning.is_empty(),
        reasoning,
        model: String::new(),
        reported_model: reported_model(&v),
        first_token_ms: None,
        total_ms: 0,
    })
}

/// The `model` field of a provider response or stream chunk, if it carries a non-empty one.
fn reported_model(v: &Value) -> Option<String> {
    v.get("model").and_then(|m| m.as_str()).map(str::trim).filter(|m| !m.is_empty()).map(String::from)
}

/// Incremental Server-Sent-Events parser: returns the `data:` payload of each complete event.
#[derive(Default)]
pub struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(&chunk.replace("\r\n", "\n"));
        let mut out = Vec::new();
        while let Some(pos) = self.buf.find("\n\n") {
            let event: String = self.buf.drain(..pos + 2).collect();
            let data: Vec<&str> = event
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(|d| d.strip_prefix(' ').unwrap_or(d))
                .collect();
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
        }
        out
    }
}

/// Removes `<think>…</think>` reasoning sections that some models inline into content,
/// even when the tags are split across stream chunks. Reasoning is not displayed.
#[derive(Default)]
pub struct ThinkFilter {
    pending: String,
    inside: bool,
    pub saw_think: bool,
    /// Reasoning text removed from the content, not yet taken.
    thought: String,
}

impl ThinkFilter {
    pub fn push(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let mut out = String::new();
        loop {
            if self.inside {
                if let Some(p) = self.pending.find("</think>") {
                    self.thought.push_str(&self.pending[..p]);
                    self.pending.drain(..p + "</think>".len());
                    self.inside = false;
                    continue;
                }
                // keep a tail that could be the start of a closing tag
                let keep = partial_suffix(&self.pending, "</think>");
                let cut = self.pending.len() - keep;
                self.thought.push_str(&self.pending[..cut]);
                self.pending.drain(..cut);
                break;
            } else {
                if let Some(p) = self.pending.find("<think>") {
                    out.push_str(&self.pending[..p]);
                    self.pending.drain(..p + "<think>".len());
                    self.inside = true;
                    self.saw_think = true;
                    continue;
                }
                let keep = partial_suffix(&self.pending, "<think>");
                let cut = self.pending.len() - keep;
                out.push_str(&self.pending[..cut]);
                self.pending.drain(..cut);
                break;
            }
        }
        out
    }

    /// Reasoning removed since the last call.
    pub fn take_thought(&mut self) -> String {
        std::mem::take(&mut self.thought)
    }

    pub fn finish(&mut self) -> String {
        if self.inside {
            let rest = std::mem::take(&mut self.pending);
            self.thought.push_str(&rest);
            String::new()
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

/// Length of the longest suffix of `s` that is a proper prefix of `tag`.
fn partial_suffix(s: &str, tag: &str) -> usize {
    (1..tag.len()).rev().find(|&n| s.len() >= n && s.is_char_boundary(s.len() - n) && tag.starts_with(&s[s.len() - n..])).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer, key: Option<&str>) -> NvidiaClient {
        let mut c = NvidiaClient::new(&format!("http://127.0.0.1:{}/v1", server.address().port()), key.map(String::from)).unwrap();
        c.max_retries = 1;
        c
    }

    fn sse(chunks: &[&str]) -> String {
        let mut s = String::new();
        for c in chunks {
            let v = json!({"choices":[{"index":0,"delta":{"content": c},"finish_reason": null}]});
            s.push_str(&format!("data: {v}\n\n"));
        }
        s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
        s.push_str("data: [DONE]\n\n");
        s
    }

    #[test]
    fn request_body_shape() {
        let b = build_chat_body("meta/llama-3.1-8b-instruct", &[ChatMessage::user("hi")], &ChatOptions::default(), true);
        assert_eq!(b["model"], "meta/llama-3.1-8b-instruct");
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["stream"], true);
        assert!(b["max_tokens"].as_u64().unwrap() > 0);
    }

    #[test]
    fn base_url_rules() {
        assert_eq!(normalize_base_url("https://integrate.api.nvidia.com/v1/").unwrap(), DEFAULT_BASE_URL);
        assert_eq!(normalize_base_url("https://integrate.api.nvidia.com/v1/chat/completions").unwrap(), DEFAULT_BASE_URL);
        assert!(normalize_base_url("http://evil.example.com/v1").is_err());
        assert!(normalize_base_url("http://localhost:8000/v1").is_ok());
    }

    #[test]
    fn sse_parser_handles_split_chunks() {
        let mut p = SseParser::default();
        assert!(p.push("data: {\"a\":").is_empty());
        assert_eq!(p.push("1}\n\ndata: [DO"), vec!["{\"a\":1}"]);
        assert_eq!(p.push("NE]\r\n\r\n"), vec!["[DONE]"]);
    }

    #[test]
    fn think_filter_strips_reasoning_across_chunks() {
        let mut f = ThinkFilter::default();
        let mut out = String::new();
        for c in ["Hel", "lo <thi", "nk>secret plan</th", "ink>World", "<", "b>"] {
            out.push_str(&f.push(c));
        }
        out.push_str(&f.finish());
        assert_eq!(out, "Hello World<b>");
        assert!(f.saw_think);
    }

    #[test]
    fn classifies_errors() {
        assert_eq!(classify(401, r#"{"detail":"Authentication failed"}"#, None, None).kind(), "provider_auth");
        assert_eq!(classify(429, "{}", None, Some(3)).kind(), "provider_rate_limited");
        assert_eq!(classify(404, "{}", Some("x/y"), None).kind(), "provider_model_unavailable");
        assert_eq!(classify(402, "{}", None, None).kind(), "provider_quota");
        assert_eq!(classify(422, r#"{"detail":"bad param top_k"}"#, Some("m"), None).kind(), "provider_bad_request");
        assert_eq!(classify(400, r#"{"error":{"message":"Model foo not found"}}"#, Some("foo"), None).kind(), "provider_model_unavailable");
        assert_eq!(classify(503, "oops", None, None).kind(), "provider_server");
    }

    #[test]
    fn error_messages_redact_keys() {
        let e = classify(401, r#"{"detail":"bad key nvapi-AAAAAAAAAAAAAAAAAAAA"}"#, None, None);
        assert!(!e.to_string().contains("AAAAAAAA"));
    }

    #[test]
    fn parses_model_list_and_infers_categories() {
        let body = r#"{"object":"list","data":[
            {"id":"meta/llama-3.1-70b-instruct","object":"model","created":735790403,"owned_by":"meta"},
            {"id":"nvidia/nv-embedqa-e5-v5","object":"model","owned_by":"nvidia"},
            {"id":"qwen/qwen2.5-coder-32b-instruct","object":"model"},
            {"id":"meta/llama-3.1-70b-instruct","object":"model"},
            {"object":"model"}
        ]}"#;
        let m = parse_models(body).unwrap();
        assert_eq!(m.len(), 3);
        assert_eq!(m[0].id, "meta/llama-3.1-70b-instruct");
        assert_eq!(m[0].inferred_category, "chat");
        assert_eq!(m[1].inferred_category, "embedding");
        assert_eq!(m[2].inferred_category, "code");
        assert_eq!(m[2].owner.as_deref(), Some("qwen"));
        assert!(parse_models("{}").is_err());
        assert!(parse_models("<html>").is_err());
    }

    #[tokio::test]
    async fn list_models_sends_bearer_and_parses() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer nvapi-test-key-000000"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"data":[{"id":"a/b","owned_by":"a"}]}"#))
            .mount(&server)
            .await;
        let m = client(&server, Some("nvapi-test-key-000000")).list_models().await.unwrap();
        assert_eq!(m[0].id, "a/b");
    }

    #[tokio::test]
    async fn invalid_key_is_classified() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"status":401,"title":"Unauthorized","detail":"Authentication failed"}"#))
            .mount(&server)
            .await;
        let err = client(&server, Some("nvapi-wrong-key-00000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "provider_auth");
    }

    #[tokio::test]
    async fn missing_key_fails_without_network() {
        let server = MockServer::start().await;
        let err = client(&server, None)
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "provider_missing_key");
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rate_limit_retries_then_reports() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0").set_body_string(r#"{"detail":"Too many requests"}"#))
            .mount(&server)
            .await;
        let err = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "provider_rate_limited");
        assert_eq!(server.received_requests().await.unwrap().len(), 2, "one retry");
    }

    #[tokio::test]
    async fn streams_deltas_in_order() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse(&["Hel", "lo", " world"]), "text/event-stream"))
            .mount(&server)
            .await;
        let mut deltas = Vec::new();
        let r = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |d| deltas.push(d.to_string()))
            .await
            .unwrap();
        assert_eq!(r.content, "Hello world");
        assert_eq!(deltas.concat(), "Hello world");
        assert!(r.streamed);
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
    }

    #[tokio::test]
    async fn shrinks_max_tokens_when_model_limit_is_lower() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"max_tokens": 8192})))
            .respond_with(ResponseTemplate::new(422).set_body_string(r#"{"detail":"max_tokens must be less than or equal to 4096"}"#))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({"max_tokens": 4096})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse(&["ok"]), "text/event-stream"))
            .mount(&server)
            .await;
        let r = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(r.content, "ok");
    }

    #[tokio::test]
    async fn sse_with_wrong_content_type_is_parsed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sse(&["a", "b"])))
            .mount(&server)
            .await;
        let r = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(r.content, "ab");
    }

    #[tokio::test]
    async fn non_stream_json_response_is_supported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"choices":[{"message":{"role":"assistant","content":"<think>x</think>ok"},"finish_reason":"stop"}]}"#,
            ))
            .mount(&server)
            .await;
        let r = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(r.content, "ok");
        assert!(!r.streamed);
    }

    #[tokio::test]
    async fn malformed_stream_chunk_is_reported() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_raw("data: {not json\n\n", "text/event-stream"))
            .mount(&server)
            .await;
        let err = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "provider_malformed");
    }

    #[tokio::test]
    async fn cancellation_stops_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(10)).set_body_string("{}"))
            .mount(&server)
            .await;
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            c2.cancel();
        });
        let started = std::time::Instant::now();
        let err = client(&server, Some("nvapi-k-0000000000000"))
            .chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &cancel, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "cancelled");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn a_provider_that_accepts_the_request_but_never_answers_times_out_quickly() {
        // The field problem: the connection is accepted and then nothing arrives. Without a start timeout the call
        // waited for minutes (streaming requests had no limit at all before the first byte).
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(20)).set_body_string("{}"))
            .mount(&server)
            .await;
        let mut c = client(&server, Some("nvapi-k-0000000000000"));
        c.start_timeout = Duration::from_millis(400);
        c.max_retries = 0;
        let started = std::time::Instant::now();
        let err = c.chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {}).await.unwrap_err();
        assert_eq!(err.kind(), "provider_timeout");
        assert!(err.to_string().contains("did not start answering"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    fn think(on: bool) -> ChatOptions {
        ChatOptions { thinking: Some(on), ..Default::default() }
    }

    #[test]
    fn thinking_params_per_family() {
        let msgs = [ChatMessage::system("Be helpful."), ChatMessage::user("hi")];
        let body = |m: &str, o: &ChatOptions| build_chat_body(m, &msgs, o, true);
        // chat_template_kwargs.enable_thinking: Nemotron 3, Qwen3, GLM
        for m in ["nvidia/nemotron-3-super-120b-a12b", "nvidia/nemotron-3-nano-30b-a3b", "qwen/qwen3-235b-a22b", "qwen/qwen3.5-397b-a17b", "z-ai/glm4.7"] {
            assert_eq!(body(m, &think(false))["chat_template_kwargs"], json!({ "enable_thinking": false }), "{m}");
            assert_eq!(body(m, &think(true))["chat_template_kwargs"]["enable_thinking"], true, "{m}");
        }
        // DeepSeek V3.1+ uses "thinking"; R1 and V3.0 have no switch
        assert_eq!(body("deepseek-ai/deepseek-v3.1-terminus", &think(true))["chat_template_kwargs"], json!({ "thinking": true }));
        assert_eq!(body("deepseek-ai/deepseek-v3.2", &think(false))["chat_template_kwargs"], json!({ "thinking": false }));
        assert!(body("deepseek-ai/deepseek-r1", &think(true)).get("chat_template_kwargs").is_none());
        // Llama-Nemotron: system prompt switch
        let b = body("nvidia/llama-3.1-nemotron-ultra-253b-v1", &think(true));
        assert_eq!(b["messages"][0]["content"], "detailed thinking on\n\nBe helpful.");
        assert!(b.get("chat_template_kwargs").is_none());
        let b = body("nvidia/llama-3.3-nemotron-super-49b-v1.5", &think(false));
        assert_eq!(b["messages"][0]["content"], "/no_think\n\nBe helpful.");
        let b = build_chat_body("nvidia/llama-3.3-nemotron-super-49b-v1", &[ChatMessage::user("hi")], &think(false), true);
        assert_eq!(b["messages"][0], json!({ "role": "system", "content": "detailed thinking off" }));
        assert_eq!(b["messages"][1]["role"], "user");
        // gpt-oss: reasoning_effort
        assert_eq!(body("openai/gpt-oss-120b", &think(true))["reasoning_effort"], "high");
        assert_eq!(body("openai/gpt-oss-20b", &think(false))["reasoning_effort"], "low");
        assert_eq!(body("openai/gpt-oss-120b", &ChatOptions { reasoning_effort: Some("medium".into()), ..think(true) })["reasoning_effort"], "medium");
        // no switch, no default override
        for m in ["qwen/qwen3-coder-480b-a35b-instruct", "moonshotai/kimi-k2-instruct", "meta/llama-3.3-70b-instruct"] {
            let b = body(m, &think(true));
            assert!(b.get("chat_template_kwargs").is_none() && b.get("reasoning_effort").is_none(), "{m}");
            assert_eq!(b["messages"][0]["content"], "Be helpful.");
        }
        assert!(body("nvidia/nemotron-3-super-120b-a12b", &ChatOptions::default()).get("chat_template_kwargs").is_none());
        // usage is requested when streaming
        assert_eq!(body("a/b", &ChatOptions::default())["stream_options"], json!({ "include_usage": true }));
        assert!(build_chat_body("a/b", &msgs, &ChatOptions::default(), false).get("stream_options").is_none());
        let skip: HashSet<String> = ["chat_template_kwargs".to_string(), "stream_options".to_string()].into();
        let b = build_chat_body_with("z-ai/glm4.7", &msgs, &think(true), true, &skip);
        assert!(b.get("chat_template_kwargs").is_none() && b.get("stream_options").is_none());
    }

    #[test]
    fn version_parsing() {
        assert_eq!(version_after("deepseek-ai/deepseek-v3.1-terminus", "deepseek-v"), Some(3.1));
        assert_eq!(version_after("z-ai/glm-4.5", "glm"), Some(4.5));
        assert_eq!(version_after("moonshotai/kimi-k2.5", "kimi-k"), Some(2.5));
        assert_eq!(version_after("qwen/qwen3-coder", "/qwen"), Some(3.0));
        assert_eq!(version_after("nvidia/nemotron-nano-12b", "nemotron-"), None);
    }

    #[tokio::test]
    async fn unsupported_param_is_dropped_and_remembered() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({ "chat_template_kwargs": { "enable_thinking": true } })))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"error":{"message":"Unrecognized request argument supplied: chat_template_kwargs"}}"#))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(sse(&["ok"]), "text/event-stream")).mount(&server).await;
        let c = client(&server, Some("nvapi-k-0000000000000"));
        let model = "z-ai/glm4.7";
        let r = c.chat(model, &[ChatMessage::user("x")], &think(true), &CancellationToken::new(), |_| {}).await.unwrap();
        assert_eq!(r.content, "ok");
        assert_eq!(r.model, model);
        assert!(c.rejected_params(model).contains("chat_template_kwargs"));
        // a second call does not send it again (memory survives a fresh client sharing it)
        let c2 = NvidiaClient::new(c.base_url(), Some("nvapi-k-0000000000000".into())).unwrap().with_param_memory(c.params.clone());
        c2.chat(model, &[ChatMessage::user("x")], &think(true), &CancellationToken::new(), |_| {}).await.unwrap();
        let reqs = server.received_requests().await.unwrap();
        assert_eq!(reqs.len(), 3, "one rejected request, one retry, one clean request");
        let last: Value = serde_json::from_slice(&reqs[2].body).unwrap();
        assert!(last.get("chat_template_kwargs").is_none());
        assert!(last.get("stream_options").is_some(), "only the rejected parameter is dropped");
    }

    #[tokio::test]
    async fn stream_options_rejection_keeps_streaming() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(wiremock::matchers::body_partial_json(json!({ "stream_options": { "include_usage": true } })))
            .respond_with(ResponseTemplate::new(400).set_body_string(r#"{"detail":"stream_options is not supported"}"#))
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(sse(&["hi"]), "text/event-stream")).mount(&server).await;
        let r = client(&server, Some("nvapi-k-0000000000000")).chat("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |_| {}).await.unwrap();
        assert_eq!(r.content, "hi");
        assert!(r.streamed, "streaming itself was not turned off");
    }

    #[tokio::test]
    async fn usage_and_reasoning_are_streamed() {
        let server = MockServer::start().await;
        let body = [
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"Let me "},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"reasoning_content":"plan."},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{"content":"<think>inline</think>Answer"},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":30,"total_tokens":42}}),
        ]
        .iter()
        .map(|v| format!("data: {v}\n\n"))
        .collect::<String>()
            + "data: [DONE]\n\n";
        Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")).mount(&server).await;
        let (mut content, mut thoughts) = (String::new(), String::new());
        let r = client(&server, Some("nvapi-k-0000000000000"))
            .chat_with_reasoning("a/b", &[ChatMessage::user("x")], &ChatOptions::default(), &CancellationToken::new(), |d| content.push_str(d), |t| thoughts.push_str(t))
            .await
            .unwrap();
        assert_eq!(r.content, "Answer");
        assert_eq!(content, "Answer");
        assert_eq!(thoughts, "Let me plan.inline");
        assert_eq!(r.reasoning, "Let me plan.inline");
        assert!(r.had_reasoning);
        assert_eq!(r.usage.unwrap()["total_tokens"], 42);
        let sent: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(sent["stream_options"]["include_usage"], true);
    }

    #[test]
    fn reasoning_only_completion_is_not_malformed() {
        let r = parse_completion(r#"{"choices":[{"message":{"role":"assistant","content":null,"reasoning_content":"thinking..."},"finish_reason":"length"}],"usage":{"total_tokens":5}}"#).unwrap();
        assert_eq!(r.content, "");
        assert_eq!(r.reasoning, "thinking...");
        assert_eq!(r.finish_reason.as_deref(), Some("length"));
        assert!(parse_completion(r#"{"choices":[{"message":{"role":"assistant","content":null}}]}"#).is_err());
    }
}
