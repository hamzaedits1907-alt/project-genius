//! LIVE test against the real NVIDIA API. Skipped unless `NVIDIA_API_KEY` is set, so it never
//! runs in CI or offline. Unlike every other test, NOTHING here is mocked: real model listing,
//! real Auto routing over the models your key can use, and a real agent run that must use
//! tools on a real project folder and return a final answer.
//!
//!   NVIDIA_API_KEY=nvapi-… cargo test -p genius-core --test live_nvidia -- --nocapture

use genius_core::agent::{AgentMode, AgentRequest};
use genius_core::app::Genius;
use genius_core::events::{EventKind, NullEmitter};
use genius_core::router::ModelRole;
use genius_core::secrets::MemoryStore;
use genius_core::settings::Effort;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_auto_routing_and_agent_run() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live NVIDIA test");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.model = None; // beginners never pick a model: Auto must work without one
    s.model_mode = "auto".into();
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key(&key).unwrap();

    let models = g.list_models(true).await.expect("real /models listing");
    println!("live: {} models available to this key", models.models.len());
    assert!(!models.models.is_empty());
    for effort in [Effort::Fast, Effort::Max] {
        let chain = g.resolve_models(ModelRole::Agent, effort).await.unwrap();
        println!("live: Auto agent chain for {effort:?}: {chain:?}");
        assert!(!chain.is_empty(), "Auto finds a usable model");
        assert!(chain.iter().all(|m| models.models.iter().any(|x| &x.id == m)), "only models the key can use");
    }

    let p = g.create_project("Live", "Live test").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("src/App.tsx", "export default function App() {\n  return <h1>Hello live test</h1>;\n}\n").unwrap();
    let conv = g.db.create_conversation(&p.id, "Live").unwrap();
    let t = g
        .start_agent(&p.id, AgentRequest { conversation_id: conv.id.clone(), message: "Read src/App.tsx and tell me exactly what the h1 says. Do not change anything.".into(), mode: AgentMode::Plan, ..Default::default() })
        .expect("agent starts without a manually selected model");
    for _ in 0..1200 {
        if g.active_tasks(Some(&p.id)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let steps = g.agent_steps(&t.id).unwrap();
    println!("live: {} tool steps: {:?}", steps.len(), steps.iter().map(|s| format!("{}:{}", s.tool, s.ok)).collect::<Vec<_>>());
    let msgs = g.db.list_conversation_messages(&conv.id, 50).unwrap();
    let answer = msgs.iter().rev().find(|m| m.role == "assistant").map(|m| m.content.clone()).unwrap_or_default();
    println!("live: answer: {answer}");
    let failed = g.bus.list(Some(&p.id), 500).unwrap().into_iter().any(|e| e.kind == EventKind::Task && e.task_id.as_deref() == Some(t.id.as_str()) && format!("{:?}", e.status) == "Failed");
    assert!(!failed, "the live agent task did not fail");
    assert!(steps.iter().any(|s| s.tool == "read_file" && s.ok), "the real model used the read_file tool");
    assert!(answer.contains("Hello live test"), "the answer is grounded in the real file");
}
