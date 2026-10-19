//! Provider adapters: the parts that differ between AI providers, kept free of networking so
//! every one of them is covered by deterministic contract tests.
//!
//! Three wire formats exist today:
//!
//! * **OpenAI chat completions** — NVIDIA NIM, OpenAI, and any OpenAI-compatible endpoint. The
//!   long-standing path in `nvidia.rs`; it is not re-implemented here.
//! * **Anthropic Messages** (`/v1/messages`, `x-api-key`, content blocks).
//! * **Google Gemini** (`generateContent`, `x-goog-api-key`, `contents`/`parts`).
//!
//! Genius drives tools through its own text protocol (`<tool>{…}</tool>` blocks that it parses and
//! executes itself), so no provider's native function-calling format is needed or used; the
//! adapters translate messages, images, sampling options, replies, streamed events, token usage
//! and errors. Usage is normalised to the OpenAI keys (`prompt_tokens`, `completion_tokens`,
//! `total_tokens`) **only from numbers the provider actually reported**.
//!
//! Verification level, stated plainly: NVIDIA is exercised live. Anthropic, OpenAI and Gemini are
//! covered by contract tests against the documented request/response shapes and by HTTP-level
//! tests against a local stand-in server; **no live call to those providers has been made**.

use crate::nvidia::{classify, ChatMessage, ChatOptions, ChatResult, ModelInfo, PResult, ProviderError};
use crate::redact::redact;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashSet;

pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Nvidia,
    OpenAi,
    Anthropic,
    Gemini,
    /// Any OpenAI-compatible endpoint at the configured base URL (self-hosted NIM, gateways, …).
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    OpenAiChat,
    AnthropicMessages,
    GeminiGenerate,
}

impl ProviderKind {
    pub const ALL: [ProviderKind; 5] = [ProviderKind::Nvidia, ProviderKind::OpenAi, ProviderKind::Anthropic, ProviderKind::Gemini, ProviderKind::Custom];

    pub fn id(self) -> &'static str {
        match self {
            ProviderKind::Nvidia => "nvidia",
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Gemini => "gemini",
            ProviderKind::Custom => "custom",
        }
    }
    pub fn from_id(id: &str) -> Option<ProviderKind> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
    /// An unknown id (an old or hand-edited setting) behaves as NVIDIA, the original provider.
    pub fn from_id_or_default(id: &str) -> ProviderKind {
        Self::from_id(id).unwrap_or(ProviderKind::Nvidia)
    }
    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::Nvidia => "NVIDIA NIM",
            ProviderKind::OpenAi => "OpenAI",
            ProviderKind::Anthropic => "Anthropic",
            ProviderKind::Gemini => "Google Gemini",
            ProviderKind::Custom => "OpenAI-compatible endpoint",
        }
    }
    pub fn wire(self) -> Wire {
        match self {
            ProviderKind::Anthropic => Wire::AnthropicMessages,
            ProviderKind::Gemini => Wire::GeminiGenerate,
            _ => Wire::OpenAiChat,
        }
    }
    pub fn default_base_url(self) -> &'static str {
        match self {
            ProviderKind::Nvidia => crate::nvidia::DEFAULT_BASE_URL,
            ProviderKind::OpenAi => "https://api.openai.com/v1",
            ProviderKind::Anthropic => "https://api.anthropic.com/v1",
            ProviderKind::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            ProviderKind::Custom => "",
        }
    }
    /// Credential-store entry name. NVIDIA keeps the name used since v0.1, so existing keys still work.
    pub fn key_name(self) -> String {
        format!("{}-api-key", self.id())
    }
    /// Host the provider is normally reached at (diagnostics warn when the base URL points elsewhere).
    pub fn expected_host(self) -> Option<&'static str> {
        match self {
            ProviderKind::Nvidia => Some("nvidia.com"),
            ProviderKind::OpenAi => Some("openai.com"),
            ProviderKind::Anthropic => Some("anthropic.com"),
            ProviderKind::Gemini => Some("googleapis.com"),
            ProviderKind::Custom => None,
        }
    }
    pub fn capabilities(self) -> ProviderCapabilities {
        let live = self == ProviderKind::Nvidia;
        ProviderCapabilities {
            provider: self.id(),
            label: self.label(),
            wire: match self.wire() {
                Wire::OpenAiChat => "openai-chat-completions",
                Wire::AnthropicMessages => "anthropic-messages",
                Wire::GeminiGenerate => "gemini-generate-content",
            },
            streaming: true,
            usage_reporting: true,
            model_listing: true,
            image_input: "by-model",
            auto_routing: live,
            tool_protocol: "genius-text-tools",
            verification: if live { "live" } else { "contract-tests" },
            notes: match self {
                ProviderKind::Nvidia => vec!["Auto model routing, reasoning switches and vision probing are tuned for NVIDIA NIM models."],
                ProviderKind::OpenAi => vec!["Standard chat-completions path. Choose the model yourself (Auto routing is NVIDIA-only).", "Verified by contract tests only; no live OpenAI call has been made."],
                ProviderKind::Anthropic => vec!["Native Messages API. Choose the model yourself (Auto routing is NVIDIA-only).", "Only `temperature` is sent (current Claude models reject temperature together with top_p); extended-thinking switches are not sent.", "Verified by contract tests only; no live Anthropic call has been made."],
                ProviderKind::Gemini => vec!["Native generateContent API. Choose the model yourself (Auto routing is NVIDIA-only).", "Thinking switches are not sent.", "Verified by contract tests only; no live Gemini call has been made."],
                ProviderKind::Custom => vec!["Works for servers that implement /models and /chat/completions the OpenAI way; provider-specific features are not assumed.", "Not verified against any particular server."],
            },
        }
    }
}

