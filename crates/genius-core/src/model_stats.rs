//! Observed per-model performance, so Auto mode learns which models actually work for this
//! user's key: successes, failures, malformed tool calls, context overflows and a smoothed
//! (EWMA) latency. Stats are persisted in the SQLite settings table (`model_stats`) so the
//! knowledge survives restarts, and they are bounded: counts decay (halve) once a model has
//! many observations, and only the most recently seen models are kept.
//!
//! Sources of observations:
//! * every routed AI call ([`Genius::ai_call_opts`](crate::app::Genius)): success + time to
//!   first token, or the classified provider error ([`outcome_for_error`]);
//! * model probes (Settings → Verify / Test connection): latency, chat and streaming support;
//! * the agent loop, which reports [`Outcome::MalformedToolCall`] when a model's tool call
//!   cannot be parsed (via [`Genius::record_model_outcome`]).
//!
//! The router ([`crate::router`]) combines these with its static family prior.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::app::Genius;
use crate::nvidia::{ProbeResult, ProviderError};

/// Settings key the stats are stored under.
pub const STATS_KEY: &str = "model_stats";
/// At most this many models are remembered (least recently updated are dropped).
pub const MAX_MODELS: usize = 200;
/// Once a model has this many observations, all its counts are halved (recent behaviour wins).
pub const DECAY_AT: u32 = 200;
/// Weight of the newest latency sample in the moving average.
const LATENCY_ALPHA: f64 = 0.3;
/// A model that was unavailable more recently than it succeeded is avoided for this long.
pub const UNAVAILABLE_TTL_SECS: i64 = 24 * 3600;
/// Rough characters per token when converting a provider's token limit to characters
/// (conservative: real text is closer to 4).
pub const CHARS_PER_TOKEN_LIMIT: u64 = 3;

/// What happened when Genius used a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The model answered; `latency_ms` is the time to the first token (or the whole call).
    Success { latency_ms: u64 },
    /// The model's reply contained a tool call Genius could not parse.
    MalformedToolCall,
    /// The model is not available to this key (404/410, "model not found").
    Unavailable,
    /// The request did not fit the model's context window.
    ContextTooLong,
    /// Server error, timeout, empty or broken reply, rejected request.
    Failed,
}

/// Observed behaviour of one model.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelStats {
    pub successes: u32,
    pub failures: u32,
    pub malformed_tool_calls: u32,
    pub unavailable: u32,
    pub context_overflows: u32,
    /// Smoothed time to first token in milliseconds.
    pub latency_ms: Option<f64>,
    pub latency_samples: u32,
    /// Largest prompt (characters) the model answered successfully.
    pub max_ok_prompt_chars: u64,
    /// Observed context limit in characters (smallest prompt that overflowed, or the
    /// provider-reported token limit converted to characters).
    pub context_limit_chars: Option<u64>,
    /// Streaming support as seen by a probe.
    pub streams: Option<bool>,
    /// Unix seconds.
    pub last_success: Option<i64>,
    pub last_unavailable: Option<i64>,
    pub updated: i64,
}

impl ModelStats {
    /// Calls that ended with an answer or a model-specific failure.
    pub fn calls(&self) -> u32 {
        self.successes + self.failures
    }

    /// Failure rate with a prior of two successful calls, so one bad call is not fatal.
    pub fn failure_rate(&self) -> f64 {
        self.failures as f64 / (self.calls() as f64 + 2.0)
    }

    /// Malformed tool calls per successful reply (smoothed, capped at 1).
    pub fn malformed_rate(&self) -> f64 {
        (self.malformed_tool_calls as f64 / (self.successes as f64 + 2.0)).min(1.0)
    }

    /// Raw success percentage for display (None without data).
    pub fn success_pct(&self) -> Option<f64> {
        (self.calls() > 0).then(|| 100.0 * self.successes as f64 / self.calls() as f64)
    }

    /// Unavailable more recently than it last succeeded, within [`UNAVAILABLE_TTL_SECS`].
    pub fn recently_unavailable(&self, now: i64) -> bool {
        match self.last_unavailable {
            Some(t) => now - t < UNAVAILABLE_TTL_SECS && self.last_success.is_none_or(|s| s < t),
            None => false,
        }
    }

    /// Whether a request of `request_chars` is expected to fit the observed context limit.
    pub fn fits(&self, request_chars: usize) -> bool {
        match self.context_limit_chars {
            Some(limit) => (request_chars as u64) < limit.saturating_mul(9) / 10,
            None => true,
        }
    }

