//! Agent integration tests. The NVIDIA API is replaced by a scripted mock (wiremock) that
//! plays the model's side of the tool protocol; everything else — conversation storage, tool
//! execution against a real project folder, sandboxing, subagents, changesets, reports — is
//! the real implementation. These tests do NOT verify live NVIDIA behaviour.

use genius_core::agent::{AgentMode, AgentRequest};
use genius_core::app::Genius;
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::secrets::MemoryStore;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(text: &str) -> ResponseTemplate {
    // A reply starting with "@@LENGTH@@" simulates output cut off by the token limit.
    let (text, finish) = match text.strip_prefix("@@LENGTH@@") {
        Some(t) => (t, "length"),
        None => (text, "stop"),
    };
    let mut s = String::new();
    for chunk in text.chars().collect::<Vec<_>>().chunks(29) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str(&format!("data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"{finish}\"}}]}}\n\ndata: [DONE]\n\n"));
    ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
}

type Script = dyn Fn(&str, &str, &[Value]) -> String + Send + Sync;

/// Calls `script(system, last_message, all_messages)` and records every request body.
struct Scripted {
    script: Box<Script>,
    log: Arc<Mutex<Vec<Value>>>,
}

impl Respond for Scripted {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        self.log.lock().unwrap().push(body.clone());
        let msgs = body["messages"].as_array().unwrap().clone();
        let system = msgs[0]["content"].as_str().unwrap_or("").to_string();
        let last = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        sse(&(self.script)(&system, &last, &msgs))
    }
}

const APP: &str = "import './App.css';\n\nexport default function App() {\n  return <main className=\"app\"><h1>Dashboard</h1></main>;\n}\n";

async fn setup(script: impl Fn(&str, &str, &[Value]) -> String + Send + Sync + 'static) -> (tempfile::TempDir, Arc<Genius>, String, Arc<Mutex<Vec<Value>>>, MockServer) {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { script: Box::new(script), log: log.clone() }).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/coder-1".into());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-agent-test-key-0123456789").unwrap();
    let p = g.create_project("Dashboard", "A modern dashboard").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", r#"{"name":"dash","scripts":{"build":"tsc && vite build"},"dependencies":{"react":"^19.0.0"}}"#).unwrap();
    ws.write_text("src/App.tsx", APP).unwrap();
    ws.write_text("src/App.css", ".app { padding: 1rem; }\n").unwrap();
    std::fs::write(ws.root().join(".env"), "API_TOKEN=SECRET_VALUE_123\n").unwrap();
    (tmp, g, p.id, log, server)
}