/// What Genius knows about a provider's integration, shown in Settings and diagnostics.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    pub provider: &'static str,
    pub label: &'static str,
    pub wire: &'static str,
    pub streaming: bool,
    /// The provider reports token counts (Genius keeps only what it reports).
    pub usage_reporting: bool,
    pub model_listing: bool,
    /// Image input depends on the model; Genius decides per model (see `router::is_vision`).
    pub image_input: &'static str,
    pub auto_routing: bool,
    pub tool_protocol: &'static str,
    /// "live" (run against the real API) or "contract-tests" (adapter tested, API not called).
    pub verification: &'static str,
    pub notes: Vec<&'static str>,
}

/// Model families of the native providers that are documented to accept image input. A vision probe
/// can still refute this for a particular model; it is not a guarantee.
pub fn declared_vision(model: &str) -> bool {
    let m = model.to_lowercase();
    let m = m.trim_start_matches("models/");
    if m.contains("embed") {
        return false;
    }
    m.starts_with("claude-") || m.starts_with("gemini-") || ["gpt-4o", "gpt-4.1", "gpt-4-turbo", "gpt-5", "chatgpt-4o"].iter().any(|p| m.starts_with(p))
}

// ------------------------------------------------------------------------------------ requests

/// Strip an endpoint suffix a user may have pasted, keeping the API root.
pub fn normalize_root(kind: ProviderKind, base: &str) -> String {
    let mut b = base.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/messages", "/models"] {
        if let Some(s) = b.strip_suffix(suffix) {
            b = s.to_string();
        }
    }
    if kind == ProviderKind::Gemini {
        // `…/v1beta/models/gemini-x:generateContent` pasted whole: keep `…/v1beta`.
        if let Some(i) = b.find("/models/") {
            b.truncate(i);
        }
    }
    b
}

pub fn chat_endpoint(kind: ProviderKind, base: &str, model: &str, stream: bool) -> String {
    match kind.wire() {
        Wire::OpenAiChat => format!("{base}/chat/completions"),
        Wire::AnthropicMessages => format!("{base}/messages"),
        Wire::GeminiGenerate => {
            let model = model.trim_start_matches("models/");
            if stream { format!("{base}/models/{model}:streamGenerateContent?alt=sse") } else { format!("{base}/models/{model}:generateContent") }
        }
    }
}

pub fn models_endpoint(kind: ProviderKind, base: &str) -> String {
    match kind.wire() {
        Wire::OpenAiChat => format!("{base}/models"),
        Wire::AnthropicMessages => format!("{base}/models?limit=1000"),
        Wire::GeminiGenerate => format!("{base}/models?pageSize=1000"),
    }
}

/// Authentication headers. The key appears here and nowhere else in this module.
pub fn auth_headers(kind: ProviderKind, key: &str) -> Vec<(&'static str, String)> {
    match kind.wire() {
        Wire::OpenAiChat => vec![("Authorization", format!("Bearer {key}"))],
        Wire::AnthropicMessages => vec![("x-api-key", key.to_string()), ("anthropic-version", ANTHROPIC_VERSION.to_string())],
        Wire::GeminiGenerate => vec![("x-goog-api-key", key.to_string())],
    }
}

/// Split a `data:image/png;base64,…` URL into (media type, base64 payload).
fn split_data_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let media = meta.strip_suffix(";base64")?;
    Some((media, data))
}

/// One conversation turn after merging: (role, text, image data URLs).
type Turn = (String, String, Vec<String>);

/// Merge consecutive messages of the same role (both APIs prefer strictly alternating turns).
fn merged_turns(messages: &[ChatMessage]) -> (Vec<String>, Vec<Turn>) {
    let mut system = Vec::new();
    let mut turns: Vec<Turn> = Vec::new();
    for m in messages {
        if m.role == "system" {
            system.push(m.content.clone());
            continue;
        }
        let role = if m.role == "assistant" { "assistant" } else { "user" }.to_string();
        match turns.last_mut() {
            Some(last) if last.0 == role => {
                last.1.push_str("\n\n");
                last.1.push_str(&m.content);
                last.2.extend(m.images.iter().cloned());
            }
            _ => turns.push((role, m.content.clone(), m.images.clone())),
        }
    }
    if turns.first().is_some_and(|t| t.0 != "user") {
        turns.insert(0, ("user".into(), "(start of conversation)".into(), vec![]));
    }
    (system, turns)
}