    /// Observed context limit in (estimated) tokens.
    pub fn context_limit_tokens(&self) -> Option<u64> {
        self.context_limit_chars.map(|c| c / CHARS_PER_TOKEN_LIMIT)
    }

    /// Fold one observation in. `prompt_chars` is the request size when known; `limit_tokens`
    /// a context limit reported by the provider.
    pub fn apply(&mut self, outcome: Outcome, prompt_chars: Option<usize>, limit_tokens: Option<u64>, now: i64) {
        self.updated = now;
        match outcome {
            Outcome::Success { latency_ms } => {
                self.successes += 1;
                self.last_success = Some(now);
                let l = latency_ms as f64;
                self.latency_ms = Some(match self.latency_ms {
                    Some(prev) => prev * (1.0 - LATENCY_ALPHA) + l * LATENCY_ALPHA,
                    None => l,
                });
                self.latency_samples = self.latency_samples.saturating_add(1);
                if let Some(p) = prompt_chars {
                    let p = p as u64;
                    self.max_ok_prompt_chars = self.max_ok_prompt_chars.max(p);
                    // A larger prompt succeeded: the earlier overflow was not a hard limit.
                    if self.context_limit_chars.is_some_and(|lim| p >= lim) {
                        self.context_limit_chars = None;
                    }
                }
            }
            Outcome::MalformedToolCall => self.malformed_tool_calls += 1,
            Outcome::Unavailable => {
                self.unavailable += 1;
                self.last_unavailable = Some(now);
            }
            Outcome::ContextTooLong => {
                self.context_overflows += 1;
                let from_tokens = limit_tokens.map(|t| t.saturating_mul(CHARS_PER_TOKEN_LIMIT));
                let from_prompt = prompt_chars.map(|p| p as u64);
                if let Some(limit) = from_tokens.or(from_prompt) {
                    // Never below what already worked.
                    let limit = limit.max(self.max_ok_prompt_chars + 1);
                    self.context_limit_chars = Some(self.context_limit_chars.map_or(limit, |l| l.min(limit)));
                }
            }
            Outcome::Failed => self.failures += 1,
        }
        if self.successes + self.failures + self.malformed_tool_calls > DECAY_AT {
            self.successes /= 2;
            self.failures /= 2;
            self.malformed_tool_calls /= 2;
            self.unavailable /= 2;
            self.context_overflows /= 2;
        }
    }

    /// Compact view for the UI (`successPct`, `latencyMs`, `toolCallErrors`, …).
    pub fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "calls": self.calls(),
            "successes": self.successes,
            "failures": self.failures,
            "successPct": self.success_pct().map(|p| p.round()),
            "latencyMs": self.latency_ms.map(|l| l.round() as u64),
            "toolCallErrors": self.malformed_tool_calls,
            "unavailable": self.unavailable,
            "contextOverflows": self.context_overflows,
            "contextLimitTokens": self.context_limit_tokens(),
            "streams": self.streams,
        })
    }
}

/// Whether a provider message says the request did not fit the model's context window.
pub fn is_context_overflow(message: &str) -> bool {
    let m = message.to_lowercase();
    ["maximum context length", "context length", "context window", "too many tokens", "prompt is too long", "input is too long", "reduce the length", "exceeds the model's maximum", "input tokens exceed"]
        .iter()
        .any(|p| m.contains(p))
}

/// Context limit (tokens) a provider message reports, e.g. "maximum context length is 32768 tokens".
pub fn context_limit_from_message(message: &str) -> Option<u64> {
    let m = message.to_lowercase();
    for marker in ["context length", "context window", "maximum context", "max context"] {
        let Some(i) = m.find(marker) else { continue };
        let tail: String = m[i + marker.len()..].chars().take(40).collect();
        let mut num = String::new();
        for c in tail.chars() {
            if c.is_ascii_digit() {
                num.push(c);
            } else if c == ',' && !num.is_empty() {
                continue;
            } else if !num.is_empty() {
                break;
            }
        }
        if let Ok(n) = num.parse::<u64>() {
            if n >= 512 {
                return Some(n);
            }
        }
    }
    None
}