async fn run(g: &Arc<Genius>, pid: &str, conv: &str, msg: &str, mode: AgentMode) -> Value {
    let t = g.start_agent(pid, AgentRequest { conversation_id: conv.into(), message: msg.into(), mode, ..Default::default() }).unwrap();
    for _ in 0..400 {
        if g.active_tasks(Some(pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ev = g.bus.list(Some(pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).unwrap();
    json!({ "taskId": t.id, "status": ev.status, "detail": ev.detail })
}

fn file_block(p: &str, c: &str) -> String {
    format!("<<<FILE: {p}>>>\n{c}<<<END FILE>>>\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn continuous_conversation_inspects_then_proposes_targeted_changes() {
    let (_tmp, g, pid, log, _server) = setup(|system, last, msgs| {
        if system.contains("Reviewer subagent") {
            assert!(last.contains("DIFF:"), "reviewer receives the diff");
            return "<final>[note] Sidebar has an aria-label. No blocking issues found.</final>".into();
        }
        let all: String = msgs.iter().map(|m| m["content"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n");
        if last.contains("USER MESSAGE:\nAdd a sidebar") {
            return "I'll find the layout first.\n<tool>{\"name\":\"search\",\"args\":{\"query\":\"Dashboard\"}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"search\"") {
            assert!(last.contains("src/App.tsx:4"), "real search result: {last}");
            return "<tool>{\"name\":\"read_file\",\"args\":{\"path\":\"src/App.tsx\"}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"read_file\"") {
            assert!(last.contains("<h1>Dashboard</h1>"), "real file content");
            let app = "import './App.css';\nimport Sidebar from './components/Sidebar';\n\nexport default function App() {\n  return <div className=\"layout\"><Sidebar /><main className=\"app\"><h1>Dashboard</h1></main></div>;\n}\n";
            let sidebar = "export default function Sidebar() {\n  return <nav aria-label=\"Main\" className=\"sidebar\">Menu</nav>;\n}\n";
            return format!("<final>Added a Sidebar component and placed it beside the main content.\n{}{}</final>", file_block("src/App.tsx", app), file_block("src/components/Sidebar.tsx", sidebar));
        }
        if last.contains("USER MESSAGE:\nMake the sidebar darker") {
            // Follow-up: the earlier request and the proposal's real status are in context.
            assert!(all.contains("Add a sidebar"), "conversation history is included");
            assert!(all.contains("files changed in this turn (proposed): src/App.tsx, src/components/Sidebar.tsx"), "proposal status comes from Genius records");
            return "<final>Should the darker sidebar use your existing dark palette (#111) or a new colour?</final>".into();
        }
        panic!("unexpected turn: {last}");
    })
    .await;

    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let r = run(&g, &pid, &conv.id, "Add a sidebar", AgentMode::Build).await;
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}");
    assert_eq!(r["detail"]["result"]["kind"], "proposal");
    let cs_id = r["detail"]["result"]["changesetId"].as_str().unwrap().to_string();

    // Nothing applied before review; unrelated file untouched.
    let ws = g.workspace(&pid).unwrap();
    assert_eq!(ws.read_text("src/App.tsx").unwrap(), APP);
    assert!(!ws.exists("src/components/Sidebar.tsx"));
    let (cs, files) = g.changeset(&cs_id).unwrap();
    assert_eq!(cs.status, "proposed");
    assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["src/App.tsx", "src/components/Sidebar.tsx"]);

    // Provenance: main agent steps + reviewer subagent recorded.
    let steps = g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    let tools: Vec<(&str, &str, bool)> = steps.iter().map(|s| (s.agent.as_str(), s.tool.as_str(), s.ok)).collect();
    assert_eq!(tools, vec![("main", "search", true), ("main", "read_file", true)]);
    assert!(!g.bus.list(Some(&pid), 3000).unwrap().iter().any(|e| e.title.starts_with("reviewer subagent")), "small proposals skip the reviewer");

    // Conversation persisted with distinct kinds and an evidence-based report.
    let msgs = g.db.list_conversation_messages(&conv.id, 50).unwrap();
    assert_eq!(msgs.iter().map(|m| (m.role.as_str(), m.kind.as_str())).collect::<Vec<_>>(), vec![("user", "chat"), ("assistant", "proposal")]);
    let report = &msgs[1].meta["report"];
    assert_eq!(report["filesRead"], json!(["src/App.tsx"]));
    assert_eq!(report["changeset"]["status"], "proposed");
    assert!(report["notVerified"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("No build or test")));
    assert_eq!(g.db.get_conversation(&conv.id).unwrap().title, "Add a sidebar", "titled from the first message");

    // Follow-up in the same conversation.
    let r2 = run(&g, &pid, &conv.id, "Make the sidebar darker", AgentMode::Build).await;
    assert_eq!(r2["detail"]["result"]["kind"], "answer");
    assert_eq!(g.db.list_conversation_messages(&conv.id, 50).unwrap().len(), 4);

    // Restart: conversation and messages survive.
    let data = g.data_dir().to_path_buf();
    drop(g);
    let g2 = Genius::new(&data, Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    assert_eq!(g2.db.list_conversations(&pid).unwrap()[0].message_count, 4);

    // The .env secret never reached the model.
    for body in log.lock().unwrap().iter() {
        assert!(!body.to_string().contains("SECRET_VALUE_123"), "secret leaked to the provider");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tools_are_sandboxed_validated_and_deduplicated() {
    let (_tmp, g, pid, log, _server) = setup(|_system, last, msgs| {
        let n = msgs.iter().filter(|m| m["content"].as_str().unwrap_or("").starts_with("<tool_result")).count();
        match n {
            0 => "<tool>{\"name\":\"read_file\",\"args\":{\"path\":\".env\"}}</tool>".into(),
            1 => {
                assert!(last.contains("ok=\"false\""), "{last}");
                "<tool>{\"name\":\"read_file\",\"args\":{\"path\":\"../../etc/passwd\"}}</tool>".into()
            }
            2 => "<tool>{\"name\":\"run_shell\",\"args\":{\"cmd\":\"rm -rf /\"}}</tool>".into(),
            3 => {
                assert!(last.contains("unknown tool"));
                "<tool>{\"name\":\"read_file\",\"args\":{\"path\":\"src/App.tsx\",\"evil\":1}}</tool>".into()
            }
            4 => "<tool>{\"name\":\"project_facts\",\"args\":{}}</tool>".into(),
            5 => "<tool>{\"name\":\"project_facts\",\"args\":{}}</tool>".into(),
            6 => {
                assert!(last.contains("identical arguments"), "duplicate suppressed: {last}");
                "<tool>not json</tool>".into()
            }
            _ => "<final>All tests pass and the build succeeded.</final>".into(),
        }
    })
    .await;
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let r = run(&g, &pid, &conv.id, "Check the project", AgentMode::Build).await;
    assert_eq!(r["detail"]["result"]["kind"], "answer", "{r}");
    let steps = g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    let oks: Vec<(String, bool)> = steps.iter().map(|s| (s.tool.clone(), s.ok)).collect();
    assert_eq!(
        oks,
        // unknown argument fields are ignored (a rejected call costs a model round-trip)
        vec![("read_file".into(), false), ("read_file".into(), false), ("run_shell".into(), false), ("read_file".into(), true), ("project_facts".into(), true), ("project_facts".into(), false)]
    );
    assert!(steps[0].summary.contains("excluded") || steps[0].summary.contains("secret"), "{}", steps[0].summary);
    // Honest report: the model claimed tests passed, but nothing ran.
    let msgs = g.db.list_conversation_messages(&conv.id, 10).unwrap();
    let warnings = msgs[1].meta["report"]["warnings"].as_array().unwrap();
    assert!(warnings[0].as_str().unwrap().contains("no successful build or test ran"), "{warnings:?}");
    for body in log.lock().unwrap().iter() {
        let s = body.to_string();
        assert!(!s.contains("SECRET_VALUE_123"));
        assert!(!s.contains("root:x:0"), "no file outside the workspace was read");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn loop_is_bounded_and_plan_mode_never_proposes_files() {
    let calls = Arc::new(Mutex::new(0usize));
    let c2 = calls.clone();
    let (_tmp, g, pid, _log, _server) = setup(move |_system, last, _msgs| {
        *c2.lock().unwrap() += 1;
        if last.contains("step limit") {
            return "<final>I ran out of steps; nothing was changed.</final>".into();
        }
        if last.contains("USER MESSAGE:\nPlan it") {
            return format!("<final>Plan: add a header.\n{}</final>", file_block("src/Header.tsx", "export {}\n"));
        }
        let n = *c2.lock().unwrap();
        format!("<tool>{{\"name\":\"list_files\",\"args\":{{\"prefix\":\"src/{n}\"}}}}</tool>")
    })
    .await;
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let t = g.start_agent(&pid, AgentRequest { conversation_id: conv.id.clone(), message: "Explore forever".into(), effort: Some("fast".into()), ..Default::default() }).unwrap();
    for _ in 0..400 {
        if g.active_tasks(Some(&pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(g.agent_steps(&t.id).unwrap().len(), 24, "bounded at 24 turns on fast effort");
    assert_eq!(*calls.lock().unwrap(), 25, "24 tool turns + 1 forced final");

    let r = run(&g, &pid, &conv.id, "Plan it", AgentMode::Plan).await;
    assert_eq!(r["detail"]["result"]["kind"], "plan");
    assert!(g.list_changesets(&pid).unwrap().is_empty(), "plan mode proposes nothing");
    assert!(!g.workspace(&pid).unwrap().exists("src/Header.tsx"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capabilities_reflect_runtime_state() {
    let (_tmp, g, pid, _log, _server) = setup(|_s, last, _m| {
        if last.starts_with("<tool_result name=\"capabilities\"") {
            assert!(last.contains("[unverified] Chat with a model"), "model not probed yet: {last}");
            assert!(last.contains("[unverified] Image input"));
            assert!(last.contains("supabase"));
            return "<final>I can read and change this project; the model has not been verified yet.</final>".into();
        }
        "<tool>{\"name\":\"capabilities\",\"args\":{}}</tool>".into()
    })
    .await;
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let r = run(&g, &pid, &conv.id, "What can you do?", AgentMode::Build).await;
    assert_eq!(r["detail"]["result"]["kind"], "answer", "{r}");
    let rep = g.capability_report_quick(Some(&pid)).unwrap();
    let get = |id: &str| rep.capabilities.iter().find(|c| c.id == id).unwrap().status.clone();
    assert_eq!(get("provider_key"), "available");
    assert_eq!(get("chat"), "unverified");
    assert_eq!(get("shell"), "unavailable");
    g.remove_api_key().unwrap();
    let rep = g.capability_report_quick(Some(&pid)).unwrap();
    assert_eq!(rep.capabilities.iter().find(|c| c.id == "provider_key").unwrap().status, "unavailable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn large_builds_stage_files_track_progress_and_ask_questions() {
    let (_tmp, g, pid, _log, _server) = setup(|system, last, _msgs| {
        if system.contains("Reviewer subagent") {
            return "<final>No blocking issues found.</final>".into();
        }
        if last.contains("USER MESSAGE:\nMake me a website") {
            assert!(system.contains("You are Genius, the AI software engineer built into the Project Genius desktop app"), "identity");
            assert!(system.contains("mock/coder-1"), "actual model id in the prompt");
            assert!(system.contains("DESIGN GUIDE") && system.contains("<ask>"));
            return "<final>Happy to! One quick question first.\n<ask>{\"question\": \"What is the website for?\", \"options\": [\"Portfolio\", \"Restaurant\", \"  \", \"Portfolio\"], \"multi\": false}</ask></final>".into();
        }
        if last.contains("USER MESSAGE:\nA restaurant") {
            return "I'll lay out the plan first.\n<tool>{\"name\":\"todo\",\"args\":{\"items\":[{\"text\":\"Design tokens\",\"status\":\"active\"},{\"text\":\"Pages\",\"status\":\"pending\"}]}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"todo\"") {
            // Staged file content mentions <tool> — only the trailing call counts.
            return format!("Writing the tokens and the menu page.\n{}{}<tool>{{\"name\":\"write_files\",\"args\":{{}}}}</tool>", file_block("src/index.css", ":root { --accent: #c2410c; }\n"), file_block("src/pages/Menu.tsx", "// renders <tool> text safely\nexport default function Menu() {\n  return <h1>Menu</h1>;\n}\n"));
        }
        if last.starts_with("<tool_result name=\"write_files\"") {
            assert!(last.contains("saved 2 file(s): src/index.css (file block), src/pages/Menu.tsx (file block)"), "{last}");
            return "<tool>{\"name\":\"read_file\",\"args\":{\"path\":\"src/pages/Menu.tsx\"}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"read_file\"") {
            assert!(last.contains("includes your edits from this task") && last.contains("<h1>Menu</h1>"), "staged files are readable: {last}");
            return "<tool>{\"name\":\"web_search\",\"args\":{\"query\":\"restaurant website ideas\"}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"web_search\"") {
            assert!(last.contains("Internet access is turned off"), "{last}");
            return format!("<final>Built the restaurant site.\n{}<ask>{{\"question\": \"Add online booking next?\", \"options\": [\"Yes\", \"No\"]}}</ask></final>", file_block("src/index.css", ":root { --accent: #b45309; }\n"));
        }
        panic!("unexpected turn: {last}");
    })
    .await;
    let mut s = g.settings().unwrap();
    s.internet_access = false;
    g.save_settings(&s).unwrap();
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();

    // 1. Ambiguous request → structured question, no tools, no changes.
    let r = run(&g, &pid, &conv.id, "Make me a website", AgentMode::Build).await;
    assert_eq!(r["detail"]["result"]["kind"], "question", "{r}");
    let msgs = g.db.list_conversation_messages(&conv.id, 10).unwrap();
    assert_eq!(msgs[1].kind, "question");
    assert_eq!(msgs[1].meta["ask"], json!({ "question": "What is the website for?", "options": ["Portfolio", "Restaurant"], "multi": false }));
    assert_eq!(msgs[1].content, "Happy to! One quick question first.");

    // 2. Staged batch + final files → one changeset (final wins), todo + narration recorded.
    let r = run(&g, &pid, &conv.id, "A restaurant", AgentMode::Build).await;
    assert_eq!(r["detail"]["result"]["kind"], "proposal", "{r}");
    let (_, files) = g.changeset(r["detail"]["result"]["changesetId"].as_str().unwrap()).unwrap();
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, vec!["src/index.css", "src/pages/Menu.tsx"]);
    assert!(files.iter().find(|f| f.path == "src/index.css").unwrap().after.as_deref().unwrap().contains("#b45309"), "final answer wins over staged");
    let steps = g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    let main: Vec<(&str, bool)> = steps.iter().filter(|s| s.agent == "main").map(|s| (s.tool.as_str(), s.ok)).collect();
    assert_eq!(main, vec![("todo", true), ("write_files", true), ("read_file", true), ("web_search", false)]);
    assert_eq!(steps[0].args["items"][0]["text"], "Design tokens");
    assert_eq!(steps[0].result["note"], "I'll lay out the plan first.");
    assert_eq!(steps[1].result["note"], "Writing the tokens and the menu page.");
    let msgs = g.db.list_conversation_messages(&conv.id, 10).unwrap();
    assert_eq!(msgs[3].kind, "proposal");
    assert_eq!(msgs[3].meta["ask"]["question"], "Add online booking next?");
    assert!(!msgs[3].content.contains("<ask>"));
    assert!(!g.workspace(&pid).unwrap().exists("src/pages/Menu.tsx"), "nothing written before review");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn output_cut_off_mid_file_is_staged_and_continued() {
    let (_tmp, g, pid, _log, _server) = setup(|system, last, _msgs| {
        if system.contains("Reviewer subagent") {
            return "<final>No blocking issues found.</final>".into();
        }
        if last.contains("USER MESSAGE:\nBuild the landing page") {
            return format!("@@LENGTH@@<final>Here is the site.\n{}<<<FILE: src/Hero.tsx>>>\nexport default function Hero() {{\n  return <se", file_block("src/tokens.css", ":root { --space: 8px; }\n"));
        }
        if last.contains("cut off by the length limit") {
            assert!(last.contains("src/tokens.css") && last.contains("must be written again in full: src/Hero.tsx"), "{last}");
            return format!("<final>Landing page ready.\n{}</final>", file_block("src/Hero.tsx", "export default function Hero() {\n  return <section>Hi</section>;\n}\n"));
        }
        panic!("unexpected turn: {last}");
    })
    .await;
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let r = run(&g, &pid, &conv.id, "Build the landing page", AgentMode::Build).await;
    assert_eq!(r["detail"]["result"]["kind"], "proposal", "{r}");
    let (_, files) = g.changeset(r["detail"]["result"]["changesetId"].as_str().unwrap()).unwrap();
    let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, vec!["src/Hero.tsx", "src/tokens.css"], "complete file from the cut-off turn is kept");
}

/// Reproduces the failure seen in a real session (write_files called without file blocks, the
/// app crashing after the change) and drives the new loop: several calls per turn, code-fence
/// fallback, edit_file, autopilot checkpoints, a failing type check handed back and fixed,
/// a message steered in mid-run, then rewinding the conversation restores the files.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn autopilot_edits_verifies_fixes_and_rewinds() {
    let genius: Arc<std::sync::OnceLock<Arc<Genius>>> = Arc::new(std::sync::OnceLock::new());
    let gh = genius.clone();
    let (_tmp, g, pid, log, _server) = setup(move |_system, last, msgs| {
        let all: String = msgs.iter().map(|m| m["content"].as_str().unwrap_or("")).collect::<Vec<_>>().join("\n");
        if last.contains("USER MESSAGE:\nAdd a stats widget") {
            assert!(last.contains("<<<CURRENT: src/App.tsx>>>"), "small projects arrive as a snapshot");
            // The user sends a follow-up while Genius works.
            let g = gh.get().unwrap();
            let task = g.active_tasks(None).into_iter().find(|t| t.kind == "agent").unwrap();
            g.steer_agent(&task.id, "Also call the widget Stats").unwrap();
            let fence = "**src/widget.css**\n```css\n.widget { padding: 8px; }\n```\n";
            return format!(
                "Adding the widget.\n{fence}<tool>{{\"name\":\"write_files\",\"args\":{{}}}}</tool>\n<tool>{{\"name\": \"write_file\", \"args\": {{\"path\": \"src/Widget.tsx\", \"content\": \"import './widget.css';\nexport default function Widget() {{\n  return <div className=\\\"widget\\\">Stats</div>;\n}}\n\"}}}}</tool>\n<tool_call>{{\"name\": \"edit_file\", \"arguments\": {{\"path\": \"src/App.tsx\", \"old_string\": \"<h1>Dashboard</h1>\", \"new_string\": \"<h1>Dashboard</h1><Widget />BROKEN\"}}}}</tool_call>"
            );
        }
        if last.contains("NEW MESSAGE FROM THE USER") {
            // Injected right after the results of the first turn's three calls.
            assert!(last.contains("Also call the widget Stats"));
            let results = msgs[msgs.len() - 2]["content"].as_str().unwrap();
            assert!(results.contains("src/widget.css (1 lines, from a code fence)"), "{results}");
            assert!(results.contains("<tool_result name=\"edit_file\" ok=\"true\">"), "{results}");
            assert!(results.contains("[Genius] Applied to the project (checkpoint): src/App.tsx"), "{results}");
            return "It's already called Stats.\n<final>Added the Stats widget.</final>".into();
        }
        if last.starts_with("AUTOPILOT CHECK FAILED") {
            assert!(last.contains("TS2304") && last.contains("BROKEN"), "real check output: {last}");
            return "Fixing the stray text.\n<tool>{\"name\":\"edit_file\",\"args\":{\"path\":\"src/App.tsx\",\"edits\":[{\"old\":\"<Widget />BROKEN\",\"new\":\"<Widget />\"},{\"old\":\"import './App.css';\",\"new\":\"import './App.css';\\nimport Widget from './Widget';\"}]}}</tool>".into();
        }
        if last.contains("<tool_result name=\"edit_file\"") {
            return "<final>Added the Stats widget and fixed the type error.</final>".into();
        }
        panic!("unexpected turn: {last}\n---\n{all}");
    })
    .await;
    let _ = genius.set(g.clone());
    let ws = g.workspace(&pid).unwrap();
    ws.write_text("package.json", r#"{"name":"dash","scripts":{"typecheck":"node check.js"}}"#).unwrap();
    ws.write_text("check.js", "const s = require('fs').readFileSync('src/App.tsx', 'utf8');\nif (s.includes('BROKEN')) { console.error(\"src/App.tsx(4,60): error TS2304: Cannot find name 'BROKEN'.\"); process.exit(2); }\n").unwrap();
    std::fs::create_dir_all(ws.root().join("node_modules")).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(ws.root().join("node_modules/.package-lock.json"), "{}").unwrap();
    let mut s = g.settings().unwrap();
    s.approvals.apply_ai_edits = genius_core::settings::Policy::Auto;
    g.save_settings(&s).unwrap();
    // The modified typecheck script needs the user's approval: approve it like the user would.
    let ga = g.clone();
    let approver = tokio::spawn(async move {
        loop {
            for a in ga.approvals.pending(None) {
                ga.approvals.respond(&a.id, true).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });

    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();
    let r = run(&g, &pid, &conv.id, "Add a stats widget", AgentMode::Build).await;
    approver.abort();
    assert_eq!(r["detail"]["result"]["kind"], "proposal", "{r}");
    assert_eq!(r["detail"]["result"]["applied"], true);
    let app = ws.read_text("src/App.tsx").unwrap();
    assert!(app.contains("import Widget from './Widget';") && app.contains("<Widget />") && !app.contains("BROKEN"), "{app}");
    assert_eq!(ws.read_text("src/widget.css").unwrap(), ".widget { padding: 8px; }\n");
    assert!(ws.exists("src/Widget.tsx"));

    let msgs = g.db.list_conversation_messages(&conv.id, 20).unwrap();
    assert_eq!(msgs.iter().map(|m| (m.role.as_str(), m.content.as_str())).collect::<Vec<_>>()[..2], [("user", "Add a stats widget"), ("user", "Also call the widget Stats")], "steered message is part of the conversation");
    let answer = msgs.last().unwrap();
    assert_eq!(answer.content, "Added the Stats widget and fixed the type error.");
    let checks = answer.meta["report"]["autopilot"]["checks"].as_array().unwrap();
    assert_eq!(checks.iter().map(|c| c["ok"].clone()).collect::<Vec<_>>(), vec![json!(false), json!(true)], "failed check handed back, then passed");
    assert_eq!(answer.meta["changesetIds"].as_array().unwrap().len(), 2, "one checkpoint per editing turn");
    let steps = g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    let tools: Vec<(&str, bool, i64)> = steps.iter().map(|s| (s.tool.as_str(), s.ok, s.result["turn"].as_i64().unwrap())).collect();
    assert_eq!(tools, vec![("write_files", true, 1), ("write_file", true, 1), ("edit_file", true, 1), ("run_check", false, 2), ("edit_file", true, 3), ("run_check", true, 4)]);
    assert_eq!(steps[1].args["content"].as_str().unwrap().lines().count(), 4, "short content kept in step args");

    // Rewind to the first message: files return to their original state.
    let first = msgs[0].id.clone();
    let rw = g.rewind_conversation(&conv.id, &first, true).unwrap();
    assert_eq!(rw["revertedChangesets"], 2);
    assert_eq!(ws.read_text("src/App.tsx").unwrap(), APP);
    assert!(!ws.exists("src/Widget.tsx") && !ws.exists("src/widget.css"));
    assert!(g.db.list_conversation_messages(&conv.id, 20).unwrap().is_empty());
    for body in log.lock().unwrap().iter() {
        assert!(!body.to_string().contains("SECRET_VALUE_123"));
    }
}
