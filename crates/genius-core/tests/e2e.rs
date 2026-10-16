//! End-to-end workflow test.
//!
//! The NVIDIA API is replaced by a local mock server (live NVIDIA access is not available
//! in CI). Everything else is real: files on disk, `npm install`, the TypeScript + Vite
//! production build, the Vite dev server answering HTTP, Build Doctor repair, history
//! restore, export, and reopening the database.
//!
//! Requires Node.js/npm and access to the npm registry. Skips (with a message) otherwise.

use genius_core::app::Genius;
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::process::find_executable;
use genius_core::secrets::MemoryStore;
use genius_core::settings::Policy;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const APP_V1: &str = r#"import { useState } from 'react';
import Counter from './components/Counter';
import './App.css';

export default function App() {
  const [title] = useState('Genius Counter');
  return (
    <main className="app">
      <h1>{title}</h1>
      <Counter initial={0} />
    </main>
  );
}
"#;

const COUNTER: &str = r#"import { useState } from 'react';

type Props = { initial: number };

export default function Counter({ initial }: Props) {
  const [count, setCount] = useState(initial);
  return (
    <section aria-label="counter">
      <p data-testid="value">Count: {count}</p>
      <button onClick={() => setCount((c) => c + 1)}>Increment</button>
    </section>
  );
}
"#;

const COUNTER_TEST: &str = r#"import { describe, expect, it } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import Counter from './Counter';

describe('Counter', () => {
  it('increments', () => {
    render(<Counter initial={1} />);
    fireEvent.click(screen.getByText('Increment'));
    expect(screen.getByTestId('value').textContent).toBe('Count: 2');
  });
});
"#;

const APP_FOOTER: &str = r#"import { useState } from 'react';
import Counter from './components/Counter';
import './App.css';

export default function App() {
  const [title] = useState('Genius Counter');
  return (
    <main className="app">
      <h1>{title}</h1>
      <Counter initial={0} />
      <footer className="footer">Built with Genius</footer>
    </main>
  );
}
"#;

// Deliberately broken: references an undefined identifier → real TS2304 from tsc.
const APP_BROKEN: &str = r#"import Counter from './components/Counter';
import './App.css';

export default function App() {
  return (
    <main className="app">
      <h1>{pageTitle}</h1>
      <Counter initial={0} />
      <footer className="footer">Built with Genius</footer>
    </main>
  );
}
"#;

const APP_FIXED: &str = r#"import Counter from './components/Counter';
import './App.css';

const pageTitle = 'Genius Counter';

export default function App() {
  return (
    <main className="app">
      <h1>{pageTitle}</h1>
      <Counter initial={0} />
      <footer className="footer">Built with Genius</footer>
    </main>
  );
}
"#;

fn block(path: &str, content: &str) -> String {
    format!("<<<FILE: {path}>>>\n{content}<<<END FILE>>>\n")
}

fn sse_body(text: &str) -> String {
    let mut s = String::new();
    // stream in small chunks to exercise incremental parsing
    let chars: Vec<char> = text.chars().collect();
    for chunk in chars.chunks(37) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    s
}

struct MockNvidia;

impl Respond for MockNvidia {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req.headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
        if auth != "Bearer nvapi-e2e-test-key-0123456789" {
            return ResponseTemplate::new(401).set_body_string(r#"{"status":401,"detail":"Authentication failed"}"#);
        }
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["model"], "mock/coder-1", "selected model is used for every request");
        let system = body["messages"][0]["content"].as_str().unwrap_or("");
        let user = body["messages"][1]["content"].as_str().unwrap_or("");
        let text = if system.contains("planning engine") {
            json!({
                "name": "Genius Counter", "purpose": "A counter app.",
                "features": [{"name":"Counter","description":"Increment a number"}],
                "pages": [{"name":"Home","description":"Main page","components":["Counter"]}],
                "components": [{"name":"Counter","description":"Shows and increments"}],
                "dataModels": [], "architecture": "Single page, local state.",
                "dependencies": [], "milestones": ["Counter"], "testStrategy": "Vitest component test",
                "risks": [], "assumptions": ["No persistence needed"],
                "files": [{"path":"src/App.tsx","purpose":"root"}]
            })
            .to_string()
        } else if system.contains("implementation engine") {
            format!(
                "Building a counter app.\n{}{}{}{}",
                block("src/App.tsx", APP_V1),
                block("src/components/Counter.tsx", COUNTER),
                block("src/components/Counter.test.tsx", COUNTER_TEST),
                block("src/App.css", ".app { padding: 2rem; }\n.footer { color: gray; }\n")
            )
        } else if system.contains("Build Doctor") {
            assert!(user.contains("TS2304"), "doctor receives real tsc diagnostics");
            assert!(user.contains("<<<CURRENT: src/App.tsx>>>"), "doctor receives the referenced file");
            format!("Root cause: `pageTitle` is used but never declared.\n{}", block("src/App.tsx", APP_FIXED))
        } else if system.contains("code-editing engine") {
            if user.contains("BREAK") {
                format!("Using a page title constant.\n{}", block("src/App.tsx", APP_BROKEN))
            } else {
                assert!(user.contains("<<<CURRENT: src/App.tsx>>>"), "relevant file sent as context");
                format!("Added a footer.\n{}", block("src/App.tsx", APP_FOOTER))
            }
        } else if system.contains("commit message") {
            "Add counter app".into()
        } else {
            "ready".into()
        };
        ResponseTemplate::new(200).set_body_raw(sse_body(&text), "text/event-stream")
    }
}

