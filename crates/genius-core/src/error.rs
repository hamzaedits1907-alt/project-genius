//! Unified error type. Every error crossing the IPC boundary is serialized as
//! `{ kind, message }` so the UI can render a specific, actionable state instead
//! of a raw stack trace. Messages are always passed through redaction first.

use serde::Serialize;

use crate::redact::redact;

#[derive(Debug, thiserror::Error)]
pub enum GeniusError {
    #[error("{0}")]
    Validation(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("path rejected: {0}")]
    PathRejected(String),
    #[error("{0}")]
    Conflict(String),
    #[error("cancelled")]
    Cancelled,
    #[error("approval denied: {0}")]
    ApprovalDenied(String),
    #[error("{0}")]
    MissingTool(String),
    #[error("{0}")]
    Provider(#[from] crate::nvidia::ProviderError),
    #[error("{0}")]
    Git(String),
    #[error("{0}")]
    GitHub(String),
    #[error("credential storage error: {0}")]
    Secret(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Internal(String),
}

impl GeniusError {
    pub fn kind(&self) -> &'static str {
        match self {
            GeniusError::Validation(_) => "validation",
            GeniusError::NotFound(_) => "not_found",
            GeniusError::PathRejected(_) => "path_rejected",
            GeniusError::Conflict(_) => "conflict",
            GeniusError::Cancelled => "cancelled",
            GeniusError::ApprovalDenied(_) => "approval_denied",
            GeniusError::MissingTool(_) => "missing_tool",
            GeniusError::Provider(e) => e.kind(),
            GeniusError::Git(_) => "git",
            GeniusError::GitHub(_) => "github",
            GeniusError::Secret(_) => "secret",
            GeniusError::Process(_) => "process",
            GeniusError::Db(_) => "database",
            GeniusError::Io(_) => "io",
            GeniusError::Json(_) => "json",
            GeniusError::Internal(_) => "internal",
        }
    }
}

impl Serialize for GeniusError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("GeniusError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &redact(&self.to_string()))?;
        st.end()
    }
}

pub type Result<T> = std::result::Result<T, GeniusError>;

pub fn validation(msg: impl Into<String>) -> GeniusError {
    GeniusError::Validation(msg.into())
}