/// The model-specific outcome of a provider error (None: the error says nothing about the
/// model — missing key, auth, quota, network, rate limits, cancellation).
pub fn outcome_for_error(e: &ProviderError) -> Option<Outcome> {
    match e {
        ProviderError::ModelUnavailable { .. } => Some(Outcome::Unavailable),
        ProviderError::BadRequest { message, .. } if is_context_overflow(message) => Some(Outcome::ContextTooLong),
        ProviderError::BadRequest { .. } | ProviderError::Server { .. } | ProviderError::Timeout(_) | ProviderError::Malformed(_) => Some(Outcome::Failed),
        _ => None,
    }
}

/// In-memory copy of the persisted stats (loaded lazily).
#[derive(Default)]
pub(crate) struct StatsCache(Mutex<Option<HashMap<String, ModelStats>>>);

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Genius {
    fn with_stats<R>(&self, f: impl FnOnce(&mut HashMap<String, ModelStats>) -> R) -> R {
        let mut guard = self.model_stats.0.lock().unwrap();
        let map = guard.get_or_insert_with(|| self.db.get_setting(STATS_KEY).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default());
        f(map)
    }

    /// Record what happened when a model was used (see [`Outcome`]). Persisted immediately.
    pub fn record_model_outcome(&self, model: &str, outcome: Outcome) {
        self.record_model_observation(model, outcome, None, None);
    }

    /// [`record_model_outcome`](Self::record_model_outcome) with the request size and a
    /// provider-reported context limit (tokens), when known.
    pub fn record_model_observation(&self, model: &str, outcome: Outcome, prompt_chars: Option<usize>, limit_tokens: Option<u64>) {
        if model.trim().is_empty() {
            return;
        }
        let now = unix_now();
        let snapshot = self.with_stats(|map| {
            map.entry(model.to_string()).or_default().apply(outcome, prompt_chars, limit_tokens, now);
            if map.len() > MAX_MODELS {
                let mut by_age: Vec<(i64, String)> = map.iter().map(|(k, v)| (v.updated, k.clone())).collect();
                by_age.sort();
                for (_, k) in by_age.into_iter().take(map.len() - MAX_MODELS) {
                    map.remove(&k);
                }
            }
            serde_json::to_value(&*map).ok()
        });
        if let Some(v) = snapshot {
            if let Err(e) = self.db.set_setting(STATS_KEY, &v) {
                log::warn!("could not persist model stats: {e}");
            }
        }
    }

    /// A probe result as an observation: latency and streaming support.
    pub fn record_probe(&self, p: &ProbeResult) {
        if p.chat {
            self.with_stats(|map| map.entry(p.model.clone()).or_default().streams = Some(p.streaming));
            // persists both
            self.record_model_outcome(&p.model, Outcome::Success { latency_ms: p.latency_ms });
        }
    }

    /// Snapshot of every model's observed stats.
    pub fn model_stats(&self) -> HashMap<String, ModelStats> {
        self.with_stats(|map| map.clone())
    }

    /// Forget all observations (e.g. after switching provider).
    pub fn reset_model_stats(&self) {
        self.with_stats(|map| map.clear());
        let _ = self.db.set_setting(STATS_KEY, &serde_json::json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn counts_rates_and_latency() {
        let mut s = ModelStats::default();
        assert_eq!(s.success_pct(), None);
        assert_eq!(s.failure_rate(), 0.0);
        s.apply(Outcome::Success { latency_ms: 1000 }, Some(5000), None, 10);
        s.apply(Outcome::Success { latency_ms: 2000 }, None, None, 11);
        s.apply(Outcome::Failed, None, None, 12);
        s.apply(Outcome::MalformedToolCall, None, None, 13);
        assert_eq!((s.successes, s.failures, s.malformed_tool_calls), (2, 1, 1));
        assert_eq!(s.success_pct().unwrap().round(), 67.0);
        assert!((s.latency_ms.unwrap() - 1300.0).abs() < 1e-6, "EWMA 0.7*1000+0.3*2000");
        assert!((s.failure_rate() - 0.2).abs() < 1e-9);
        assert!((s.malformed_rate() - 0.25).abs() < 1e-9);
        assert_eq!(s.max_ok_prompt_chars, 5000);
        assert_eq!(s.updated, 13);
    }

    #[test]
    fn context_limit_is_observed_and_relaxed_by_larger_successes() {
        let mut s = ModelStats::default();
        s.apply(Outcome::Success { latency_ms: 10 }, Some(20_000), None, 1);
        s.apply(Outcome::ContextTooLong, Some(150_000), None, 2);
        assert_eq!(s.context_limit_chars, Some(150_000));
        assert!(s.fits(100_000) && !s.fits(140_000));
        s.apply(Outcome::ContextTooLong, Some(200_000), Some(32_768), 3);
        assert_eq!(s.context_limit_chars, Some(98_304), "provider-reported limit wins when smaller");
        assert_eq!(s.context_limit_tokens(), Some(32_768));
        s.apply(Outcome::Success { latency_ms: 10 }, Some(120_000), None, 4);
        assert_eq!(s.context_limit_chars, None, "a larger prompt worked: the limit was not real");
    }

    #[test]
    fn unavailability_expires_or_is_cleared_by_success() {
        let mut s = ModelStats::default();
        s.apply(Outcome::Unavailable, None, None, 1000);
        assert!(s.recently_unavailable(1000 + 60));
        assert!(!s.recently_unavailable(1000 + UNAVAILABLE_TTL_SECS + 1));
        s.apply(Outcome::Success { latency_ms: 5 }, None, None, 1100);
        assert!(!s.recently_unavailable(1200));
    }

    #[test]
    fn counts_decay_to_stay_bounded() {
        let mut s = ModelStats::default();
        for i in 0..1000 {
            s.apply(if i % 4 == 0 { Outcome::Failed } else { Outcome::Success { latency_ms: 100 } }, None, None, i);
        }
        assert!(s.calls() <= DECAY_AT + 1, "{}", s.calls());
        assert!(s.failures > 0 && s.successes > s.failures);
    }

    #[test]
    fn classifies_provider_errors() {
        let unavailable = ProviderError::ModelUnavailable { model: "x".into(), status: 404, message: String::new() };
        assert_eq!(outcome_for_error(&unavailable), Some(Outcome::Unavailable));
        let ctx = ProviderError::BadRequest { status: 400, message: "This model's maximum context length is 32768 tokens. However, you requested 40000 tokens".into() };
        assert_eq!(outcome_for_error(&ctx), Some(Outcome::ContextTooLong));
        assert_eq!(context_limit_from_message("This model's maximum context length is 32768 tokens"), Some(32768));
        assert_eq!(context_limit_from_message("context window of 128,000 tokens exceeded"), Some(128_000));
        assert_eq!(context_limit_from_message("bad thing"), None);
        assert_eq!(outcome_for_error(&ProviderError::Server { status: 500, message: "boom".into() }), Some(Outcome::Failed));
        assert_eq!(outcome_for_error(&ProviderError::Timeout("x".into())), Some(Outcome::Failed));
        assert_eq!(outcome_for_error(&ProviderError::RateLimited { retry_after_secs: None, message: String::new() }), None);
        assert_eq!(outcome_for_error(&ProviderError::Auth { status: 401, message: String::new() }), None);
        assert_eq!(outcome_for_error(&ProviderError::Cancelled), None);
    }

    #[tokio::test]
    async fn persists_across_restarts_and_is_bounded() {
        let tmp = tempfile::tempdir().unwrap();
        let mk = || Genius::new(&tmp.path().join("data"), Arc::new(crate::events::NullEmitter), Arc::new(crate::secrets::MemoryStore::default())).unwrap();
        {
            let g = mk();
            g.record_model_outcome("a/model", Outcome::Success { latency_ms: 800 });
            g.record_model_outcome("a/model", Outcome::MalformedToolCall);
            g.record_model_observation("b/model", Outcome::ContextTooLong, Some(90_000), None);
            g.record_probe(&ProbeResult { model: "c/model".into(), chat: true, streaming: false, latency_ms: 321, sample: "ready".into() });
            g.record_model_outcome("", Outcome::Failed);
        }
        let g = mk();
        let s = g.model_stats();
        assert_eq!(s.len(), 3);
        assert_eq!(s["a/model"].successes, 1);
        assert_eq!(s["a/model"].malformed_tool_calls, 1);
        assert_eq!(s["a/model"].latency_ms, Some(800.0));
        assert_eq!(s["b/model"].context_limit_chars, Some(90_000));
        assert_eq!(s["c/model"].latency_ms, Some(321.0));
        assert_eq!(s["c/model"].streams, Some(false));
        assert_eq!(s["a/model"].view()["toolCallErrors"], 1);
        for i in 0..(MAX_MODELS + 20) {
            g.record_model_outcome(&format!("m/{i}"), Outcome::Failed);
        }
        assert_eq!(g.model_stats().len(), MAX_MODELS);
        g.reset_model_stats();
        assert!(g.model_stats().is_empty());
    }
}
