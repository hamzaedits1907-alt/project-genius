#![allow(clippy::collapsible_if)]
//! Opt-in LIVE Code Mode test: a real model builds a real multi-file app in a real project folder,
//! then the real `npm` type check and production build decide whether it works.
//!
//! Skipped unless `NVIDIA_API_KEY` is set. Run with:
//!   NVIDIA_API_KEY=nvapi-… cargo test -p genius-core --test live_code -- --nocapture
//!
//! It exists because a green mocked test once let "drafts that never build" through: the failure only
//! shows with a real model producing a real, large answer.

use genius_core::app::Genius;
use genius_core::events::NullEmitter;
use genius_core::process::Operation;
use genius_core::secrets::MemoryStore;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const PROMPT: &str = "Build a personal finance tracker web app. I want to add income and expenses with a category and date, see my balance, a list I can search/filter/delete from, a spending-by-category chart (plain SVG or CSS, no chart library), and my data saved in the browser. Make it look modern and polished.";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_code_mode_builds_an_app_that_type_checks_and_builds() {
    let Ok(key) = std::env::var("NVIDIA_API_KEY") else {
        eprintln!("SKIPPED: set NVIDIA_API_KEY to run the live Code Mode test");
        return;
    };
    if genius_core::process::find_executable("npm").is_none() {
        eprintln!("SKIPPED: npm is not installed");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.model = None;
    s.model_mode = "auto".into();
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    s.approvals.dependency_install = genius_core::settings::Policy::Auto; // like the "auto" setting in the app
    s.approvals.apply_ai_edits = genius_core::settings::Policy::Auto;
    g.save_settings(&s).unwrap();
    g.save_api_key(&key).unwrap();

    let p = g.create_project("Personal Finance Tracker", PROMPT).unwrap();
    let cancel = CancellationToken::new();
    let ws = g.workspace(&p.id).unwrap();
    let started = Instant::now();
    // the same two steps the UI takes for a new project: plan, then generate
    g.start_plan(&p.id, PROMPT, "").unwrap();
    wait_idle(&g, &p.id, started, 600).await;
    let planned = g.db.get_project(&p.id).unwrap();
    let spec = planned.spec.clone().expect("the planning step must produce a plan");
    let nfiles = spec["files"].as_array().map(|a| a.len()).unwrap_or(0);
    println!("live code: planned in {:?}: {} planned files", started.elapsed(), nfiles);
    assert!(nfiles <= 30, "the plan must stay buildable");
    let gen_started = Instant::now();
    g.start_generate(&p.id).unwrap();
    wait_idle(&g, &p.id, gen_started, 1500).await;
    println!("live code: generated in {:?}", gen_started.elapsed());
    for e in g.bus.list(Some(&p.id), 400).unwrap().into_iter().rev().take(25).collect::<Vec<_>>().into_iter().rev() {
        println!("  event [{:?}] {}", e.status, e.title);
    }
    let files = ws.list_files().unwrap_or_default().into_iter().map(|e| e.path).collect::<Vec<_>>();
    println!("live code: {} files: {:?}", files.len(), files);
    let app = ws.read_text_opt("src/App.tsx").unwrap().unwrap_or_default();
    assert!(!genius_core::scaffold::is_template_app(&app), "src/App.tsx is still the starter page");

    let r = g.run_operation(&p.id, None, Operation::Install, "live test", true, &cancel).await.unwrap();
    println!("install ok={}", r.ok());
    let ty = g.run_operation(&p.id, None, Operation::Typecheck, "live test", true, &cancel).await.unwrap();
    println!("typecheck ok={} tail={}", ty.ok(), ty.output_tail.chars().rev().take(1500).collect::<String>().chars().rev().collect::<String>());
    let b = g.run_operation(&p.id, None, Operation::Build, "live test", true, &cancel).await.unwrap();
    println!("build ok={}", b.ok());
    assert!(ty.ok() && b.ok(), "the generated app must type-check and build");
}

async fn wait_idle(g: &Arc<Genius>, pid: &str, since: Instant, secs: u64) {
    let mut seen = std::collections::HashSet::new();
    loop {
        // show progress as it happens: every model call and its duration
        for e in g.bus.list(Some(pid), 400).unwrap_or_default().into_iter().rev() {
            if e.title.contains('(') && seen.insert(format!("{}{:?}", e.id, e.status)) {
                println!("  [{:>5.0}s] {:?} {} {}", since.elapsed().as_secs_f32(), e.status, e.title, e.detail.get("totalMs").map(|v| format!("totalMs={v} out={}", e.detail.get("outputChars").unwrap_or(&serde_json::Value::Null))).unwrap_or_default());
            }
        }
        if g.active_tasks(Some(pid)).is_empty() {
            return;
        }
        assert!(since.elapsed() < Duration::from_secs(secs), "did not finish in {secs} s");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