/// Request body for the native wires. `skip` lists optional parameters this model rejected before.
pub fn build_body(kind: ProviderKind, model: &str, messages: &[ChatMessage], opts: &ChatOptions, stream: bool, skip: &HashSet<String>) -> Value {
    let (system, turns) = merged_turns(messages);
    let system = system.join("\n\n");
    match kind.wire() {
        Wire::AnthropicMessages => {
            let msgs: Vec<Value> = turns
                .iter()
                .map(|(role, text, images)| {
                    if images.is_empty() {
                        return json!({ "role": role, "content": text });
                    }
                    let mut parts: Vec<Value> = images
                        .iter()
                        .map(|u| match split_data_url(u) {
                            Some((media, data)) => json!({ "type": "image", "source": { "type": "base64", "media_type": media, "data": data } }),
                            None => json!({ "type": "image", "source": { "type": "url", "url": u } }),
                        })
                        .collect();
                    parts.push(json!({ "type": "text", "text": text }));
                    json!({ "role": role, "content": parts })
                })
                .collect();
            let mut body = json!({ "model": model, "max_tokens": opts.max_tokens, "messages": msgs, "stream": stream });
            if !system.is_empty() {
                body["system"] = json!(system);
            }
            if !skip.contains("temperature") {
                body["temperature"] = json!(opts.temperature.clamp(0.0, 1.0));
            }
            body
        }
        Wire::GeminiGenerate => {
            let contents: Vec<Value> = turns
                .iter()
                .map(|(role, text, images)| {
                    let mut parts: Vec<Value> = images
                        .iter()
                        .map(|u| match split_data_url(u) {
                            Some((media, data)) => json!({ "inlineData": { "mimeType": media, "data": data } }),
                            None => json!({ "fileData": { "fileUri": u } }),
                        })
                        .collect();
                    parts.push(json!({ "text": text }));
                    json!({ "role": if role == "assistant" { "model" } else { "user" }, "parts": parts })
                })
                .collect();
            let mut generation = json!({ "maxOutputTokens": opts.max_tokens });
            if !skip.contains("temperature") {
                generation["temperature"] = json!(opts.temperature.clamp(0.0, 2.0));
            }
            if !skip.contains("top_p") {
                generation["topP"] = json!(opts.top_p.clamp(0.0, 1.0));
            }
            let mut body = json!({ "contents": contents, "generationConfig": generation });
            if !system.is_empty() {
                body["systemInstruction"] = json!({ "parts": [{ "text": system }] });
            }
            body
        }
        Wire::OpenAiChat => crate::nvidia::build_chat_body_with(model, messages, opts, stream, skip),
    }
}

// ----------------------------------------------------------------------------------- responses

fn usage(prompt: u64, completion: u64, total: Option<u64>) -> Value {
    json!({ "prompt_tokens": prompt, "completion_tokens": completion, "total_tokens": total.unwrap_or(prompt + completion) })
}

fn anthropic_finish(reason: &str) -> &'static str {
    match reason {
        "end_turn" | "stop_sequence" | "pause_turn" => "stop",
        "max_tokens" | "model_context_window_exceeded" => "length",
        "tool_use" => "tool_calls",
        "refusal" => "content_filter",
        _ => "stop",
    }
}

fn gemini_finish(reason: &str) -> &'static str {
    match reason {
        "STOP" => "stop",
        "MAX_TOKENS" => "length",
        _ => "content_filter", // SAFETY, RECITATION, BLOCKLIST, PROHIBITED_CONTENT, SPII, …
    }
}

fn anthropic_usage(u: &Value) -> Option<(u64, u64)> {
    let get = |k: &str| u.get(k).and_then(|v| v.as_u64());
    let input = get("input_tokens");
    let output = get("output_tokens");
    if input.is_none() && output.is_none() {
        return None;
    }
    // Cached prompt tokens are still prompt tokens.
    Some((input.unwrap_or(0) + get("cache_creation_input_tokens").unwrap_or(0) + get("cache_read_input_tokens").unwrap_or(0), output.unwrap_or(0)))
}

