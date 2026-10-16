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
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::redact::redact;

pub const DEFAULT_BASE_URL: &str = "https://integrate.api.nvidia.com/v1";

#[derive(Debug, Clone, thiserror::Error, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderError {
    #[error("No NVIDIA API key is saved. Add one in Settings → AI Provider.")]
    MissingKey,
    #[error("No model is selected. Choose a model in Settings → AI Provider.")]
    NoModel,
    #[error("NVIDIA rejected the API key (HTTP {status}). Check that the key is correct and active. {message}")]
    Auth { status: u16, message: String },
    #[error("The model '{model}' is not available to this key (HTTP {status}). {message}")]
    ModelUnavailable { model: String, status: u16, message: String },
    #[error("Rate limited by NVIDIA (HTTP 429). {message}")]
    RateLimited { retry_after_secs: Option<u64>, message: String },
    #[error("NVIDIA reports the account has no remaining credits or quota (HTTP {status}). {message}")]
    Quota { status: u16, message: String },
    #[error("The request was rejected (HTTP {status}); the selected model may not support a parameter Genius sent. {message}")]
    BadRequest { status: u16, message: String },
    #[error("NVIDIA server error (HTTP {status}). Try again shortly. {message}")]
    Server { status: u16, message: String },
    #[error("Network error contacting NVIDIA: {0}")]
    Network(String),
    #[error("The request to NVIDIA timed out: {0}")]
    Timeout(String),
    #[error("Unexpected response from NVIDIA: {0}")]
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
            ProviderError::RateLimited { .. } | ProviderError::Server { status: 502..=504, .. } | ProviderError::Network(_)
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
}

