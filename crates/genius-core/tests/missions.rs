//! Custom agents and missions against a scripted mock model (wiremock). Everything else —
//! the agent loop, tool enforcement, isolated workspaces, conflict checks, changesets, SQLite — is real.

use genius_core::app::Genius;
use genius_core::custom_agents::{Budget, CustomAgent, Team};
use genius_core::events::RecordingEmitter;
use genius_core::missions::Mission;
use genius_core::secrets::MemoryStore;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(text: &str) -> ResponseTemplate {
    let mut s = String::new();
    for chunk in text.chars().collect::<Vec<_>>().chunks(40) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"total_tokens\":120}}\n\ndata: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
}

type Script = dyn Fn(&str, usize, &str) -> String + Send + Sync;
struct Scripted(Box<Script>);
impl Respond for Scripted {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let system = msgs[0]["content"].as_str().unwrap_or("").to_string();
        let results = msgs.iter().filter(|m| m["content"].as_str().unwrap_or("").starts_with("<tool_result")).count();
        let last = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        sse(&(self.0)(&system, results, &last))
    }
}

struct Env {
    _tmp: tempfile::TempDir,
    g: Arc<Genius>,
    pid: String,
    emitter: Arc<RecordingEmitter>,
    _server: MockServer,
}

async fn setup(script: impl Fn(&str, usize, &str) -> String + Send + Sync + 'static) -> Env {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted(Box::new(script))).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let emitter = Arc::new(RecordingEmitter::default());
    let g = Genius::new(&tmp.path().join("data"), emitter.clone(), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/coder-1".into());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-agent-test-key-0123456789").unwrap();
    let p = g.create_project("Demo", "demo").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", r#"{"name":"demo","scripts":{}}"#).unwrap();
    ws.write_text("src/a.ts", "export const a = 1;\n").unwrap();
    ws.write_text("src/b.ts", "export const b = 1;\n").unwrap();
    ws.write_text("docs/secret.md", "top secret plan\n").unwrap();
    Env { _tmp: tmp, g, pid: p.id, emitter, _server: server }
}

fn agent(name: &str, f: impl FnOnce(&mut CustomAgent)) -> CustomAgent {
    let mut a = CustomAgent { name: name.into(), ..Default::default() };
    f(&mut a);
    a
}