fn gemini_text(v: &Value, result: &mut ChatResult, on_delta: &mut dyn FnMut(&str), on_reasoning: &mut dyn FnMut(&str)) -> PResult<()> {
    if let Some(m) = v.get("modelVersion").and_then(|m| m.as_str()).filter(|m| !m.is_empty()) {
        result.reported_model.get_or_insert_with(|| m.to_string());
    }
    if let Some(u) = v.get("usageMetadata").filter(|u| u.is_object()) {
        let get = |k: &str| u.get(k).and_then(|v| v.as_u64());
        if get("promptTokenCount").is_some() || get("totalTokenCount").is_some() {
            let prompt = get("promptTokenCount").unwrap_or(0);
            let completion = get("candidatesTokenCount").unwrap_or(0) + get("thoughtsTokenCount").unwrap_or(0);
            result.usage = Some(usage(prompt, completion, get("totalTokenCount")));
        }
    }
    let Some(c) = v.get("candidates").and_then(|c| c.get(0)) else {
        if let Some(reason) = v.pointer("/promptFeedback/blockReason").and_then(|r| r.as_str()) {
            return Err(ProviderError::BadRequest { status: 400, message: format!("Gemini blocked the prompt ({reason}).") });
        }
        return Ok(());
    };
    for part in c.pointer("/content/parts").and_then(|p| p.as_array()).into_iter().flatten() {
        let Some(text) = part.get("text").and_then(|t| t.as_str()) else { continue };
        if part.get("thought").and_then(|t| t.as_bool()) == Some(true) {
            result.had_reasoning = true;
            on_reasoning(text);
            result.reasoning.push_str(text);
        } else {
            on_delta(text);
            result.content.push_str(text);
        }
    }
    if let Some(fr) = c.get("finishReason").and_then(|f| f.as_str()) {
        result.finish_reason = Some(gemini_finish(fr).into());
    }
    Ok(())
}

/// A complete (non-streamed) reply.
pub fn parse_completion(kind: ProviderKind, body: &str) -> PResult<ChatResult> {
    let v: Value = serde_json::from_str(body).map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;
    match kind.wire() {
        Wire::AnthropicMessages => {
            if v.get("type").and_then(|t| t.as_str()) == Some("error") {
                return Err(stream_error(&v));
            }
            let blocks = v.get("content").and_then(|c| c.as_array()).ok_or_else(|| ProviderError::Malformed("response has no content blocks".into()))?;
            let mut r = ChatResult::default();
            for b in blocks {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => r.content.push_str(b.get("text").and_then(|t| t.as_str()).unwrap_or("")),
                    Some("thinking") => {
                        r.had_reasoning = true;
                        r.reasoning.push_str(b.get("thinking").and_then(|t| t.as_str()).unwrap_or(""));
                    }
                    _ => {}
                }
            }
            r.finish_reason = v.get("stop_reason").and_then(|s| s.as_str()).map(|s| anthropic_finish(s).to_string());
            r.usage = v.get("usage").and_then(anthropic_usage).map(|(p, c)| usage(p, c, None));
            r.reported_model = v.get("model").and_then(|m| m.as_str()).filter(|m| !m.is_empty()).map(String::from);
            Ok(r)
        }
        Wire::GeminiGenerate => {
            if let Some(e) = v.get("error") {
                return Err(ProviderError::Server { status: 200, message: redact(&e.to_string()) });
            }
            let mut r = ChatResult::default();
            gemini_text(&v, &mut r, &mut |_| {}, &mut |_| {})?;
            if r.content.is_empty() && r.reasoning.is_empty() && r.finish_reason.is_none() {
                return Err(ProviderError::Malformed("response has no candidates".into()));
            }
            Ok(r)
        }
        Wire::OpenAiChat => crate::nvidia::parse_completion(body),
    }
}

/// An error object delivered inside a stream or a 200 response.
fn stream_error(v: &Value) -> ProviderError {
    let kind = v.pointer("/error/type").and_then(|t| t.as_str()).unwrap_or("");
    let message = redact(v.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("provider reported an error"));
    match kind {
        "authentication_error" | "permission_error" => ProviderError::Auth { status: 401, message },
        "rate_limit_error" => ProviderError::RateLimited { retry_after_secs: None, message },
        "overloaded_error" => ProviderError::Server { status: 529, message },
        "not_found_error" => ProviderError::ModelUnavailable { model: "?".into(), status: 404, message },
        "invalid_request_error" => ProviderError::BadRequest { status: 400, message },
        _ => ProviderError::Server { status: 500, message },
    }
}

/// Consumes streamed `data:` payloads of the native wires and builds the final [`ChatResult`].
pub struct StreamState {
    wire: Wire,
    pub result: ChatResult,
    /// The provider signalled the end of the reply (`message_stop`); Gemini ends by closing the stream.
    pub done: bool,
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
}

impl StreamState {
    pub fn new(kind: ProviderKind) -> Self {
        Self { wire: kind.wire(), result: ChatResult { streamed: true, ..Default::default() }, done: false, prompt_tokens: None, completion_tokens: None }
    }

