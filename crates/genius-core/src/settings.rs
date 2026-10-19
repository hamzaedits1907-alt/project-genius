//! Non-secret application settings, stored as one JSON document in SQLite.

use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::error::{GeniusError, Result};
use crate::nvidia::{normalize_base_url, DEFAULT_BASE_URL};

const KEY: &str = "app_settings";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    Ask,
    Auto,
}

/// How hard the model should work: trades speed for quality (output budget, reasoning, model choice).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Fast,
    Balanced,
    #[default]
    Max,
}

impl Effort {
    pub fn as_str(&self) -> &'static str {
        match self {
            Effort::Fast => "fast",
            Effort::Balanced => "balanced",
            Effort::Max => "max",
        }
    }

    /// "fast" | "balanced" | "max" (and aliases); None for anything else, including "auto".
    pub fn parse(s: &str) -> Option<Effort> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "fast" | "low" | "quick" => Effort::Fast,
            "balanced" | "medium" | "normal" => Effort::Balanced,
            "max" | "high" | "maximum" => Effort::Max,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalPolicies {
    /// Apply AI-proposed edits to existing files without review.
    pub apply_ai_edits: Policy,
    /// Run `npm install` (executes package install scripts).
    pub dependency_install: Policy,
    /// Create commits proposed by Genius.
    pub git_commit: Policy,
    /// Push to remotes. Always asks unless explicitly switched to auto.
    pub git_push: Policy,
}

impl Default for ApprovalPolicies {
    fn default() -> Self {
        Self { apply_ai_edits: Policy::Ask, dependency_install: Policy::Ask, git_commit: Policy::Ask, git_push: Policy::Ask }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub base_url: String,
    pub model: Option<String>,
    /// "auto": Genius picks the best available model per job; "manual": always `model`.
    pub model_mode: String,
    pub effort: Effort,
    /// Requests that don't name an effort get one from their complexity (simple → fast,
    /// normal → balanced, complex → max), never above `effort`.
    pub auto_effort: bool,
    /// Let reasoning models think before answering (reasoning is streamed to the UI).
    pub thinking: bool,
    pub temperature: f32,
    pub max_tokens: u32,
    pub context_budget_chars: usize,
    pub workspace_dir: Option<String>,
    pub approvals: ApprovalPolicies,
    /// After AI changes are applied, run the build automatically to verify them.
    pub verify_after_apply: bool,
    /// Maximum automatic Build Doctor attempts per failure before stopping.
    pub max_repair_attempts: u32,
    pub theme: String,
    pub onboarding_done: bool,
    /// Interface density: "simple" (guided) or "advanced" (all tools visible).
    pub ui_mode: String,
    /// Check GitHub Releases for a newer version on startup.
    pub auto_check_updates: bool,
    /// Allow the assistant to use internet/web tools.
    pub internet_access: bool,
    /// Display name of the assistant.
    pub assistant_name: String,
    /// Which provider API the saved key belongs to: "nvidia" | "openai" | "anthropic" | "gemini" |
    /// "custom" (any OpenAI-compatible endpoint at `base_url`). See `provider.rs`.
    pub provider: String,
    /// Limits on Genius's own work per task (calls, minutes, concurrency, retries).
    pub limits: crate::budget::TaskLimits,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            model: None,
            model_mode: "auto".into(),
            effort: Effort::Max,
            auto_effort: true,
            thinking: true,
            temperature: 0.2,
            max_tokens: 16384,
            context_budget_chars: 60_000,
            workspace_dir: None,
            approvals: ApprovalPolicies::default(),
            verify_after_apply: true,
            max_repair_attempts: 3,
            theme: "dark".into(),
            onboarding_done: false,
            ui_mode: "simple".into(),
            auto_check_updates: true,
            internet_access: true,
            assistant_name: "Genius".into(),
            provider: "nvidia".into(),
            limits: crate::budget::TaskLimits::default(),
        }
    }
}

impl Settings {
    pub fn load(db: &Db) -> Result<Self> {
        Ok(match db.get_setting(KEY)? {
            Some(v) => serde_json::from_value(v).unwrap_or_default(),
            None => Self::default(),
        })
    }

    pub fn save(&self, db: &Db) -> Result<()> {
        self.validate()?;
        db.set_setting(KEY, &serde_json::to_value(self)?)
    }

    /// Auto model routing (anything but an explicit "manual").
    pub fn is_auto(&self) -> bool {
        self.model_mode != "manual"
    }

