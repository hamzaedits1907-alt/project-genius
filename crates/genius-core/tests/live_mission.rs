#![allow(clippy::collapsible_if)]
//! Opt-in LIVE mission test: a real mission (planning, task graph, isolated workspace, real model,
//! real tools, integration as a changeset) against the real NVIDIA API.
//!
//! Skipped unless `NVIDIA_API_KEY` is set. Run with:
//!   NVIDIA_API_KEY=nvapi-… cargo test -p genius-core --test live_mission -- --nocapture
//!
//! What it proves: with a real model, Agent Mode creates a plan, runs a task, performs a real
//! allowed file operation in an isolated copy, integrates it into the project and reports an
//! honest outcome. It does NOT prove anything about other providers or models.

use genius_core::app::Genius;
use genius_core::events::NullEmitter;
use genius_core::secrets::MemoryStore;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_mission_creates_a_real_file_and_reports_honestly() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live mission test");
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

    let p = g.create_project("Mission sandbox", "scratch").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", r#"{"name":"sandbox","scripts":{}}"#).unwrap();
    ws.write_text("src/alpha.txt", "alpha\n").unwrap();
    ws.write_text("src/beta.txt", "beta\n").unwrap();

    let started = Instant::now();
    let m = g.create_mission(&p.id, "Create the file docs/inventory.md. It must start with the line '# Inventory' and then list every file inside the src folder, one per line, as a Markdown bullet.", None).unwrap();
    let mut m = wait(&g, &m.id, &["ready", "failed"], 180).await;
    println!("live mission: planned in {:?}: status={} tasks={}", started.elapsed(), m.status, m.tasks.len());
    for t in &m.tasks {
        println!("  plan task: {} [{}] assignee={:?} deps={:?}", t.title, t.status, t.assignee_agent_id, t.depends_on);
    }
    assert_eq!(m.status, "ready", "planning must produce a startable mission: {:#?}", m.meta);
    g.start_mission(&m.id).unwrap();
    // The default agent approval policy asks before changes land: approve like the user would in the UI.
    let mut approvals = 0;
    loop {
        m = wait_or_approval(&g, &m.id, 420).await;
        let waiting: Vec<_> = m.tasks.iter().filter(|t| t.status == "awaiting_approval").map(|t| t.id.clone()).collect();
        if waiting.is_empty() {
            break;
        }
        for id in waiting {
            println!("live mission: task {id} is waiting for approval (changes: {:?}); approving", m.tasks.iter().find(|t| t.id == id).map(|t| t.changed_paths.clone()));
            g.approve_mission_task(&id, true).unwrap();
            approvals += 1;
        }
        assert!(approvals < 6, "approval loop");
    }
    println!("live mission: finished in {:?}: status={} summary={:?}", started.elapsed(), m.status, m.summary);
    for t in &m.tasks {
        println!("  task: {} [{}] verification={:?} blocker={:?} usage={:?}
    result: {:?}", t.title, t.status, t.verification, t.blocker, t.usage, t.result.as_deref().map(|r| r.chars().take(300).collect::<String>()));
    }
    let content = ws.read_text("docs/inventory.md");
    println!("live mission: docs/inventory.md = {content:?}");
    assert_eq!(m.status, "completed", "{m:#?}");
    let content = content.expect("the mission must have created docs/inventory.md in the real project");
    assert!(content.contains("# Inventory") && content.contains("alpha.txt") && content.contains("beta.txt"), "{content}");
    assert!(m.tasks.iter().all(|t| t.workspace.is_none()), "isolated workspaces are removed");
}

async fn wait(g: &Arc<Genius>, id: &str, want: &[&str], secs: u64) -> genius_core::missions::Mission {
    for _ in 0..(secs * 10) {
        let m = g.get_mission(id).unwrap();
        if want.contains(&m.status.as_str()) {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("mission did not reach {want:?}: {:#?}", g.get_mission(id).unwrap());
}

/// Until the mission is over or a task needs the user's decision.
async fn wait_or_approval(g: &Arc<Genius>, id: &str, secs: u64) -> genius_core::missions::Mission {
    for _ in 0..(secs * 10) {
        let m = g.get_mission(id).unwrap();
        if ["completed", "failed", "cancelled"].contains(&m.status.as_str()) || m.tasks.iter().any(|t| t.status == "awaiting_approval") {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("mission did not finish: {:#?}", g.get_mission(id).unwrap());
}