    pub fn feed(&mut self, data: &str, on_delta: &mut dyn FnMut(&str), on_reasoning: &mut dyn FnMut(&str)) -> PResult<()> {
        let v: Value = serde_json::from_str(data).map_err(|e| ProviderError::Malformed(format!("invalid stream chunk: {e}")))?;
        match self.wire {
            Wire::AnthropicMessages => match v.get("type").and_then(|t| t.as_str()) {
                Some("message_start") => {
                    if let Some(m) = v.pointer("/message/model").and_then(|m| m.as_str()).filter(|m| !m.is_empty()) {
                        self.result.reported_model = Some(m.to_string());
                    }
                    if let Some((p, c)) = v.pointer("/message/usage").and_then(anthropic_usage) {
                        self.prompt_tokens = Some(p);
                        self.completion_tokens = Some(c);
                    }
                }
                Some("content_block_delta") => match v.pointer("/delta/type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        let t = v.pointer("/delta/text").and_then(|t| t.as_str()).unwrap_or("");
                        on_delta(t);
                        self.result.content.push_str(t);
                    }
                    Some("thinking_delta") => {
                        let t = v.pointer("/delta/thinking").and_then(|t| t.as_str()).unwrap_or("");
                        self.result.had_reasoning = true;
                        on_reasoning(t);
                        self.result.reasoning.push_str(t);
                    }
                    _ => {}
                },
                Some("message_delta") => {
                    if let Some(s) = v.pointer("/delta/stop_reason").and_then(|s| s.as_str()) {
                        self.result.finish_reason = Some(anthropic_finish(s).into());
                    }
                    if let Some(n) = v.pointer("/usage/output_tokens").and_then(|n| n.as_u64()) {
                        self.completion_tokens = Some(n);
                    }
                    if let Some(n) = v.pointer("/usage/input_tokens").and_then(|n| n.as_u64()) {
                        self.prompt_tokens = Some(n);
                    }
                }
                Some("message_stop") => self.done = true,
                Some("error") => return Err(stream_error(&v)),
                _ => {} // ping, content_block_start/stop, …
            },
            Wire::GeminiGenerate => {
                if v.get("error").is_some() {
                    return Err(ProviderError::Server { status: 200, message: redact(&v["error"].to_string()) });
                }
                gemini_text(&v, &mut self.result, on_delta, on_reasoning)?;
            }
            Wire::OpenAiChat => {}
        }
        Ok(())
    }

    pub fn finish(mut self) -> PResult<ChatResult> {
        if self.wire == Wire::AnthropicMessages && (self.prompt_tokens.is_some() || self.completion_tokens.is_some()) {
            self.result.usage = Some(usage(self.prompt_tokens.unwrap_or(0), self.completion_tokens.unwrap_or(0), None));
        }
        if self.result.finish_reason.is_none() && self.result.content.is_empty() && self.result.reasoning.is_empty() {
            return Err(ProviderError::Malformed("stream ended without any content".into()));
        }
        Ok(self.result)
    }
}

// --------------------------------------------------------------------------------------- models

