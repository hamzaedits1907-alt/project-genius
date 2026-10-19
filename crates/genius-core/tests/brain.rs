//! Project Brain at the Genius level: persistence across restarts, incremental refresh, recorded
//! knowledge kept apart from verified facts, project isolation, scoped visibility, and the
//! `project_brain` tool as a scripted model would use it. Real files and a real git repository;
//! the model is a scripted stand-in.

use genius_core::agent::{AgentMode, AgentRequest};
use genius_core::app::Genius;
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::secrets::MemoryStore;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn genius(dir: &std::path::Path) -> Arc<Genius> {
    let g = Genius::new(&dir.join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.workspace_dir = Some(dir.join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    g
}

fn seed(g: &Arc<Genius>, name: &str) -> String {
    let p = g.create_project(name, "brain test").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", r#"{"name":"shop","scripts":{"build":"tsc && vite build","test":"vitest run"},"dependencies":{"react":"^19","vite":"^7"}}"#).unwrap();
    ws.write_text("src/main.tsx", "import App from './App';\nrender(<App />);\n").unwrap();
    ws.write_text("src/App.tsx", "import { formatPrice } from './lib/price';\nexport default function App() {\n  return <b>{formatPrice(3)}</b>;\n}\n").unwrap();
    ws.write_text("src/lib/price.ts", "export function formatPrice(n: number) {\n  return `$${n.toFixed(2)}`;\n}\n").unwrap();
    ws.write_text("src/lib/price.test.ts", "import { formatPrice } from './price';\ntest('fmt', () => expect(formatPrice(1)).toBe('$1.00'));\n").unwrap();
    p.id
}

fn all(_: &str) -> bool {
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_index_persists_refreshes_incrementally_and_survives_a_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let pid = seed(&g, "Shop");
    let (_, first) = g.brain_refresh(&pid).unwrap();
    assert_eq!((first.scanned, first.reused), (4, 0));
    let (_, again) = g.brain_refresh(&pid).unwrap();
    assert_eq!((again.scanned, again.reused), (0, 4), "second refresh re-reads nothing");

    let data = g.data_dir().to_path_buf();
    drop(g);
    let g2 = Genius::new(&data, Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let (_, after_restart) = g2.brain_refresh(&pid).unwrap();
    assert_eq!((after_restart.scanned, after_restart.reused), (0, 4), "the stored index is reused after a restart");

    g2.workspace(&pid).unwrap().write_text("src/lib/price.ts", "export function formatPrice(n: number) { return String(n); }\nexport function taxOf(n: number) { return n * 0.2; }\n").unwrap();
    let (_, changed) = g2.brain_refresh(&pid).unwrap();
    assert_eq!((changed.scanned, changed.reused), (1, 3));
    let o = g2.brain_overview(&pid).unwrap();
    assert_eq!(o.project_type, ["React (Vite)"]);
    assert!(o.commands.iter().any(|c| c.command == "npm run test"));
    assert!(g2.brain_prompt(&pid).contains("Entry points: src/main.tsx"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verified_facts_and_recorded_knowledge_are_separate_and_staleness_is_flagged() {
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let pid = seed(&g, "Shop");
    g.db.add_memory(&pid, "decision", "Prices are formatted in src/lib/price.ts to keep rounding in one place", "user", "").unwrap();
    g.db.add_memory(&pid, "note", "formatPrice used to live in src/utils/money.ts", "agent", "").unwrap();
    g.db.add_memory(&pid, "note", "Unrelated: the logo is blue", "agent", "").unwrap();

    let a = g.brain_ask(&pid, "where is formatPrice", &["src/lib/price.ts".into()], &all).await.unwrap();
    assert_eq!((a.hits[0].name.as_str(), a.hits[0].path.as_str(), a.hits[0].line), ("formatPrice", "src/lib/price.ts", 1));
    assert!(a.excerpts[0].text.contains("toFixed"), "the excerpt is the real code: {:?}", a.excerpts[0]);
    let impact = a.impact.as_ref().unwrap();
    assert!(impact.affected.iter().any(|x| x.path == "src/App.tsx" && x.depth == 1) && impact.affected.iter().any(|x| x.path == "src/main.tsx" && x.depth == 2));
    assert_eq!(impact.tests, ["src/lib/price.test.ts"]);

    assert_eq!(a.notes.len(), 2, "only memory relevant to the question is returned: {:?}", a.notes);
    let user_note = a.notes.iter().find(|n| n.origin == "user-stated").unwrap();
    assert!(user_note.stale.is_empty());
    let old = a.notes.iter().find(|n| n.origin == "agent inference").unwrap();
    assert!(old.stale[0].contains("src/utils/money.ts"), "memory pointing at a file that is gone is flagged: {old:?}");

    let text = a.render(8000);
    let verified = text.find("VERIFIED SOURCE FACTS").unwrap();
    let recorded = text.find("RECORDED KNOWLEDGE").unwrap();
    assert!(verified < recorded);
    assert!(text[..recorded].contains("formatPrice") && !text[..recorded].contains("money.ts"), "recorded claims never appear in the verified section");
    assert!(text[recorded..].contains("possibly stale"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn git_history_and_earlier_changes_are_attached_to_the_files_concerned() {
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let pid = seed(&g, "Shop");
    let root = g.workspace(&pid).unwrap().root().to_path_buf();
    let git = |args: &[&str]| {
        let o = std::process::Command::new("git").args(["-c", "user.name=T", "-c", "user.email=t@example.com"]).args(args).current_dir(&root).output().unwrap();
        assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "Format prices with two decimals"]);
    let a = g.brain_ask(&pid, "formatPrice", &[], &all).await.unwrap();
    assert!(a.git_history.iter().any(|h| h.path == "src/lib/price.ts" && h.subject == "Format prices with two decimals"), "{:?}", a.git_history);
    assert!(a.render(4000).contains("git "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn projects_never_see_each_other_and_scope_hides_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let a = seed(&g, "Shop");
    let b = g.create_project("Other", "other").unwrap().id;
    g.workspace(&b).unwrap().write_text("src/rocket.ts", "export function launchRocket() {}\n").unwrap();
    g.db.add_memory(&b, "note", "launchRocket is wired in src/rocket.ts", "user", "").unwrap();

    let in_a = g.brain_ask(&a, "launchRocket", &[], &all).await.unwrap();
    assert!(in_a.hits.is_empty() && in_a.notes.is_empty(), "project A knows nothing about project B");
    let in_b = g.brain_ask(&b, "launchRocket", &[], &all).await.unwrap();
    assert_eq!(in_b.hits[0].name, "launchRocket");

    let only_lib = |p: &str| p.starts_with("src/lib/");
    let scoped = g.brain_ask(&a, "formatPrice App", &["src/lib/price.ts".into(), "src/App.tsx".into()], &only_lib).await.unwrap();
    assert!(scoped.hits.iter().all(|h| h.path.starts_with("src/lib/")));
    let i = scoped.impact.unwrap();
    assert_eq!(i.changed, ["src/lib/price.ts"], "a path outside the scope is not even acknowledged");
    assert!(i.affected.iter().all(|x| x.path.starts_with("src/lib/")));
}

// ------------------------------------------------------------------- the tool, used by a model

struct Scripted {
    log: Arc<Mutex<Vec<Value>>>,
}
impl Respond for Scripted {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let msgs = body["messages"].as_array().unwrap().clone();
        self.log.lock().unwrap().push(body);
        let last = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let text = if last.starts_with("<tool_result name=\"project_brain\"") {
            assert!(last.contains("VERIFIED SOURCE FACTS") && last.contains("src/lib/price.ts:1"), "{last}");
            assert!(last.contains("CHANGE IMPACT") && last.contains("src/App.tsx"), "{last}");
            "<final>formatPrice is in src/lib/price.ts; App.tsx and main.tsx depend on it.</final>".to_string()
        } else {
            "I'll ask the Project Brain.\n<tool>{\"name\":\"project_brain\",\"args\":{\"query\":\"formatPrice\",\"impact\":[\"src/lib/price.ts\"]}}</tool>".to_string()
        };
        let sse = format!("data: {}\n\ndata: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n", json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":null}]}));
        ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream")
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_agent_gets_a_brain_overview_in_its_prompt_and_can_call_the_tool() {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { log: log.clone() }).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/coder-1".into());
    s.model_mode = "manual".into();
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-brain-test-key-0123456789").unwrap();
    let pid = seed(&g, "Shop");
    let conv = g.db.create_conversation(&pid, "Q").unwrap();
    let t = g.start_agent(&pid, AgentRequest { conversation_id: conv.id.clone(), message: "What would changing formatPrice affect?".into(), mode: AgentMode::Plan, ..Default::default() }).unwrap();
    for _ in 0..400 {
        if g.active_tasks(Some(&pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let ev = g.bus.list(Some(&pid), 3000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).unwrap();
    assert_eq!(ev.status, EventStatus::Completed, "{ev:?}");
    let steps = g.agent_steps(&t.id).unwrap();
    assert_eq!(steps.iter().map(|s| (s.tool.as_str(), s.ok)).collect::<Vec<_>>(), [("project_brain", true)]);
    let first = log.lock().unwrap()[0].to_string();
    assert!(first.contains("PROJECT BRAIN (verified by file scan") && first.contains("npm run build"), "the overview is in the system prompt");
    let answer = g.db.list_conversation_messages(&conv.id, 20).unwrap().into_iter().rev().find(|m| m.role == "assistant").unwrap();
    assert!(answer.content.contains("App.tsx"));
}

#[test]
fn malformed_brain_calls_are_rejected_before_running() {
    use genius_core::toolcall::validate;
    assert!(validate("project_brain", &json!({}), false).is_err());
    assert!(validate("project_brain", &json!({ "impact": "src/a.ts" }), false).is_err(), "impact must be a list");
    assert!(validate("project_brain", &json!({ "query": 5 }), false).is_err());
    assert!(validate("project_brain", &json!({ "query": "x" }), false).is_ok());
    assert!(validate("project_brain", &json!({ "impact": ["a.ts"] }), false).is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn callers_and_callees_follow_call_like_uses_and_prefer_files_that_import_the_definition() {
    let tmp = tempfile::tempdir().unwrap();
    let g = genius(tmp.path());
    let pid = seed(&g, "Shop");
    let ws = g.workspace(&pid).unwrap();
    // an unrelated file that happens to use the same name without importing the definition
    ws.write_text("src/other.ts", "export function report() { return formatPrice(9); }\n").unwrap();
    let a = g.brain_ask_with(&pid, "", &[], Some("formatPrice"), &all).await.unwrap();
    let c = a.callers.as_ref().unwrap();
    assert_eq!(c.definitions.len(), 1);
    assert_eq!((c.definitions[0].path.as_str(), c.definitions[0].line), ("src/lib/price.ts", 1));
    let by_path: Vec<(&str, bool)> = c.callers.iter().map(|x| (x.path.as_str(), x.imports_definition)).collect();
    assert_eq!(by_path[0], ("src/App.tsx", true), "the importer comes first: {by_path:?}");
    assert!(by_path.contains(&("src/other.ts", false)), "{by_path:?}");
    assert!(!by_path.iter().any(|(p, _)| *p == "src/lib/price.ts"), "the defining file is not its own caller");
    let text = a.render(4000);
    assert!(text.contains("defined as function at src/lib/price.ts:1") && text.contains("src/App.tsx (imports the definition)") && text.contains("src/other.ts (same name only)"), "{text}");
    // a scoped agent sees only what its scope allows
    let only_lib = |p: &str| p.starts_with("src/lib/");
    let scoped = g.brain_ask_with(&pid, "", &[], Some("formatPrice"), &only_lib).await.unwrap();
    let visible: Vec<String> = scoped.callers.unwrap().callers.into_iter().map(|c| c.path).collect();
    assert_eq!(visible, vec!["src/lib/price.test.ts".to_string()], "only callers inside the scope are visible");
    // what a file calls
    let (ix, _) = g.brain_refresh(&pid).unwrap();
    let out = genius_core::brain::callees(&ix, "src/App.tsx", 20);
    assert!(out.contains(&("formatPrice".to_string(), "src/lib/price.ts".to_string())), "{out:?}");
    // the tool accepts "callers" alone
    assert!(genius_core::toolcall::validate("project_brain", &json!({ "callers": "formatPrice" }), false).is_ok());
    assert!(genius_core::toolcall::validate("project_brain", &json!({}), false).is_err());
}
