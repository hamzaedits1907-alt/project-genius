//! Project lifecycle regression suite: install → preview → conversational edits → repair.
//!
//! REAL: the Genius template on disk, `npm install` from the project root (npm registry),
//! the Vite dev server, the identity-checked HTTP readiness and module-graph compile check,
//! `tsc && vite build`, headless Chromium rendering the previewed page, process cleanup,
//! and reopening the database.
//!
//! MOCK (deterministic, clearly not NVIDIA): the model. A local wiremock server plays the
//! model's side of the protocol with scripted answers. These tests say nothing about how a
//! real NVIDIA model behaves.
//!
//! Requires Node.js/npm and registry access; skipped with a message otherwise
//! (`GENIUS_SKIP_E2E=1` to skip). Chromium rendering checks are skipped when no Chromium is found.

use genius_core::agent::{AgentMode, AgentRequest};
use genius_core::app::Genius;
use genius_core::events::{EventKind, NullEmitter};
use genius_core::preview::{self, CheckFailure, PreviewState, PreviewStatus};
use genius_core::process::find_executable;
use genius_core::secrets::MemoryStore;
use genius_core::settings::Policy;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(text: &str) -> ResponseTemplate {
    let mut s = String::new();
    for chunk in text.chars().collect::<Vec<_>>().chunks(41) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
}

/// Scripted model: queued answers per role, recorded requests.
#[derive(Default)]
struct Model {
    agent: Mutex<VecDeque<String>>,
    doctor: Mutex<VecDeque<String>>,
    log: Mutex<Vec<String>>,
}

struct Responder(Arc<Model>);

impl Respond for Responder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let system = msgs[0]["content"].as_str().unwrap_or("");
        let last = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        self.0.log.lock().unwrap().push(last);
        let answer = if system.contains("Reviewer subagent") {
            "<final>No blocking issues found.</final>".to_string()
        } else if system.contains("Build Doctor") {
            self.0.doctor.lock().unwrap().pop_front().unwrap_or_else(|| "Root cause: unknown. No change.".into())
        } else {
            self.0.agent.lock().unwrap().pop_front().unwrap_or_else(|| "<final>Nothing to do.</final>".into())
        };
        sse(&answer)
    }
}

fn block(p: &str, c: &str) -> String {
    format!("<<<FILE: {p}>>>\n{c}<<<END FILE>>>\n")
}

/// Kills every process the test started even when an assertion fails half-way (a panicking
/// test never reaches `shutdown`, and its dev servers would outlive it).
struct KillOnDrop(Arc<Genius>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        self.0.procs.kill_all_now();
    }
}

fn skip() -> bool {
    if std::env::var("GENIUS_SKIP_E2E").is_ok() || find_executable("npm").is_none() {
        eprintln!("SKIPPED: npm not available or GENIUS_SKIP_E2E set");
        return true;
    }
    false
}

