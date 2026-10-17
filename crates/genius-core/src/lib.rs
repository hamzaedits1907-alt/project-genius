//! Project Genius core library. Platform-independent; the Tauri shell is a thin
//! command layer over [`app::Genius`].

pub mod agent;
pub mod app;
pub mod attachments;
pub mod capabilities;
pub mod approvals;
pub mod blocks;
pub mod changes;
pub mod context;
pub mod db;
pub mod diagnostics;
pub mod error;
pub mod events;
pub mod export;
pub mod git;
pub mod github;
pub mod inspector;
pub mod integrations;
pub mod nvidia;
pub mod process;
pub mod redact;
pub mod router;
pub mod scaffold;
pub mod secrets;
pub mod secretscan;
pub mod settings;
pub mod spec;
pub mod supabase;
pub mod uilog;
pub mod updater;
pub mod project_context;
pub mod prompts;
pub mod vcs;
pub mod web;
pub mod workspace;

pub use error::{GeniusError, Result};