impl ChatMessage {
    pub fn system(c: impl Into<String>) -> Self {
        Self { role: "system".into(), content: c.into() }
    }
    pub fn user(c: impl Into<String>) -> Self {
        Self { role: "user".into(), content: c.into() }
    }
    pub fn assistant(c: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: c.into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOptions {
    pub temperature: f32,
    pub top_p: f32,
    pub max_tokens: u32,
    pub stream: bool,
}

impl Default for ChatOptions {
    fn default() -> Self {
        Self { temperature: 0.2, top_p: 0.95, max_tokens: 8192, stream: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ChatResult {
    pub content: String,
    pub finish_reason: Option<String>,
    pub streamed: bool,
    pub usage: Option<Value>,
    /// True when the model emitted separate reasoning tokens (not shown, only counted).
    pub had_reasoning: bool,
}

#[derive(Clone)]
pub struct NvidiaClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    pub idle_timeout: Duration,
    pub max_retries: u32,
}

impl NvidiaClient {
    pub fn new(base_url: &str, api_key: Option<String>) -> PResult<Self> {
        let base = normalize_base_url(base_url).map_err(ProviderError::Malformed)?;
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .user_agent(concat!("ProjectGenius/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| ProviderError::Network(e.to_string()))?;
        Ok(Self { http, base_url: base, api_key, idle_timeout: Duration::from_secs(180), max_retries: 3 })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn key(&self) -> PResult<&str> {
        self.api_key.as_deref().ok_or(ProviderError::MissingKey)
    }

    /// GET /models. Authenticated when a key is available.
    pub async fn list_models(&self) -> PResult<Vec<ModelInfo>> {
        let mut req = self.http.get(format!("{}/models", self.base_url)).timeout(Duration::from_secs(30));
        if let Some(k) = &self.api_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.map_err(net_err)?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(net_err)?;
        if !(200..300).contains(&status) {
            return Err(classify(status, &body, None, None));
        }
        parse_models(&body)
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
        // Moderate sampling: some hosted models reject temperature 0.
        let opts = ChatOptions { temperature: 0.2, top_p: 0.7, max_tokens: 16, stream: true };
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
    pub async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        opts: &ChatOptions,
        cancel: &CancellationToken,
        mut on_delta: impl FnMut(&str),
    ) -> PResult<ChatResult> {
        if model.trim().is_empty() {
            return Err(ProviderError::NoModel);
        }
        let mut attempt = 0;
        let mut stream = opts.stream;
        let mut opts = opts.clone();
        loop {
            attempt += 1;
            let res = self.chat_once(model, messages, &opts, stream, cancel, &mut on_delta).await;
            match res {
                Ok(r) => return Ok(r),
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
        messages: &[ChatMessage],
        opts: &ChatOptions,
        stream: bool,
        cancel: &CancellationToken,
        on_delta: &mut impl FnMut(&str),
    ) -> PResult<ChatResult> {
        let key = self.key()?;
        let body = build_chat_body(model, messages, opts, stream);
        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(key)
            .header("Accept", if stream { "text/event-stream" } else { "application/json" })
            .json(&body);
        if !stream {
            req = req.timeout(Duration::from_secs(600));
        }
        let send = req.send();
        let resp = tokio::select! {
            _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
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
            return Err(classify(status, &text, Some(model), retry_after));
        }
        let is_sse = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("text/event-stream"))
            .unwrap_or(false);

        if !is_sse {
            let text = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                t = resp.text() => t.map_err(net_err)?,
            };
            if stream && text.trim_start().starts_with("data:") {
                // Server sent SSE with a wrong content-type: parse it as a stream anyway.
                return parse_sse_body(&text, on_delta);
            }
            let mut r = parse_completion(&text)?;
            let mut filter = ThinkFilter::default();
            r.content = filter.push(&r.content) + &filter.finish();
            if !r.content.is_empty() {
                on_delta(&r.content);
            }
            return Ok(r);
        }

        let mut result = ChatResult { streamed: true, ..Default::default() };
        let mut sse = SseParser::default();
        let mut filter = ThinkFilter::default();
        let mut bytes = resp.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::Cancelled),
                _ = tokio::time::sleep(self.idle_timeout) => {
                    return Err(ProviderError::Timeout(format!("no data received for {}s while streaming", self.idle_timeout.as_secs())))
                }
                n = bytes.next() => n,
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(net_err)?;
            for data in sse.push(&String::from_utf8_lossy(&chunk)) {
                if data.trim() == "[DONE]" {
                    let tail = filter.finish();
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
                let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else { continue };
                let delta = &choice["delta"];
                if delta.get("reasoning_content").and_then(|r| r.as_str()).is_some_and(|r| !r.is_empty()) {
                    result.had_reasoning = true;
                }
                if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
                    let visible = filter.push(text);
                    if filter.saw_think {
                        result.had_reasoning = true;
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
        if !tail.is_empty() {
            on_delta(&tail);
            result.content.push_str(&tail);
        }
        if result.finish_reason.is_none() && result.content.is_empty() {
            return Err(ProviderError::Malformed("stream ended without any content".into()));
        }
        Ok(result)
    }
}

/// Parse a fully-buffered SSE body (used when a server mislabels its content type).
fn parse_sse_body(text: &str, on_delta: &mut impl FnMut(&str)) -> PResult<ChatResult> {
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
    result.had_reasoning = filter.saw_think;
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

pub fn build_chat_body(model: &str, messages: &[ChatMessage], opts: &ChatOptions, stream: bool) -> Value {
    json!({
        "model": model,
        "messages": messages,
        "temperature": opts.temperature,
        "top_p": opts.top_p,
        "max_tokens": opts.max_tokens,
        "stream": stream,
    })
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
        404 => ProviderError::ModelUnavailable { model: model.unwrap_or("?").into(), status, message },
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
    let content = choice
        .pointer("/message/content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| ProviderError::Malformed("response has no message content".into()))?
        .to_string();
    Ok(ChatResult {
        content,
        finish_reason: choice.get("finish_reason").and_then(|f| f.as_str()).map(String::from),
        streamed: false,
        usage: v.get("usage").cloned(),
        had_reasoning: choice.pointer("/message/reasoning_content").is_some_and(|r| !r.is_null()),
    })
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
}

impl ThinkFilter {
    pub fn push(&mut self, text: &str) -> String {
        self.pending.push_str(text);
        let mut out = String::new();
        loop {
            if self.inside {
                if let Some(p) = self.pending.find("</think>") {
                    self.pending.drain(..p + "</think>".len());
                    self.inside = false;
                    continue;
                }
                // keep a tail that could be the start of a closing tag
                let keep = partial_suffix(&self.pending, "</think>");
                let cut = self.pending.len() - keep;
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

    pub fn finish(&mut self) -> String {
        if self.inside {
            self.pending.clear();
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
}