async fn wait_tasks(g: &Arc<Genius>, pid: &str) {
    for _ in 0..2400 {
        if g.active_tasks(Some(pid)).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("tasks did not finish");
}

fn task_result(g: &Arc<Genius>, pid: &str, task_id: &str) -> Value {
    let ev = g.bus.list(Some(pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(task_id) && e.kind == EventKind::Task).expect("task event");
    assert!(ev.status.is_terminal(), "task finished");
    assert_eq!(ev.status, EventStatus::Completed, "task failed: {}", ev.detail);
    ev.detail["result"].clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn full_mvp_workflow() {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        eprintln!("SKIPPED: npm not available (or GENIUS_SKIP_E2E set)");
        return;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"object":"list","data":[{"id":"mock/coder-1","object":"model","owned_by":"mock"},{"id":"mock/embed-1","object":"model","owned_by":"mock"}]}"#))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(MockNvidia).mount(&server).await;

    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let secrets = Arc::new(MemoryStore::default());
    let g = Genius::new(&data, Arc::new(NullEmitter), secrets.clone()).unwrap();

    // ---- Settings: key, connection, models
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.approvals.dependency_install = Policy::Auto; // unattended test; default is Ask
    g.save_settings(&s).unwrap();
    assert!(g.list_models(true).await.unwrap().models.len() == 2, "listing works without key? (mock allows)");
    g.save_api_key("nvapi-e2e-test-key-0123456789").unwrap();
    let ks = g.key_status();
    assert!(ks.stored && ks.hint.as_deref() == Some("nvapi-…6789"));
    let models = g.list_models(true).await.unwrap();
    assert_eq!(models.source, "live");
    assert_eq!(models.models[0].id, "mock/coder-1");
    assert_eq!(g.list_models(false).await.unwrap().source, "cache");
    g.select_model("mock/coder-1").unwrap();
    let conn = g.test_connection().await.unwrap();
    assert_eq!(conn["probe"]["chat"], true);
    assert_eq!(conn["probe"]["streaming"], true);

    // ---- Create + plan
    let p = g.create_project("Genius Counter", "").unwrap();
    let pid = p.id.clone();
    let t = g.start_plan(&pid, "Build a simple counter application with a button.", "").unwrap();
    wait_tasks(&g, &pid).await;
    let plan = task_result(&g, &pid, &t.id);
    assert_eq!(plan["spec"]["name"], "Genius Counter");
    assert!(g.project(&pid).unwrap().notes.contains("Single page"), "project memory seeded from plan");

    // ---- Generate → install → build → preview (all real)
    let t = g.start_generate(&pid).unwrap();
    wait_tasks(&g, &pid).await;
    let gen = task_result(&g, &pid, &t.id);
    assert_eq!(gen["applied"], true);
    assert_eq!(gen["verify"]["ok"], true, "real build succeeded: {gen}");
    let url = gen["verify"]["previewUrl"].as_str().expect("preview url").to_string();
    let files: Vec<String> = g.list_files(&pid).unwrap().into_iter().map(|f| f.path).collect();
    for f in ["package.json", "src/App.tsx", "src/components/Counter.tsx", "index.html"] {
        assert!(files.contains(&f.to_string()), "{f} exists on disk");
    }
    assert!(!files.iter().any(|f| f.starts_with("node_modules")), "node_modules hidden from tree");
    let html = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(html.contains("<div id=\"root\"></div>"), "dev server serves the real index.html");
    let app_module = reqwest::get(format!("{url}src/App.tsx")).await.unwrap().text().await.unwrap();
    assert!(app_module.contains("Genius Counter"), "dev server serves the generated App");

    // ---- Run the generated tests for real
    let out = g.run_operation(&pid, None, genius_core::process::Operation::Test, "e2e", true, &Default::default()).await.unwrap();
    assert!(out.ok(), "vitest passed: {}", out.output_tail);

    // ---- Edit in editor (manual change recorded)
    let css = g.read_file(&pid, "src/App.css").unwrap();
    g.save_file(&pid, "src/App.css", &format!("{css}h1 {{ margin: 0; }}\n")).unwrap().expect("manual changeset");

    // ---- Change request → reviewable diff → apply → verified by build
    let ctx = g.preview_context(&pid, "Add a footer to the App", &[], &[]).unwrap();
    assert!(ctx.files.iter().any(|f| f.path == "src/App.tsx" && f.included));
    let t = g.start_change(&pid, "Add a footer to the App", vec![], vec![]).unwrap();
    wait_tasks(&g, &pid).await;
    let ch = task_result(&g, &pid, &t.id);
    assert_eq!(ch["applied"], false, "default policy requires review");
    let cs_id = ch["changesetId"].as_str().unwrap().to_string();
    let (cs, cs_files) = g.changeset(&cs_id).unwrap();
    assert_eq!(cs.status, "proposed");
    let diff = genius_core::changes::unified_diff(&cs_files[0].path, cs_files[0].before.as_deref(), cs_files[0].after.as_deref());
    assert!(diff.contains("+      <footer className=\"footer\">Built with Genius</footer>"));
    assert!(!g.read_file(&pid, "src/App.tsx").unwrap().contains("footer"), "not applied before approval");
    g.apply_changeset(&cs_id).unwrap();
    assert!(g.read_file(&pid, "src/App.tsx").unwrap().contains("footer"));
    wait_tasks(&g, &pid).await; // background verification build
    assert_eq!(g.project(&pid).unwrap().status["lastBuild"]["status"], "succeeded");

    // ---- Broken change → real build failure → Build Doctor → fix → build passes
    let t = g.start_change(&pid, "BREAK: use a pageTitle constant", vec![], vec![]).unwrap();
    wait_tasks(&g, &pid).await;
    let broken = task_result(&g, &pid, &t.id)["changesetId"].as_str().unwrap().to_string();
    let mut s = g.settings().unwrap();
    s.approvals.apply_ai_edits = Policy::Auto; // let the doctor's fix apply + re-verify
    g.save_settings(&s).unwrap();
    g.apply_changeset(&broken).unwrap();
    wait_tasks(&g, &pid).await;
    let runs = g.db.list_runs(&pid, 20).unwrap();
    let failed = runs.iter().find(|r| r.op == "build" && r.status == "failed").expect("a real failed build");
    assert!(failed.output_tail.contains("TS2304"), "real tsc output captured");
    assert_ne!(failed.exit_code, Some(0));
    let latest_build = runs.iter().find(|r| r.op == "build").unwrap();
    assert_eq!(latest_build.status, "succeeded", "doctor fix verified by a new real build");
    let repair = g.list_changesets(&pid).unwrap().into_iter().find(|c| c.kind == "repair").expect("repair changeset");
    assert_eq!(repair.status, "applied");
    assert!(repair.summary.contains("Root cause"));

    // ---- Restore an earlier state, then export
    g.stop_preview(&pid).await.unwrap();
    assert!(g.preview_url(&pid).is_none());
    let gen_cs = g.list_changesets(&pid).unwrap().into_iter().find(|c| c.kind == "generate").unwrap();
    let restore = g.propose_restore_to(&gen_cs.id).unwrap();
    g.apply_changeset(&restore.id).unwrap();
    assert_eq!(g.read_file(&pid, "src/App.tsx").unwrap(), APP_V1);
    let zip = tmp.path().join("export.zip");
    let ex = g.export_project(&pid, zip.to_str().unwrap(), true).unwrap();
    assert!(ex.files >= 8 && zip.exists());

    // ---- Readiness report reflects real results
    let checks = g.readiness(&pid).await.unwrap();
    let get = |id: &str| checks.iter().find(|c| c.id == id).unwrap().status.clone();
    assert_eq!(get("deps"), "pass");
    assert_eq!(get("secrets"), "pass");

    // ---- Close and reopen: everything persisted
    g.shutdown().await;
    drop(g);
    let g2 = Genius::new(&data, Arc::new(NullEmitter), secrets).unwrap();
    let projects = g2.list_projects().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].spec.as_ref().unwrap()["name"], "Genius Counter");
    assert!(g2.list_changesets(&pid).unwrap().len() >= 6);
    assert!(g2.bus.list(Some(&pid), 3000).unwrap().iter().all(|e| e.status != EventStatus::Running), "no stale running events");
    assert!(g2.db.list_messages(&pid, 50).unwrap().len() >= 4);
}

#[tokio::test]
async fn invalid_key_is_reported_clearly() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(MockNvidia).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401).set_body_string(r#"{"detail":"Authentication failed"}"#))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(tmp.path(), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    g.save_settings(&s).unwrap();
    let e = g.test_connection().await.unwrap_err();
    assert_eq!(e.kind(), "provider_missing_key");
    g.save_api_key("nvapi-wrong-key-000000000000").unwrap();
    let e = g.test_connection().await.unwrap_err();
    assert_eq!(e.kind(), "provider_auth");
    let ser = serde_json::to_string(&e).unwrap();
    assert!(!ser.contains("nvapi-wrong"), "key never appears in errors: {ser}");
    let ev = g.bus.list(None, 50).unwrap();
    assert!(!serde_json::to_string(&ev).unwrap().contains("nvapi-wrong-key"), "key never appears in activity");
}