    pub fn validate(&self) -> Result<()> {
        normalize_base_url(&self.base_url).map_err(GeniusError::Validation)?;
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(GeniusError::Validation("temperature must be between 0 and 2".into()));
        }
        if !(256..=131_072).contains(&self.max_tokens) {
            return Err(GeniusError::Validation("max tokens must be between 256 and 131072".into()));
        }
        if !["auto", "manual"].contains(&self.model_mode.as_str()) {
            return Err(GeniusError::Validation("model mode must be \"auto\" or \"manual\"".into()));
        }
        if !(4_000..=400_000).contains(&self.context_budget_chars) {
            return Err(GeniusError::Validation("context budget must be between 4,000 and 400,000 characters".into()));
        }
        if self.max_repair_attempts > 5 {
            return Err(GeniusError::Validation("at most 5 automatic repair attempts are allowed".into()));
        }
        if !["dark", "light", "system"].contains(&self.theme.as_str()) {
            return Err(GeniusError::Validation("unknown theme".into()));
        }
        if !["simple", "advanced"].contains(&self.ui_mode.as_str()) {
            return Err(GeniusError::Validation("ui mode must be \"simple\" or \"advanced\"".into()));
        }
        if !crate::provider::ProviderKind::ALL.iter().any(|k| k.id() == self.provider) {
            return Err(GeniusError::Validation("unknown provider".into()));
        }
        self.limits.validate().map_err(GeniusError::Validation)?;
        let name_len = self.assistant_name.trim().chars().count();
        if !(1..=40).contains(&name_len) {
            return Err(GeniusError::Validation("assistant name must be 1 to 40 characters".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe_and_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        let s = Settings::load(&db).unwrap();
        assert_eq!(s.approvals.git_push, Policy::Ask);
        assert_eq!(s.approvals.dependency_install, Policy::Ask);
        assert_eq!(s.base_url, DEFAULT_BASE_URL);
        let mut s2 = s.clone();
        s2.model = Some("meta/llama-3.3-70b-instruct".into());
        s2.save(&db).unwrap();
        assert_eq!(Settings::load(&db).unwrap(), s2);
    }

    #[test]
    fn rejects_invalid() {
        let db = Db::open_in_memory().unwrap();
        let s = Settings { base_url: "http://example.com".into(), ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { max_tokens: 10, ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { ui_mode: "expert".into(), ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { assistant_name: "   ".into(), ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { assistant_name: "x".repeat(41), ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { model_mode: "smart".into(), ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { max_tokens: 131_073, ..Default::default() };
        assert!(s.save(&db).is_err());
        let s = Settings { ui_mode: "advanced".into(), assistant_name: "Nova".into(), max_tokens: 131_072, model_mode: "manual".into(), ..Default::default() };
        assert!(s.save(&db).is_ok());
    }

    #[test]
    fn new_fields_have_defaults() {
        let s = Settings::default();
        assert_eq!(s.ui_mode, "simple");
        assert!(s.auto_check_updates);
        assert!(s.internet_access);
        assert_eq!(s.assistant_name, "Genius");
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["uiMode"], "simple");
        assert_eq!(v["autoCheckUpdates"], true);
        assert_eq!(v["internetAccess"], true);
        assert_eq!(v["assistantName"], "Genius");
        assert_eq!(v["modelMode"], "auto");
        assert_eq!(v["effort"], "max");
        assert_eq!(v["thinking"], true);
        assert_eq!(v["autoEffort"], true);
        assert_eq!(Effort::parse(" High "), Some(Effort::Max));
        assert_eq!(Effort::parse("auto"), None);
        let e: Effort = serde_json::from_value(serde_json::json!("balanced")).unwrap();
        assert_eq!(e, Effort::Balanced);
    }

    #[test]
    fn tolerates_partial_documents() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting(KEY, &serde_json::json!({"model": "x/y"})).unwrap();
        let s = Settings::load(&db).unwrap();
        assert_eq!(s.model.as_deref(), Some("x/y"));
        assert_eq!(s.max_tokens, 16384);
        assert_eq!(s.ui_mode, "simple");
        assert_eq!(s.model_mode, "auto", "existing settings without the field switch to auto");
        assert_eq!(s.effort, Effort::Max);
        assert!(s.thinking);
        assert!(s.internet_access);
    }
}