async fn genius(data: &std::path::Path, projects: &std::path::Path, server: &MockServer) -> Arc<Genius> {
    let g = Genius::new(data, Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/scripted-coder".into());
    s.workspace_dir = Some(projects.to_string_lossy().to_string());
    s.approvals.dependency_install = Policy::Auto;
    s.approvals.apply_ai_edits = Policy::Auto;
    s.verify_after_apply = true;
    s.max_repair_attempts = 0; // Build Doctor runs only when a test asks for it
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-lifecycle-test-key-0123456789").unwrap();
    g
}

fn app_tsx(heading: &str) -> String {
    format!("import './App.css';\n\nexport default function App() {{\n  return (\n    <main className=\"app\">\n      <h1>{heading}</h1>\n    </main>\n  );\n}}\n")
}

/// A project containing exactly the Genius template plus a small App.
fn template_project(g: &Arc<Genius>, name: &str, heading: &str) -> String {
    let p = g.create_project(name, "lifecycle test").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    for (path, content) in genius_core::scaffold::files(name, "test") {
        ws.write_text(&path, &content).unwrap();
    }
    ws.write_text("src/App.tsx", &app_tsx(heading)).unwrap();
    ws.write_text("src/App.css", ".app { padding: 2rem; }\n").unwrap();
    p.id
}

async fn settle(g: &Arc<Genius>, pid: &str, secs: u64) -> PreviewState {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let st = g.preview_state(pid);
        if !st.status.busy() && st.status != PreviewStatus::Idle {
            return st;
        }
        assert!(Instant::now() < deadline, "preview did not settle in {secs}s: {st:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_tasks(g: &Arc<Genius>, pid: &str, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !g.active_tasks(Some(pid)).is_empty() {
        assert!(Instant::now() < deadline, "task did not finish in {secs}s");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

fn task_result(g: &Arc<Genius>, pid: &str, task_id: &str) -> Value {
    let ev = g.bus.list(Some(pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(task_id) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).expect("task event");
    json!({ "status": ev.status, "detail": ev.detail })
}

fn chromium() -> Option<PathBuf> {
    ["/opt/pw-browsers/chromium"].iter().map(PathBuf::from).find(|p| p.exists()).or_else(|| find_executable("chromium")).or_else(|| find_executable("google-chrome"))
}

/// Render the page in headless Chromium and return the DOM after scripts ran.
fn render(url: &str) -> Option<String> {
    let chrome = chromium()?;
    let out = std::process::Command::new(chrome)
        .args(["--headless=new", "--no-sandbox", "--disable-gpu", "--no-proxy-server", "--no-first-run", "--virtual-time-budget=10000", "--dump-dom", url])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn assert_renders(url: &str, needle: &str) {
    match render(url) {
        Some(dom) => assert!(dom.contains(needle), "rendered page should contain {needle:?}; got:\n{}", &dom[..dom.len().min(3000)]),
        None => eprintln!("NOTE: Chromium not found; rendering of {needle:?} not verified"),
    }
}

async fn agent(g: &Arc<Genius>, pid: &str, msg: &str) -> Value {
    let conv = g.db.current_conversation(pid).unwrap();
    let t = g.start_agent(pid, AgentRequest { conversation_id: conv.id, message: msg.into(), mode: AgentMode::Build, ..Default::default() }).unwrap();
    wait_tasks(g, pid, 300).await;
    task_result(g, pid, &t.id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn preview_edit_repair_lifecycle() {
    if skip() {
        return;
    }
    let model = Arc::new(Model::default());
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Responder(model.clone())).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let g = genius(&data, &tmp.path().join("projects"), &server).await;
    let _cleanup = KillOnDrop(g.clone());
    let pid = template_project(&g, "Lifecycle App", "Hello from Genius");

    // ---- 1-2. Dependencies are detected as missing from the files themselves.
    let dep = g.dep_status(&pid).unwrap();
    assert!(dep.needs_install(), "{dep:?}");
    assert!(dep.reason.contains("not been installed"));
    assert!(dep.new_packages.is_empty(), "template packages are trusted: {:?}", dep.new_packages);

    // ---- 3-5. One call: install → start → identity + compile check → ready; real render.
    let started = Instant::now();
    let first = g.start_preview(&pid).unwrap();
    assert!(started.elapsed() < Duration::from_secs(2), "start_preview returns immediately");
    assert!(first.status.busy(), "{first:?}");
    let st = settle(&g, &pid, 240).await;
    assert_eq!(st.status, PreviewStatus::Ready, "{st:?}");
    assert!(st.modules_checked >= 3, "main.tsx, App.tsx, css compiled: {st:?}");
    assert!(!g.dep_status(&pid).unwrap().needs_install(), "install recorded");
    let runs = g.db.list_runs(&pid, 20).unwrap();
    let install = runs.iter().find(|r| r.op == "install").expect("install ran");
    assert_eq!(install.status, "succeeded");
    assert_eq!(PathBuf::from(&install.cwd), g.workspace(&pid).unwrap().root().to_path_buf(), "npm ran in the project root");
    let url = st.url.clone().unwrap();
    let res = reqwest::Client::builder().no_proxy().build().unwrap().get(&url).send().await.unwrap();
    assert!(res.headers().get(preview::TOKEN_HEADER).is_some(), "identity header served");
    assert_renders(&url, "<h1>Hello from Genius</h1>");

    // no duplicate server when Preview is pressed again
    let again = g.start_preview(&pid).unwrap();
    assert_eq!(again.run_id, st.run_id);
    assert_eq!(g.procs.active(Some(&pid)).iter().filter(|r| r.op == genius_core::process::Operation::Dev).count(), 1);

    // ---- 6-7. Conversational change on the same project; preview shows it after refresh.
    model.agent.lock().unwrap().push_back(format!("<final>I changed the heading.\n{}</final>", block("src/App.tsx", &app_tsx("Hello again"))));
    let r = agent(&g, &pid, "Change the heading to Hello again").await;
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["detail"]["result"]["applied"], true, "{r}");
    assert_eq!(r["detail"]["result"]["verify"]["build"]["status"], "succeeded", "{r}");
    assert_eq!(r["detail"]["result"]["verify"]["preview"]["status"], "ready", "{r}");
    assert_eq!(g.preview_state(&pid).run_id, st.run_id, "same server, hot-reloaded");
    assert_renders(&url, "<h1>Hello again</h1>");

    // ---- 9. Malformed proposals are blocked before they touch files.
    let pkg_before = std::fs::read_to_string(g.workspace(&pid).unwrap().root().join("package.json")).unwrap();
    let dup_pkg = pkg_before.trim_end().trim_end_matches('}').to_string() + ",\n  \"dependencies\": { \"uuid\": \"^11.1.0\" }\n}\n";
    let uses_uuid = "import { v4 } from 'uuid';\nimport './App.css';\n\nconst id = v4();\n\nexport default function App() {\n  return (\n    <main className=\"app\">\n      <h1>Hello uuid</h1>\n      <p data-len={id.length}>id ready</p>\n    </main>\n  );\n}\n";
    for _ in 0..3 {
        model.agent.lock().unwrap().push_back(format!("<final>Added ids.\n{}{}</final>", block("package.json", &dup_pkg), block("src/App.tsx", uses_uuid)));
    }
    let r = agent(&g, &pid, "Give every note a unique id").await;
    assert_eq!(r["status"], "failed", "blocked after bounded correction rounds: {r}");
    let ws = g.workspace(&pid).unwrap();
    assert_eq!(std::fs::read_to_string(ws.root().join("package.json")).unwrap(), pkg_before, "package.json untouched");
    assert!(ws.read_text("src/App.tsx").unwrap().contains("Hello again"), "App.tsx untouched");
    assert!(model.log.lock().unwrap().iter().any(|m| m.contains("BLOCKED") && m.contains("duplicate key")), "model was told why");
    let msgs = g.db.list_conversation_messages(&g.db.current_conversation(&pid).unwrap().id, 50).unwrap();
    assert!(msgs.last().unwrap().content.contains("duplicate key"), "user sees the reason");

    // ---- 8. A corrected proposal (after one blocked round) changes package.json; dependency
    //         state is recalculated, the new package installed, the preview restarted.
    let mut pkg: Value = serde_json::from_str(&pkg_before).unwrap();
    pkg["dependencies"]["uuid"] = json!("^11.1.0");
    let good_pkg = serde_json::to_string_pretty(&pkg).unwrap() + "\n";
    model.agent.lock().unwrap().push_back(format!("<final>Added ids.\n{}</final>", block("src/App.tsx", uses_uuid))); // forgets package.json
    model.agent.lock().unwrap().push_back(format!("<final>Added ids with the uuid package.\n{}{}</final>", block("package.json", &good_pkg), block("src/App.tsx", uses_uuid)));
    let r = agent(&g, &pid, "Give every note a unique id").await;
    assert_eq!(r["status"], "completed", "{r}");
    assert!(model.log.lock().unwrap().iter().any(|m| m.contains("'uuid', which is not listed in package.json")), "undeclared import was blocked first");
    let v = &r["detail"]["result"]["verify"];
    assert_eq!(v["install"], "succeeded", "{r}");
    assert_eq!(v["preview"]["status"], "ready", "{r}");
    let st2 = g.preview_state(&pid);
    assert_ne!(st2.run_id, st.run_id, "dependencies changed → server restarted");
    assert!(!g.dep_status(&pid).unwrap().needs_install());
    assert_renders(st2.url.as_deref().unwrap(), "Hello uuid");

    // ---- 10. Missing dependency in code written outside the AI (the reported `uuid` class of
    //          failure): the preview reports the real compile error; Fix automatically repairs it.
    // (dayjs is not a transitive dependency of the template, unlike e.g. nanoid, which Vite pulls in)
    let uses_dayjs = "import dayjs from 'dayjs';\nimport './App.css';\n\nexport default function App() {\n  return (\n    <main className=\"app\">\n      <h1>Hello dayjs</h1>\n      <p>{dayjs('2024-01-02').format('YYYY')}</p>\n    </main>\n  );\n}\n";
    g.save_file(&pid, "src/App.tsx", uses_dayjs).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let broken = loop {
        let s = g.preview_state(&pid);
        if s.status == PreviewStatus::Failed {
            break s;
        }
        assert!(Instant::now() < deadline, "compile error not detected: {s:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let problem = broken.problem.clone().unwrap();
    assert_eq!(problem.stage, "compile");
    assert!(problem.summary.contains("dayjs"), "{problem:?}");
    assert_eq!(problem.file.as_deref(), Some("src/App.tsx"));
    let mut pkg2: Value = serde_json::from_str(&ws.read_text("package.json").unwrap()).unwrap();
    pkg2["dependencies"]["dayjs"] = json!("^1.11.13");
    model
        .doctor
        .lock()
        .unwrap()
        .push_back(format!("Root cause: src/App.tsx imports dayjs, which is not a dependency.\n{}", block("package.json", &(serde_json::to_string_pretty(&pkg2).unwrap() + "\n"))));
    let t = g.start_auto_fix(&pid, None).unwrap();
    wait_tasks(&g, &pid, 400).await;
    let r = task_result(&g, &pid, &t.id);
    assert_eq!(r["status"], "completed", "{r}");
    assert_eq!(r["detail"]["result"]["fixed"], true, "{r}");
    assert_eq!(r["detail"]["result"]["attempts"].as_array().unwrap().len(), 1);
    assert!(
        model.log.lock().unwrap().iter().any(|m| m.contains("Failed to resolve import") && m.contains("'dayjs' is imported but NOT listed")),
        "doctor got the real error and the deterministic hint"
    );
    let fixed = g.preview_state(&pid);
    assert_eq!(fixed.status, PreviewStatus::Ready);
    assert_renders(fixed.url.as_deref().unwrap(), "<p>2024</p>");

    // ---- 11. A production-build failure is diagnosed accurately and does not take the
    //          (working) development preview down with it.
    let type_error = "import './App.css';\n\nexport default function App() {\n  const count: number = 'three';\n  return <main className=\"app\"><h1>Typed {count}</h1></main>;\n}\n";
    g.save_file(&pid, "src/App.tsx", type_error).unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let rep = g.verify_pipeline(&pid, "manual-verify", &cancel, false).await.unwrap();
    assert_eq!(rep["build"]["status"], "failed", "{rep}");
    let summary = rep["build"]["summary"].as_str().unwrap();
    assert!(summary.contains("TypeScript error in src/App.tsx line 4") && summary.contains("not assignable"), "{summary}");
    assert_eq!(rep["preview"]["status"], "ready", "dev preview still works: {rep}");
    assert_eq!(rep["ok"], false);

    // ---- 12. Restart and stop without duplicate or orphaned processes.
    let before = g.preview_state(&pid);
    let old_pid = g.procs.pid_of(before.run_id.as_deref().unwrap()).unwrap();
    g.restart_preview(&pid).unwrap();
    let after = settle(&g, &pid, 120).await;
    assert_eq!(after.status, PreviewStatus::Ready, "{after:?}");
    assert_ne!(after.run_id, before.run_id);
    assert!(after.generation > before.generation);
    assert_eq!(g.procs.active(Some(&pid)).len(), 1, "exactly one server");
    #[cfg(unix)]
    assert_ne!(unsafe { libc::kill(-(old_pid as i32), 0) }, 0, "old process group is gone");
    let port = after.port.unwrap();
    g.stop_preview(&pid).await.unwrap();
    let stopped = g.preview_state(&pid);
    assert_eq!(stopped.status, PreviewStatus::Idle);
    assert!(stopped.url.is_none() && g.preview_url(&pid).is_none());
    assert!(g.procs.active(Some(&pid)).is_empty());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err(), "port released");

    // ---- 13. Project and conversation survive an application restart; no stale "ready".
    let conv = g.db.current_conversation(&pid).unwrap();
    let n_msgs = g.db.list_conversation_messages(&conv.id, 500).unwrap().len();
    g.start_preview(&pid).unwrap();
    assert_eq!(settle(&g, &pid, 120).await.status, PreviewStatus::Ready);
    g.shutdown().await;
    drop(g);
    let g2 = genius(&data, &tmp.path().join("projects"), &server).await;
    let _cleanup2 = KillOnDrop(g2.clone());
    assert!(g2.list_projects().unwrap().iter().any(|p| p.id == pid));
    assert_eq!(g2.db.list_conversation_messages(&conv.id, 500).unwrap().len(), n_msgs);
    assert_eq!(g2.preview_state(&pid).status, PreviewStatus::Idle, "a new session never claims an old server");
    assert!(g2.db.list_runs(&pid, 100).unwrap().iter().all(|r| r.status != "running"));
    assert!(!g2.dep_status(&pid).unwrap().needs_install(), "install state persisted on disk");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_projects_server_is_never_taken_for_anothers() {
    if skip() {
        return;
    }
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(&tmp.path().join("data"), &tmp.path().join("projects"), &server).await;
    let _cleanup = KillOnDrop(g.clone());
    let a = template_project(&g, "Project Alpha", "Alpha page");
    let b = template_project(&g, "Project Beta", "Beta page");
    g.start_preview(&a).unwrap();
    g.start_preview(&b).unwrap();
    let (sa, sb) = (settle(&g, &a, 240).await, settle(&g, &b, 240).await);
    assert_eq!((sa.status, sb.status), (PreviewStatus::Ready, PreviewStatus::Ready), "{sa:?} {sb:?}");
    assert_ne!(sa.port, sb.port);
    assert_ne!(sa.run_id, sb.run_id);
    assert_renders(sa.url.as_deref().unwrap(), "Alpha page");
    assert_renders(sb.url.as_deref().unwrap(), "Beta page");

    // Each server carries its own run's token: A's server fails B's identity check.
    let client = preview::http_client();
    let tok = |port: u16| {
        let client = client.clone();
        async move { client.get(format!("http://127.0.0.1:{port}/")).send().await.unwrap().headers().get(preview::TOKEN_HEADER).unwrap().to_str().unwrap().to_string() }
    };
    let (ta, tb) = (tok(sa.port.unwrap()).await, tok(sb.port.unwrap()).await);
    assert_ne!(ta, tb);
    let r = preview::check_server(&client, sa.port.unwrap(), Some(&tb), None, Duration::from_secs(10)).await;
    assert!(matches!(r, Err(CheckFailure::WrongServer(_))), "{r:?}");

    // A foreign program on a port is never accepted as a preview.
    let foreign = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let fport = foreign.local_addr().unwrap().port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for s in foreign.incoming().take(1) {
            let mut s = s.unwrap();
            let mut buf = [0u8; 1024];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 20\r\n\r\n<html>other</html>\r\n");
        }
    });
    let r = preview::check_server(&client, fport, Some(&ta), None, Duration::from_secs(10)).await;
    assert!(matches!(r, Err(CheckFailure::WrongServer(_))), "{r:?}");

    // Stopping one project leaves the other running.
    g.stop_preview(&a).await.unwrap();
    assert_eq!(g.preview_state(&b).status, PreviewStatus::Ready);
    assert_eq!(g.procs.active(Some(&b)).len(), 1);
    g.shutdown().await;
}

/// New (never approved) packages need approval under the default policy; declining installs nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_packages_require_approval() {
    if find_executable("npm").is_none() {
        eprintln!("SKIPPED: npm not available");
        return;
    }
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(&tmp.path().join("data"), &tmp.path().join("projects"), &server).await;
    let mut s = g.settings().unwrap();
    s.approvals.dependency_install = Policy::Ask;
    g.save_settings(&s).unwrap();
    let p = g.create_project("Approval", "x").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", r#"{"name":"approval","scripts":{"dev":"vite"},"dependencies":{"left-pad":"^1.3.0"}}"#).unwrap();
    let st = g.dep_status(&p.id).unwrap();
    assert_eq!(st.new_packages, vec!["left-pad".to_string()]);
    let g2 = g.clone();
    let pid = p.id.clone();
    let h = tokio::spawn(async move { g2.ensure_installed(&pid, None, &tokio_util::sync::CancellationToken::new()).await });
    let deadline = Instant::now() + Duration::from_secs(10);
    let req = loop {
        if let Some(r) = g.approvals.pending(Some(&p.id)).into_iter().next() {
            break r;
        }
        assert!(Instant::now() < deadline, "no approval requested");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert!(req.title.contains("left-pad"), "{}", req.title);
    g.approvals.respond(&req.id, false).unwrap();
    assert!(h.await.unwrap().is_err(), "declined");
    assert!(!ws.root().join("node_modules").exists(), "nothing installed");
}
