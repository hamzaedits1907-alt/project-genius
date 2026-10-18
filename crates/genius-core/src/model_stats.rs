//! Observed per-model behaviour (stub; full implementation lands with the router work).
use crate::app::Genius;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome { Success { latency_ms: u64 }, MalformedToolCall, Unavailable, ContextTooLong, Failed }
impl Genius {
    pub fn record_model_outcome(&self, _model: &str, _outcome: Outcome) {}
}