pub fn parse_models(kind: ProviderKind, body: &str) -> PResult<Vec<ModelInfo>> {
    if kind.wire() != Wire::GeminiGenerate {
        let mut models = crate::nvidia::parse_models(body)?;
        if kind == ProviderKind::Anthropic {
            for m in &mut models {
                m.owner = Some("anthropic".into());
            }
        }
        return Ok(models);
    }
    let v: Value = serde_json::from_str(body).map_err(|e| ProviderError::Malformed(format!("model list is not JSON: {e}")))?;
    let arr = v.get("models").and_then(|m| m.as_array()).ok_or_else(|| ProviderError::Malformed("model list has no 'models' array".into()))?;
    let mut out: Vec<ModelInfo> = arr
        .iter()
        .filter(|m| m.get("supportedGenerationMethods").and_then(|s| s.as_array()).is_none_or(|s| s.iter().any(|g| g == "generateContent")))
        .filter_map(|m| {
            let id = m.get("name")?.as_str()?.trim_start_matches("models/").to_string();
            (!id.is_empty()).then(|| ModelInfo { inferred_category: crate::nvidia::infer_category(&id).into(), owner: Some("google".into()), created: None, id })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

// ---------------------------------------------------------------------------------------- errors

/// Provider-aware HTTP error classification; falls back to the generic one.
pub fn classify_error(kind: ProviderKind, status: u16, body: &str, model: Option<&str>, retry_after: Option<u64>) -> ProviderError {
    let lower = body.to_lowercase();
    match kind.wire() {
        // Gemini reports an invalid key as 400 INVALID_ARGUMENT / API_KEY_INVALID, not 401.
        Wire::GeminiGenerate if status == 400 && (lower.contains("api_key_invalid") || lower.contains("api key not valid")) => {
            return ProviderError::Auth { status, message: "Google rejected the API key (API key not valid).".into() };
        }
        // Anthropic reports an empty balance as 400 invalid_request_error.
        Wire::AnthropicMessages if status == 400 && lower.contains("credit balance") => {
            return ProviderError::Quota { status, message: "Anthropic reports the credit balance is too low.".into() };
        }
        Wire::AnthropicMessages if status == 529 => {
            return ProviderError::Server { status, message: "Anthropic is overloaded; try again shortly.".into() };
        }
        _ => {}
    }
    classify(status, body, model, retry_after)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &str = "data:image/png;base64,iVBORw0KGgo=";

    fn conv() -> Vec<ChatMessage> {
        let mut with_image = ChatMessage::user("What colour is this?");
        with_image.images = vec![PNG.into()];
        vec![ChatMessage::system("You are Genius."), ChatMessage::user("ctx"), ChatMessage::assistant("Understood."), ChatMessage::user("more ctx"), with_image]
    }

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn kinds_round_trip_and_unknown_ids_fall_back_to_nvidia() {
        for k in ProviderKind::ALL {
            assert_eq!(ProviderKind::from_id(k.id()), Some(k));
        }
        assert_eq!(ProviderKind::from_id_or_default("bogus"), ProviderKind::Nvidia);
        assert_eq!(ProviderKind::Nvidia.key_name(), crate::secrets::NVIDIA_KEY, "existing NVIDIA keys keep working");
        assert_eq!(ProviderKind::Anthropic.key_name(), "anthropic-api-key");
        let honest = |k: ProviderKind| k.capabilities().verification;
        assert_eq!(honest(ProviderKind::Nvidia), "live");
        for k in [ProviderKind::OpenAi, ProviderKind::Anthropic, ProviderKind::Gemini, ProviderKind::Custom] {
            assert_eq!(honest(k), "contract-tests", "{k:?} must not claim live verification");
            assert!(!k.capabilities().auto_routing);
        }
    }

    #[test]
    fn declared_vision_covers_native_families_without_claiming_nvidia_text_models() {
        for m in ["claude-sonnet-5-5", "gemini-2.5-flash", "models/gemini-2.5-pro", "gpt-4o-mini", "gpt-5.1"] {
            assert!(declared_vision(m), "{m}");
        }
        for m in ["text-embedding-004", "gemini-embedding-001", "openai/gpt-oss-20b", "nvidia/nemotron-3-super-120b-a12b", "gemma-3-27b"] {
            assert!(!declared_vision(m), "{m}");
        }
    }

    #[test]
    fn endpoints_and_auth_follow_each_providers_documented_shape() {
        let a = ProviderKind::Anthropic;
        assert_eq!(chat_endpoint(a, "https://api.anthropic.com/v1", "claude-x", true), "https://api.anthropic.com/v1/messages");
        let h = auth_headers(a, "sk-ant-1");
        assert!(h.contains(&("x-api-key", "sk-ant-1".into())) && h.contains(&("anthropic-version", ANTHROPIC_VERSION.into())));
        let g = ProviderKind::Gemini;
        let base = "https://generativelanguage.googleapis.com/v1beta";
        assert_eq!(chat_endpoint(g, base, "models/gemini-2.5-pro", true), format!("{base}/models/gemini-2.5-pro:streamGenerateContent?alt=sse"));
        assert_eq!(chat_endpoint(g, base, "gemini-2.5-pro", false), format!("{base}/models/gemini-2.5-pro:generateContent"));
        assert_eq!(auth_headers(g, "AIza1"), vec![("x-goog-api-key", "AIza1".to_string())], "the key goes in a header, never in the URL");
        assert!(!chat_endpoint(g, base, "m", true).contains("key="));
        assert_eq!(auth_headers(ProviderKind::OpenAi, "sk-1"), vec![("Authorization", "Bearer sk-1".to_string())]);
        assert_eq!(chat_endpoint(ProviderKind::OpenAi, "https://api.openai.com/v1", "gpt", true), "https://api.openai.com/v1/chat/completions");
        assert_eq!(normalize_root(a, "https://api.anthropic.com/v1/messages/"), "https://api.anthropic.com/v1");
        assert_eq!(normalize_root(g, &format!("{base}/models/gemini-2.5-pro:generateContent")), base);
    }

    #[test]
    fn anthropic_body_has_top_level_system_alternating_turns_and_base64_images() {
        let opts = ChatOptions { temperature: 1.7, top_p: 0.9, max_tokens: 777, stream: true, thinking: Some(true), reasoning_effort: None };
        let b = build_body(ProviderKind::Anthropic, "claude-x", &conv(), &opts, true, &none());
        assert_eq!(b["system"], "You are Genius.");
        assert_eq!(b["max_tokens"], 777);
        assert_eq!(b["stream"], true);
        assert!((b["temperature"].as_f64().unwrap() - 1.0).abs() < 1e-9, "clamped to Anthropic's 0–1 range");
        assert!(b.get("top_p").is_none(), "never sent together with temperature");
        assert!(b.get("thinking").is_none(), "thinking switches are not sent");
        let m = b["messages"].as_array().unwrap();
        let roles: Vec<&str> = m.iter().map(|x| x["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"], "consecutive user turns are merged");
        let img = &m[2]["content"][0];
        assert_eq!(img["type"], "image");
        assert_eq!(img["source"], json!({ "type": "base64", "media_type": "image/png", "data": "iVBORw0KGgo=" }));
        assert_eq!(m[2]["content"][1]["text"], "more ctx\n\nWhat colour is this?");
        let skipped = build_body(ProviderKind::Anthropic, "m", &conv(), &opts, false, &HashSet::from(["temperature".to_string()]));
        assert!(skipped.get("temperature").is_none(), "a parameter the model rejected is left out");
    }

    #[test]
    fn a_conversation_that_starts_with_the_assistant_gets_a_user_opener() {
        let msgs = vec![ChatMessage::assistant("hi"), ChatMessage::user("yo")];
        let b = build_body(ProviderKind::Anthropic, "m", &msgs, &ChatOptions::default(), false, &none());
        assert_eq!(b["messages"][0]["role"], "user");
        assert!(b.get("system").is_none());
    }

    #[test]
    fn gemini_body_uses_contents_parts_inline_data_and_system_instruction() {
        let opts = ChatOptions { temperature: 0.3, top_p: 0.8, max_tokens: 500, stream: true, thinking: None, reasoning_effort: None };
        let b = build_body(ProviderKind::Gemini, "gemini-x", &conv(), &opts, true, &none());
        assert_eq!(b["systemInstruction"]["parts"][0]["text"], "You are Genius.");
        assert_eq!(b["generationConfig"]["maxOutputTokens"], 500);
        assert!((b["generationConfig"]["temperature"].as_f64().unwrap() - 0.3).abs() < 1e-6);
        assert!(b.get("model").is_none() && b.get("stream").is_none(), "model is in the URL, streaming is chosen by the endpoint");
        let c = b["contents"].as_array().unwrap();
        assert_eq!(c.iter().map(|x| x["role"].as_str().unwrap()).collect::<Vec<_>>(), ["user", "model", "user"]);
        assert_eq!(c[2]["parts"][0]["inlineData"], json!({ "mimeType": "image/png", "data": "iVBORw0KGgo=" }));
        assert_eq!(c[2]["parts"][1]["text"], "more ctx\n\nWhat colour is this?");
    }

    #[test]
    fn openai_wire_is_the_existing_body_unchanged() {
        let opts = ChatOptions::default();
        let a = build_body(ProviderKind::OpenAi, "gpt-x", &conv(), &opts, true, &none());
        assert_eq!(a, crate::nvidia::build_chat_body_with("gpt-x", &conv(), &opts, true, &none()));
        assert_eq!(a["messages"][4]["content"][1]["image_url"]["url"], PNG);
    }

    #[test]
    fn anthropic_reply_maps_text_thinking_stop_reason_and_measured_usage() {
        let body = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-x-20260101","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Hello "},{"type":"text","text":"there"}],"stop_reason":"max_tokens","usage":{"input_tokens":12,"output_tokens":5,"cache_read_input_tokens":3}}"#;
        let r = parse_completion(ProviderKind::Anthropic, body).unwrap();
        assert_eq!(r.content, "Hello there");
        assert_eq!(r.reasoning, "hmm");
        assert!(r.had_reasoning);
        assert_eq!(r.finish_reason.as_deref(), Some("length"), "max_tokens → the same 'length' the agent loop already handles");
        assert_eq!(r.reported_model.as_deref(), Some("claude-x-20260101"));
        assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 15, "completion_tokens": 5, "total_tokens": 20 }));
        let no_usage = parse_completion(ProviderKind::Anthropic, r#"{"content":[{"type":"text","text":"x"}],"stop_reason":"end_turn"}"#).unwrap();
        assert!(no_usage.usage.is_none(), "no usage block → no invented counts");
        assert!(matches!(parse_completion(ProviderKind::Anthropic, r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#), Err(ProviderError::Server { status: 529, .. })));
    }

    #[test]
    fn gemini_reply_maps_parts_thoughts_finish_reason_and_usage() {
        let body = r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"plan","thought":true},{"text":"Answer"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":4,"thoughtsTokenCount":2,"totalTokenCount":16},"modelVersion":"gemini-x-001"}"#;
        let r = parse_completion(ProviderKind::Gemini, body).unwrap();
        assert_eq!((r.content.as_str(), r.reasoning.as_str()), ("Answer", "plan"));
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
        assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 10, "completion_tokens": 6, "total_tokens": 16 }));
        assert_eq!(r.reported_model.as_deref(), Some("gemini-x-001"));
        let cut = parse_completion(ProviderKind::Gemini, r#"{"candidates":[{"content":{"parts":[{"text":"par"}]},"finishReason":"MAX_TOKENS"}]}"#).unwrap();
        assert_eq!(cut.finish_reason.as_deref(), Some("length"));
        let blocked = parse_completion(ProviderKind::Gemini, r#"{"promptFeedback":{"blockReason":"SAFETY"}}"#);
        assert!(matches!(blocked, Err(ProviderError::BadRequest { .. })), "{blocked:?}");
    }

    fn stream(kind: ProviderKind, events: &[&str]) -> (PResult<ChatResult>, String, String) {
        let mut s = StreamState::new(kind);
        let (mut text, mut thought) = (String::new(), String::new());
        for e in events {
            if let Err(err) = s.feed(e, &mut |d| text.push_str(d), &mut |d| thought.push_str(d)) {
                return (Err(err), text, thought);
            }
        }
        (s.finish(), text, thought)
    }

    #[test]
    fn anthropic_stream_accumulates_text_thinking_usage_and_stop() {
        let ev = [
            r#"{"type":"message_start","message":{"id":"m","model":"claude-x","usage":{"input_tokens":25,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"let me see"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let (r, text, thought) = stream(ProviderKind::Anthropic, &ev);
        let r = r.unwrap();
        assert_eq!((text.as_str(), thought.as_str()), ("Hello", "let me see"), "callbacks received the deltas as they arrived");
        assert_eq!(r.content, "Hello");
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
        assert_eq!(r.reported_model.as_deref(), Some("claude-x"));
        assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 25, "completion_tokens": 15, "total_tokens": 40 }));
        assert!(r.streamed);
    }

    #[test]
    fn stream_errors_are_classified_not_swallowed() {
        let (r, _, _) = stream(ProviderKind::Anthropic, &[r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#]);
        assert!(matches!(r, Err(ProviderError::Server { status: 529, .. })));
        let (r, _, _) = stream(ProviderKind::Anthropic, &["not json"]);
        assert!(matches!(r, Err(ProviderError::Malformed(_))));
        let (r, _, _) = stream(ProviderKind::Gemini, &[]);
        assert!(matches!(r, Err(ProviderError::Malformed(_))), "an empty stream is an error, not an empty answer");
    }

    #[test]
    fn gemini_stream_accumulates_chunks_and_keeps_the_last_usage() {
        let ev = [
            r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"Hel"}]}}],"usageMetadata":{"promptTokenCount":7,"totalTokenCount":7}}"#,
            r#"{"candidates":[{"content":{"role":"model","parts":[{"text":"lo"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":2,"totalTokenCount":9},"modelVersion":"gemini-x"}"#,
        ];
        let (r, text, _) = stream(ProviderKind::Gemini, &ev);
        let r = r.unwrap();
        assert_eq!((text.as_str(), r.content.as_str()), ("Hello", "Hello"));
        assert_eq!(r.usage.unwrap(), json!({ "prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9 }));
        assert_eq!(r.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn model_lists_parse_per_provider() {
        let g = r#"{"models":[{"name":"models/gemini-2.5-pro","supportedGenerationMethods":["generateContent","countTokens"]},{"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]},{"name":"models/gemini-2.5-flash"}]}"#;
        let ids: Vec<String> = parse_models(ProviderKind::Gemini, g).unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["gemini-2.5-flash", "gemini-2.5-pro"], "prefix stripped, non-generation models dropped");
        let a = r#"{"data":[{"type":"model","id":"claude-x","display_name":"Claude X","created_at":"2026-01-01T00:00:00Z"}],"has_more":false}"#;
        let m = parse_models(ProviderKind::Anthropic, a).unwrap();
        assert_eq!((m[0].id.as_str(), m[0].owner.as_deref()), ("claude-x", Some("anthropic")));
        assert!(parse_models(ProviderKind::Gemini, "{}").is_err());
    }

    #[test]
    fn provider_specific_error_shapes_map_to_the_right_kind() {
        let k = |kind, status, body: &str| classify_error(kind, status, body, Some("m"), None).kind();
        assert_eq!(k(ProviderKind::Gemini, 400, r#"{"error":{"code":400,"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#), "provider_auth");
        assert_eq!(k(ProviderKind::Gemini, 404, r#"{"error":{"message":"models/x is not found","status":"NOT_FOUND"}}"#), "provider_model_unavailable");
        assert_eq!(k(ProviderKind::Gemini, 429, r#"{"error":{"status":"RESOURCE_EXHAUSTED","message":"quota"}}"#), "provider_rate_limited");
        assert_eq!(k(ProviderKind::Anthropic, 401, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#), "provider_auth");
        assert_eq!(k(ProviderKind::Anthropic, 400, r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#), "provider_quota");
        assert_eq!(k(ProviderKind::Anthropic, 529, r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#), "provider_server");
        assert_eq!(k(ProviderKind::Anthropic, 429, r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#), "provider_rate_limited");
        assert_eq!(k(ProviderKind::OpenAi, 401, r#"{"error":{"message":"Incorrect API key provided: sk-abc","type":"invalid_request_error"}}"#), "provider_auth");
    }

    #[test]
    fn error_messages_never_carry_the_key() {
        let e = classify_error(ProviderKind::OpenAi, 401, r#"{"error":{"message":"Incorrect API key provided: sk-proj-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"}}"#, None, None);
        assert!(!e.to_string().contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"), "{e}");
    }
}
