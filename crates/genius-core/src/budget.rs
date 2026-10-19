//! Execution budgets for Genius's **own** work: how many model calls, tool calls, minutes and
//! tasks one request may use before Genius stops and says so.
//!
//! Genius is bring-your-own-key: the user's provider bills them directly and Genius neither sees
//! nor controls that bill. These limits only keep a runaway loop from burning calls. Token counts
//! come from the provider's `usage` when it sends one (**measured**); when it does not, a
//! characters ÷ 4 guess is kept apart (**estimated**) and never presented as an exact count.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskLimits {
    /// Model requests one task may make (0 = no limit).
    pub max_model_calls: u32,
    /// Tool calls one task may make (0 = no limit).
    pub max_tool_calls: u32,
    /// Wall-clock minutes one task may run (0 = no limit).
    pub max_minutes: u32,
    /// Tokens one task may use, measured plus estimated (0 = no limit). Local guard only.
    pub max_tokens: u64,
    /// Tasks that may run at the same time across all projects.
    pub max_concurrent_tasks: u32,
    /// Provider retries (rate limit / network / 502–504) before a call gives up.
    pub max_provider_retries: u32,
}

impl Default for TaskLimits {
    fn default() -> Self {
        Self { max_model_calls: 100, max_tool_calls: 150, max_minutes: 45, max_tokens: 0, max_concurrent_tasks: 6, max_provider_retries: 3 }
    }
}

impl TaskLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_model_calls > 1000 || self.max_tool_calls > 5000 || self.max_minutes > 24 * 60 {
            return Err("budget limits are out of range (model calls ≤ 1000, tool calls ≤ 5000, minutes ≤ 1440)".into());
        }
        if !(1..=32).contains(&self.max_concurrent_tasks) {
            return Err("max concurrent tasks must be between 1 and 32".into());
        }
        if self.max_provider_retries > 10 {
            return Err("provider retries must be 10 or fewer".into());
        }
        Ok(())
    }

    /// The first limit that is used up, if any. Pure: callers pass what they measured.
    pub fn check(&self, used: &Usage) -> Option<BudgetStop> {
        let stop = |limit: &'static str, used: u64, max: u64| Some(BudgetStop { limit, used, max });
        if self.max_model_calls > 0 && used.model_calls >= self.max_model_calls as u64 {
            return stop("maxModelCalls", used.model_calls, self.max_model_calls as u64);
        }
        if self.max_tool_calls > 0 && used.tool_calls >= self.max_tool_calls as u64 {
            return stop("maxToolCalls", used.tool_calls, self.max_tool_calls as u64);
        }
        if self.max_minutes > 0 && used.seconds >= self.max_minutes as u64 * 60 {
            return stop("maxMinutes", used.seconds / 60, self.max_minutes as u64);
        }
        let tokens = used.measured_tokens + used.estimated_tokens;
        if self.max_tokens > 0 && tokens >= self.max_tokens {
            return stop("maxTokens", tokens, self.max_tokens);
        }
        None
    }
}

/// What one task has consumed so far.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub model_calls: u64,
    pub tool_calls: u64,
    pub seconds: u64,
    /// Tokens the provider reported.
    pub measured_tokens: u64,
    /// Tokens Genius guessed for requests whose provider reported none.
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetStop {
    pub limit: &'static str,
    pub used: u64,
    pub max: u64,
}

impl BudgetStop {
    pub fn message(&self) -> String {
        let unit = match self.limit {
            "maxMinutes" => " minutes",
            "maxTokens" => " tokens (measured + estimated)",
            _ => "",
        };
        format!("{} ({}{unit}) reached after {}", self.limit, self.max, self.used)
    }
}

/// Rough token estimate for text the provider did not count: ⌈chars ÷ 4⌉. Always an estimate.
pub fn estimate_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(4)
}

/// How trustworthy the token figures of a task are.
pub fn usage_source(calls: u64, calls_with_usage: u64) -> &'static str {
    match (calls, calls_with_usage) {
        (0, _) => "none",
        (c, u) if u >= c => "measured",
        (_, 0) => "estimated",
        _ => "mixed",
    }
}

/// The report block shown to the user: measured and estimated figures never merged.
pub fn usage_report(prompt: u64, completion: u64, total: u64, estimated: u64, calls: u64, calls_with_usage: u64, tool_calls: u64, limits: &TaskLimits) -> Value {
    json!({
        "promptTokens": prompt, "completionTokens": completion, "totalTokens": total,
        "estimatedTokens": estimated, "source": usage_source(calls, calls_with_usage),
        "modelCalls": calls, "toolCalls": tool_calls,
        "limits": limits,
        "note": "Tokens are what the provider reported; estimates (characters ÷ 4) are listed separately. Your provider's own dashboard is the authority for billing.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(m: u64, t: u64, s: u64, meas: u64, est: u64) -> Usage {
        Usage { model_calls: m, tool_calls: t, seconds: s, measured_tokens: meas, estimated_tokens: est }
    }

    #[test]
    fn each_limit_stops_on_its_own_and_zero_means_unlimited() {
        let l = TaskLimits { max_model_calls: 3, max_tool_calls: 5, max_minutes: 2, max_tokens: 1000, ..Default::default() };
        assert_eq!(l.check(&used(2, 4, 119, 400, 100)), None);
        assert_eq!(l.check(&used(3, 0, 0, 0, 0)).unwrap().limit, "maxModelCalls");
        assert_eq!(l.check(&used(0, 5, 0, 0, 0)).unwrap().limit, "maxToolCalls");
        assert_eq!(l.check(&used(0, 0, 120, 0, 0)).unwrap().limit, "maxMinutes");
        let t = l.check(&used(0, 0, 0, 600, 400)).unwrap();
        assert_eq!((t.limit, t.used), ("maxTokens", 1000), "measured and estimated both count toward the local guard");
        let off = TaskLimits { max_model_calls: 0, max_tool_calls: 0, max_minutes: 0, max_tokens: 0, ..Default::default() };
        assert_eq!(off.check(&used(9999, 9999, 999_999, 9_999_999, 0)), None);
    }

    #[test]
    fn validation_rejects_nonsense() {
        assert!(TaskLimits::default().validate().is_ok());
        assert!(TaskLimits { max_concurrent_tasks: 0, ..Default::default() }.validate().is_err());
        assert!(TaskLimits { max_model_calls: 100_000, ..Default::default() }.validate().is_err());
        assert!(TaskLimits { max_provider_retries: 99, ..Default::default() }.validate().is_err());
    }

    #[test]
    fn estimates_are_labelled_never_exact() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(9), 3);
        assert_eq!(usage_source(0, 0), "none");
        assert_eq!(usage_source(3, 3), "measured");
        assert_eq!(usage_source(3, 0), "estimated");
        assert_eq!(usage_source(3, 1), "mixed");
        let r = usage_report(10, 5, 15, 0, 1, 1, 2, &TaskLimits::default());
        assert_eq!(r["source"], "measured");
        assert_eq!(r["estimatedTokens"], 0);
    }

    #[test]
    fn old_saved_settings_without_limits_get_defaults() {
        let l: TaskLimits = serde_json::from_value(json!({ "maxToolCalls": 7 })).unwrap();
        assert_eq!((l.max_tool_calls, l.max_model_calls), (7, 100));
    }
}