async fn wait_mission(g: &Arc<Genius>, id: &str, want: &[&str]) -> Mission {
    for _ in 0..600 {
        let m = g.get_mission(id).unwrap();
        if want.contains(&m.status.as_str()) {
            return m;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("mission did not reach {want:?}: {:#?}", g.get_mission(id).unwrap());
}

async fn wait_task(g: &Arc<Genius>, id: &str, want: &[&str]) -> genius_core::missions::MissionTask {
    for _ in 0..600 {
        let t = g.db.get_task_row(id).unwrap();
        if want.contains(&t.status.as_str()) {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("task did not reach {want:?}: {:#?}", g.db.get_task_row(id).unwrap());
}

fn call(name: &str, args: Value) -> String {
    format!("<tool>{}</tool>", json!({ "name": name, "args": args }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn custom_agent_respects_tool_scope_network_and_plan_only_limits() {
    let env = setup(|_sys, n, last| match n {
        0 => format!(
            "Trying things.\n{}\n{}\n{}\n{}",
            call("read_file", json!({ "path": "docs/secret.md" })),
            call("read_file", json!({ "path": "src/a.ts" })),
            call("web_search", json!({ "query": "rust" })),
            call("write_file", json!({ "path": "src/a.ts", "content": "hacked" }))
        ),
        _ => {
            assert!(last.contains("outside the scope"), "scope refusal: {last}");
            assert!(last.contains("network access is disabled"), "{last}");
            assert!(last.contains("plan_only"), "{last}");
            assert!(last.contains("export const a = 1"), "in-scope read works: {last}");
            "<final>Reviewed src only.</final>".into()
        }
    })
    .await;
    let a = env
        .g
        .save_agent(agent("Scout", |a| {
            a.tools = vec!["read_file".into(), "web_search".into(), "write_file".into()];
            a.approval = "plan_only".into();
            a.network = false;
            a.scope.paths = vec!["src".into()];
        }))
        .unwrap();
    let m = env.g.run_agent_task(&a.id, &env.pid, "Look at the sources").unwrap();
    let m = wait_mission(&env.g, &m.id, &["completed", "failed"]).await;
    assert_eq!(m.status, "completed", "{m:#?}");
    assert_eq!(m.tasks[0].status, "done");
    assert!(m.tasks[0].result.as_deref().unwrap().contains("Reviewed src only"));
    assert!(m.tasks[0].violations.len() >= 3, "{:?}", m.tasks[0].violations);
    // plan_only never writes
    assert_eq!(env.g.workspace(&env.pid).unwrap().read_text("src/a.ts").unwrap(), "export const a = 1;\n");
    // logs and usage
    let logs = env.g.agent_logs(&a.id, None).unwrap();
    assert!(logs.iter().any(|l| l.kind == "tool") && logs.iter().any(|l| l.kind == "result") && logs.iter().any(|l| l.kind == "status"));
    let rt = env.g.agent_runtimes().unwrap().into_iter().find(|r| r.agent_id == a.id).unwrap();
    assert_eq!((rt.status.as_str(), rt.usage.tasks_done), ("completed", 1));
    assert!(rt.usage.tokens > 0 && rt.usage.calls >= 2);
    // real status transitions were emitted, in order
    let st: Vec<String> = env.emitter.events.lock().unwrap().iter().filter(|(c, p)| c == "genius://agent-status" && p["agentId"] == json!(a.id)).map(|(_, p)| p["status"].as_str().unwrap().to_string()).collect();
    assert_eq!(st, vec!["planning", "completed"], "plan_only agents plan, then complete");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_exhaustion_is_an_honest_terminal_outcome() {
    let env = setup(|_s, n, _l| call("list_files", json!({ "prefix": format!("src{n}") }))).await;
    let a = env.g.save_agent(agent("Looper", |a| a.budget = Budget { max_steps: 50, max_minutes: 5, max_tokens: 5000, max_model_calls: 3 })).unwrap();
    let m = env.g.run_agent_task(&a.id, &env.pid, "Loop forever").unwrap();
    let m = wait_mission(&env.g, &m.id, &["completed", "failed"]).await;
    assert_eq!(m.status, "failed");
    let t = &m.tasks[0];
    assert_eq!(t.status, "failed");
    assert!(t.blocker.as_deref().unwrap().starts_with("budget exhausted: maxModelCalls"), "{:?}", t.blocker);
    assert!(t.result.as_deref().unwrap().starts_with("budget exhausted"));
    assert_eq!(t.usage.calls, 3);
}

fn plan(tasks: Value) -> String {
    json!({ "summary": "plan", "tasks": tasks }).to_string()
}

fn who(sys: &str) -> &'static str {
    if sys.contains("coordinator \"Coordinator\"") {
        "coord"
    } else if sys.contains("custom agent \"Alpha\"") {
        "alpha"
    } else if sys.contains("custom agent \"Beta\"") {
        "beta"
    } else {
        "other"
    }
}

fn team(env: &Env, alpha_approval: &str, beta_approval: &str) -> (CustomAgent, CustomAgent, Team) {
    let edit = |n: &str, ap: &str| {
        env.g
            .save_agent(agent(n, |a| {
                a.tools = vec!["read_file".into(), "write_file".into(), "edit_file".into(), "list_files".into()];
                a.approval = ap.into();
            }))
            .unwrap()
    };
    let (a, b) = (edit("Alpha", alpha_approval), edit("Beta", beta_approval));
    let t = env.g.save_team(Team { name: "T".into(), coordinator_agent_id: "builtin-coordinator".into(), agent_ids: vec![a.id.clone(), b.id.clone()], ..Default::default() }).unwrap();
    (a, b, t)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_tasks_on_different_files_integrate_and_cycles_fall_back() {
    let ids = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let ids2 = ids.clone();
    let env = setup(move |sys, n, l| match who(sys) {
        "coord" => {
            let v = ids2.lock().unwrap().clone();
            if l.contains("cyclic") || l.contains("a cycle") {
                return plan(json!([
                    { "id": "x", "title": "X", "assignee": v[0], "dependsOn": ["y"] },
                    { "id": "y", "title": "Y", "assignee": v[1], "dependsOn": ["x"] }
                ]));
            }
            plan(json!([
                { "id": "t1", "title": "Edit a", "description": "set a", "assignee": v[0], "dependsOn": [] },
                { "id": "t2", "title": "Edit b", "description": "set b", "assignee": v[1], "dependsOn": [] }
            ]))
        }
        "alpha" if n == 0 => call("write_file", json!({ "path": "src/a.ts", "content": "export const a = 'alpha';\n" })),
        "beta" if n == 0 => call("write_file", json!({ "path": "src/b.ts", "content": "export const b = 'beta';\n" })),
        _ => "<final>done</final>".into(),
    })
    .await;
    let (a, b, t) = team(&env, "auto", "auto");
    *ids.lock().unwrap() = vec![a.id.clone(), b.id.clone()];
    let m = env.g.create_mission(&env.pid, "Update both files", Some(&t.id)).unwrap();
    assert_eq!(m.status, "planning");
    let m = wait_mission(&env.g, &m.id, &["ready"]).await;
    assert_eq!(m.tasks.len(), 2);
    env.g.start_mission(&m.id).unwrap();
    let m = wait_mission(&env.g, &m.id, &["completed", "failed"]).await;
    assert_eq!(m.status, "completed", "{m:#?}");
    let ws = env.g.workspace(&env.pid).unwrap();
    assert!(ws.read_text("src/a.ts").unwrap().contains("alpha") && ws.read_text("src/b.ts").unwrap().contains("beta"));
    assert!(m.tasks.iter().all(|t| t.status == "done" && t.changeset_id.is_some() && t.workspace.is_none()));
    assert!(m.summary.as_deref().unwrap().contains("2 of 2"));
    // applied through the changeset system (revertible), isolated workspaces are gone
    assert!(env.g.list_changesets(&env.pid).unwrap().iter().filter(|c| c.status == "applied" && c.kind == "mission").count() == 2);
    assert!(env.g.list_projects().unwrap().len() == 1, "hidden workspaces are not projects");
    assert!(!env._tmp.path().join("data/mission-workspaces").read_dir().map(|mut d| d.any(|e| e.unwrap().path().read_dir().map(|mut x| x.next().is_some()).unwrap_or(false))).unwrap_or(false));

    // an invalid (empty) plan falls back to a single task
    let m2 = env.g.create_mission(&env.pid, "cyclic goal", Some(&t.id)).unwrap();
    let m2 = wait_mission(&env.g, &m2.id, &["ready"]).await;
    assert_eq!(m2.tasks.len(), 1);
    assert!(m2.meta["plan"].as_str().unwrap().contains("single task"), "{:?}", m2.meta);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_file_conflict_is_reported_and_nothing_is_overwritten() {
    let ids = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let ids2 = ids.clone();
    let env = setup(move |sys, n, _l| match who(sys) {
        "coord" => {
            let v = ids2.lock().unwrap().clone();
            plan(json!([
                { "id": "t1", "title": "A1", "assignee": v[0] },
                { "id": "t2", "title": "A2", "assignee": v[1] }
            ]))
        }
        "alpha" if n == 0 => call("write_file", json!({ "path": "src/a.ts", "content": "export const a = 'alpha';\n" })),
        "beta" if n == 0 => call("write_file", json!({ "path": "src/a.ts", "content": "export const a = 'beta';\n" })),
        _ => "<final>done</final>".into(),
    })
    .await;
    let (a, b, t) = team(&env, "auto", "auto");
    *ids.lock().unwrap() = vec![a.id.clone(), b.id.clone()];
    let m = env.g.create_mission(&env.pid, "Both change a", Some(&t.id)).unwrap();
    let m = wait_mission(&env.g, &m.id, &["ready"]).await;
    env.g.start_mission(&m.id).unwrap();
    let m = wait_mission(&env.g, &m.id, &["failed", "completed"]).await;
    assert_eq!(m.status, "failed");
    let done: Vec<_> = m.tasks.iter().filter(|t| t.status == "done").collect();
    let blocked: Vec<_> = m.tasks.iter().filter(|t| t.status == "blocked").collect();
    assert_eq!((done.len(), blocked.len()), (1, 1), "{m:#?}");
    assert_eq!(blocked[0].conflicts, vec!["src/a.ts".to_string()]);
    assert!(blocked[0].blocker.as_deref().unwrap().contains("nothing was overwritten"));
    let live = env.g.workspace(&env.pid).unwrap().read_text("src/a.ts").unwrap();
    let winner = if done[0].assignee_agent_id.as_deref() == Some(a.id.as_str()) { "alpha" } else { "beta" };
    assert!(live.contains(winner), "the integrated task's content stays: {live}");
    // approving a conflicted task is refused; retry re-runs it on top of the current project
    assert!(env.g.approve_mission_task(&blocked[0].id, true).is_err());
    env.g.retry_mission_task(&blocked[0].id).unwrap();
    let t = wait_task(&env.g, &blocked[0].id, &["done", "blocked", "failed"]).await;
    assert_eq!(t.status, "done", "retry on the current project integrates: {t:#?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_cancel_and_restart_persistence() {
    let ids = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let ids2 = ids.clone();
    let env = setup(move |sys, n, _l| match who(sys) {
        "coord" => {
            let v = ids2.lock().unwrap().clone();
            plan(json!([
                { "id": "t1", "title": "Alpha edits a", "assignee": v[0] },
                { "id": "t2", "title": "Beta edits b", "assignee": v[1], "dependsOn": ["t1"] },
                { "id": "t3", "title": "Beta later", "assignee": v[1], "dependsOn": ["t2"] }
            ]))
        }
        "alpha" if n == 0 => call("write_file", json!({ "path": "src/a.ts", "content": "export const a = 'alpha';\n" })),
        "beta" if n == 0 => call("write_file", json!({ "path": "src/b.ts", "content": "export const b = 'beta';\n" })),
        _ => "<final>done</final>".into(),
    })
    .await;
    let (a, b, t) = team(&env, "auto", "ask");
    *ids.lock().unwrap() = vec![a.id.clone(), b.id.clone()];
    let m = env.g.create_mission(&env.pid, "Two step", Some(&t.id)).unwrap();
    let m = wait_mission(&env.g, &m.id, &["ready"]).await;
    env.g.start_mission(&m.id).unwrap();
    let m = {
        let t2 = &m.tasks[1].id;
        wait_task(&env.g, t2, &["awaiting_approval"]).await;
        env.g.get_mission(&m.id).unwrap()
    };
    let ws = env.g.workspace(&env.pid).unwrap();
    assert!(ws.read_text("src/a.ts").unwrap().contains("alpha"));
    assert_eq!(ws.read_text("src/b.ts").unwrap(), "export const b = 1;\n", "ask: the live project is untouched until approval");
    assert_eq!(m.tasks[1].changed_paths, vec!["src/b.ts".to_string()]);
    assert_eq!(m.tasks[2].status, "pending");
    assert!(m.tasks[1].changeset_id.is_some());

    // restart: the mission and the pending approval survive
    let data = env.g.data_dir().to_path_buf();
    let (tid, mid) = (m.tasks[1].id.clone(), m.id.clone());
    drop(ws);
    let g2 = Genius::new(&data, Arc::new(RecordingEmitter::default()), Arc::new(MemoryStore::default())).unwrap();
    let m2 = g2.get_mission(&mid).unwrap();
    g2.save_api_key("nvapi-agent-test-key-0123456789").unwrap();
    assert_eq!(m2.status, "paused");
    assert_eq!(m2.tasks[1].status, "awaiting_approval");
    assert_eq!(m2.tasks[0].status, "done");
    g2.resume_mission(&mid).unwrap();
    g2.approve_mission_task(&tid, true).unwrap();
    wait_task(&g2, &tid, &["done"]).await;
    assert!(g2.workspace(&env.pid).unwrap().read_text("src/b.ts").unwrap().contains("beta"));

    // cancel: completed work is preserved, the rest is cancelled
    let m3 = wait_task(&g2, &m2.tasks[2].id, &["awaiting_approval", "running", "ready"]).await;
    let _ = m3;
    let c = g2.cancel_mission(&mid).unwrap();
    assert_eq!(c.status, "cancelled");
    let after = wait_task(&g2, &m2.tasks[2].id, &["cancelled"]).await;
    assert_eq!(after.status, "cancelled");
    let c = g2.get_mission(&mid).unwrap();
    assert_eq!(c.tasks[0].status, "done");
    assert_eq!(c.tasks[1].status, "done");
    assert!(g2.workspace(&env.pid).unwrap().read_text("src/a.ts").unwrap().contains("alpha"));
    assert!(c.summary.unwrap().contains("Not completed"));
}
