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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            model: None,
            temperature: 0.2,
            max_tokens: 8192,
            context_budget_chars: 60_000,
            workspace_dir: None,
            approvals: ApprovalPolicies::default(),
            verify_after_apply: true,
            max_repair_attempts: 3,
            theme: "dark".into(),
            onboarding_done: false,
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

    pub fn validate(&self) -> Result<()> {
        normalize_base_url(&self.base_url).map_err(GeniusError::Validation)?;
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(GeniusError::Validation("temperature must be between 0 and 2".into()));
        }
        if !(256..=65536).contains(&self.max_tokens) {
            return Err(GeniusError::Validation("max tokens must be between 256 and 65536".into()));
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
    }

    #[test]
    fn tolerates_partial_documents() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting(KEY, &serde_json::json!({"model": "x/y"})).unwrap();
        let s = Settings::load(&db).unwrap();
        assert_eq!(s.model.as_deref(), Some("x/y"));
        assert_eq!(s.max_tokens, 8192);
    }
}
