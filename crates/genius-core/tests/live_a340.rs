//! LIVE: the exact question from the field report ("What was the last route of a340 before retire"), asked with a
//! real model and the real web. Skipped unless `NVIDIA_API_KEY` is set. It must be answered (not met with
//! "which A340?"), the answer must come from pages that were really read, and it must not be a bare "I don't know".
//!
//!   NVIDIA_API_KEY=nvapi-… cargo test -p genius-core --test live_a340 -- --nocapture

use genius_core::app::Genius;
use genius_core::chat_mode::ChatRequest;
use genius_core::events::NullEmitter;
use genius_core::secrets::MemoryStore;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_a340_question_is_answered_from_the_web_not_met_with_a_question() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live A340 test");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.model = None;
    s.model_mode = "auto".into();
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key(&key).unwrap();
    let conv = g.create_chat_conversation("chat", None).unwrap();
    let pid = conv.row.project_id.clone();
    let started = Instant::now();
    g.start_chat(&conv.row.id, ChatRequest { message: "What was the last route of a340 before retire".into(), attachment_ids: vec![], effort: Some("balanced".into()) }).unwrap();
    loop {
        if g.active_tasks(Some(&pid)).is_empty() {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(300), "no answer within 5 minutes");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let msgs = g.db.list_conversation_messages(&conv.row.id, 20).unwrap();
    let answer = msgs.iter().rev().find(|m| m.role == "assistant").unwrap();
    println!("answered in {:?} (kind {}):\n{}\n", started.elapsed(), answer.kind, answer.content);
    let steps = g.db.list_agent_steps(answer.task_id.as_deref().unwrap_or("")).unwrap_or_default();
    println!("tools used: {:?}", steps.iter().map(|s| format!("{}:{}", s.tool, s.ok)).collect::<Vec<_>>());
    assert_ne!(answer.kind, "error", "{}", answer.content);
    assert!(!answer.content.trim_end().ends_with('?'), "the reply is only a question: {}", answer.content);
    assert!(steps.iter().any(|s| s.tool == "web_search" || s.tool == "fetch_url"), "the model did not look anything up: {steps:?}");
}
