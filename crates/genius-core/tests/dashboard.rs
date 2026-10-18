#![allow(clippy::err_expect)]
//! Dashboard regression: a scripted model builds a multi-widget dashboard against a REAL
//! scaffolded Vite + React + TypeScript project with real tools, real `npm install`, real
//! `tsc`, real `vite build` and a real dev server. Only the model is a wiremock script.
//!
//! The script starts with malformed tool calls in several dialects, writes one real
//! TypeScript error (TS2322), must receive the actual compiler diagnostic back, fixes it with
//! `edit_file`, then a follow-up request (dark mode) is applied, rebuilt, and rolled back.
//!
//! Requires Node.js/npm and the npm registry. Skips otherwise (or with GENIUS_SKIP_E2E set).

use genius_core::agent::{AgentMode, AgentRequest};
use genius_core::app::Genius;
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::process::find_executable;
use genius_core::secrets::MemoryStore;
use genius_core::settings::Policy;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn sse(text: &str) -> ResponseTemplate {
    let mut s = String::new();
    for chunk in text.chars().collect::<Vec<_>>().chunks(61) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
    ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
}

type Script = dyn Fn(&str, &str, &[Value]) -> String + Send + Sync;

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

fn file_block(p: &str, c: &str) -> String {
    format!("<<<FILE: {p}>>>\n{c}<<<END FILE>>>\n")
}

// ---------------------------------------------------------------------------- file contents

const TYPES_TS: &str = "export type Stat = { label: string; value: number; delta: string };
export type Order = { id: string; customer: string; region: 'north' | 'south' | 'west'; amount: number; status: 'paid' | 'pending' };
export type Activity = { id: string; text: string; time: string };
";

const DATA_TS: &str = "import type { Activity, Order, Stat } from './types';

export const stats: Stat[] = [
  { label: 'Revenue', value: 48210, delta: '+12%' },
  { label: 'Orders', value: 1204, delta: '+4%' },
  { label: 'Customers', value: 932, delta: '+7%' },
];
export const series: number[] = [12, 18, 15, 27, 31, 29, 42];
export const orders: Order[] = [
  { id: 'A-1001', customer: 'Acme Co', region: 'north', amount: 1200, status: 'paid' },
  { id: 'A-1002', customer: 'Globex', region: 'south', amount: 860, status: 'pending' },
  { id: 'A-1003', customer: 'Initech', region: 'west', amount: 430, status: 'paid' },
];
export const activity: Activity[] = [
  { id: '1', text: 'Acme Co paid invoice A-1001', time: '2m ago' },
  { id: '2', text: 'Globex opened a ticket', time: '19m ago' },
];
";

// The TS2322 lives in App.tsx: `value` is typed number but a string is passed.
const STAT_CARD: &str = "type Props = { label: string; value: number; delta: string };

export default function StatCard({ label, value, delta }: Props) {
  return (
    <article className=\"stat-card\" aria-label={label}>
      <h3>{label}</h3>
      <p className=\"stat-value\">{value.toLocaleString()}</p>
      <span className=\"stat-delta\">{delta}</span>
    </article>
  );
}
";

const LINE_CHART: &str = "type Props = { points: number[]; title: string };

export default function LineChart({ points, title }: Props) {
  const w = 320;
  const h = 120;
  const max = Math.max(...points, 1);
  const d = points.map((p, i) => `${(i / (points.length - 1)) * w},${h - (p / max) * h}`).join(' ');
  return (
    <figure className=\"chart\">
      <figcaption>{title}</figcaption>
      <svg viewBox={`0 0 ${w} ${h}`} role=\"img\" aria-label={title}>
        <polyline fill=\"none\" stroke=\"currentColor\" strokeWidth=\"2\" points={d} />
      </svg>
    </figure>
  );
}
";

const TABLE: &str = "import type { Order } from '../types';

export default function DataTable({ rows }: { rows: Order[] }) {
  return (
    <table className=\"orders\">
      <caption>Recent orders</caption>
      <thead>
        <tr><th>ID</th><th>Customer</th><th>Region</th><th>Amount</th><th>Status</th></tr>
      </thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id}><td>{r.id}</td><td>{r.customer}</td><td>{r.region}</td><td>{r.amount}</td><td>{r.status}</td></tr>
        ))}
      </tbody>
    </table>
  );
}
";

const FEED: &str = "import type { Activity } from '../types';

export default function ActivityFeed({ items }: { items: Activity[] }) {
  return (
    <section className=\"feed\" aria-label=\"Recent activity\">
      <h2>Recent activity</h2>
      <ul>
        {items.map((a) => (
          <li key={a.id}>{a.text} <time>{a.time}</time></li>
        ))}
      </ul>
    </section>
  );
}
";

const FILTERS: &str = "type Props = { region: string; onRegion: (r: string) => void };

export default function Filters({ region, onRegion }: Props) {
  return (
    <label className=\"filters\">
      Region
      <select value={region} onChange={(e) => onRegion(e.target.value)}>
        <option value=\"all\">All regions</option>
        <option value=\"north\">North</option>
        <option value=\"south\">South</option>
        <option value=\"west\">West</option>
      </select>
    </label>
  );
}
";

const APP_TSX: &str = "import { useState } from 'react';
import './App.css';
import { activity, orders, series, stats } from './data';
import StatCard from './components/StatCard';
import LineChart from './components/LineChart';
import DataTable from './components/DataTable';
import ActivityFeed from './components/ActivityFeed';
import Filters from './components/Filters';

export default function App() {
  const [region, setRegion] = useState('all');
  const rows = orders.filter((o) => region === 'all' || o.region === region);
  return (
    <main className=\"dashboard\">
      <h1>Sales dashboard</h1>
      <Filters region={region} onRegion={setRegion} />
      <div className=\"stats\">
        <StatCard label=\"Revenue\" value=\"48,210\" delta=\"+12%\" />
        <StatCard label=\"Orders\" value={stats[1].value} delta={stats[1].delta} />
        <StatCard label=\"Customers\" value={stats[2].value} delta={stats[2].delta} />
      </div>
      <LineChart points={series} title=\"Weekly revenue\" />
      <DataTable rows={rows} />
      <ActivityFeed items={activity} />
    </main>
  );
}
";

const APP_CSS: &str = ".dashboard { max-width: 960px; margin: 0 auto; padding: 24px; }
.stats { display: grid; grid-template-columns: repeat(auto-fit, minmax(160px, 1fr)); gap: 16px; }
.stat-card { border: 1px solid #ccc; border-radius: 8px; padding: 12px; }
.chart svg { width: 100%; height: auto; }
.orders { width: 100%; border-collapse: collapse; }
.orders td, .orders th { border-bottom: 1px solid #ddd; padding: 6px; text-align: left; }
";

const DARK_CSS: &str = ".dashboard.dark { background: #111; color: #eee; }
.dashboard.dark .stat-card { border-color: #444; }
";

// ---------------------------------------------------------------------------- helpers

/// Run an agent request and wait (generously: real npm install/tsc/vite happen inside).
async fn run_agent(g: &Arc<Genius>, pid: &str, conv: &str, msg: &str) -> Value {
    let t = g
        .start_agent(
            pid,
            AgentRequest { conversation_id: conv.into(), message: msg.into(), mode: AgentMode::Build, effort: Some("max".into()), autopilot: Some(true), ..Default::default() },
        )
        .unwrap();
    for _ in 0..4800 {
        if g.active_tasks(Some(pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(g.active_tasks(Some(pid)).is_empty(), "agent task did not finish in time");
    let ev = g.bus.list(Some(pid), 5000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(t.id.as_str()) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).unwrap();
    json!({ "taskId": t.id, "status": ev.status, "detail": ev.detail })
}

fn dist_js(root: &std::path::Path) -> String {
    let mut out = String::new();
    if let Ok(rd) = std::fs::read_dir(root.join("dist/assets")) {
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "js") {
                out.push_str(&std::fs::read_to_string(e.path()).unwrap_or_default());
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_builds_through_malformed_calls_real_diagnostics_dev_server_followup_and_rollback() {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        eprintln!("SKIPPED: npm not available (or GENIUS_SKIP_E2E set)");
        return;
    }

    // Observations recorded by the scripted model, asserted after the run (a panic inside the
    // mock would only surface as an opaque HTTP 500).
    let notes: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let n2 = notes.clone();
    let wrong_calls = Arc::new(Mutex::new(0usize));
    let wc = wrong_calls.clone();

    let script = move |system: &str, last: &str, _msgs: &[Value]| -> String {
        let note = |k: &str, v: &str| n2.lock().unwrap().push((k.to_string(), v.to_string()));
        if system.contains("Reviewer subagent") {
            return "<final>No blocking issues found.</final>".into();
        }
        // ---- request 1
        if last.contains("USER MESSAGE:\nBuild a dashboard") {
            // A: `arguments` is a JSON *string* whose content has raw newlines in it.
            let inner = format!("{{\"path\":\"src/types.ts\",\"content\":\"{}\"}}", TYPES_TS);
            let a = json!({ "name": "write_file", "arguments": inner }).to_string();
            // B: `files` as an object map {path: content}.
            let b = json!({ "name": "write_files", "args": { "files": { "src/data.ts": DATA_TS, "src/components/StatCard.tsx": STAT_CARD } } }).to_string();
            // C: OpenAI tool-call shape with a JSON string of arguments.
            let c = json!({ "type": "function", "function": { "name": "write_file", "arguments": json!({ "path": "src/components/LineChart.tsx", "content": LINE_CHART }).to_string() } }).to_string();
            // D: genuinely wrong (no content).
            let d = json!({ "name": "write_file", "args": { "path": "src/components/DataTable.tsx" } }).to_string();
            return format!("Building the dashboard.\n<tool>{a}</tool>\n<tool>{b}</tool>\n<tool_call>{c}</tool_call>\n<tool>{d}</tool>");
        }
        if last.starts_with("<tool_result") && last.contains("name=\"write_file\" ok=\"false\"") {
            let mut w = wc.lock().unwrap();
            *w += 1;
            note(&format!("wrong-call-feedback-{}", *w), last);
            if *w == 1 {
                // the same wrong call, again
                let d = json!({ "name": "write_file", "args": { "path": "src/components/DataTable.tsx" } }).to_string();
                return format!("<tool>{d}</tool>");
            }
            // after the second rejection the model switches to file blocks, with a type error in App.tsx
            return format!(
                "Switching to file blocks.\n<final>The dashboard is ready.\n{}{}{}{}{}</final>",
                file_block("src/components/DataTable.tsx", TABLE),
                file_block("src/components/ActivityFeed.tsx", FEED),
                file_block("src/components/Filters.tsx", FILTERS),
                file_block("src/App.tsx", APP_TSX),
                file_block("src/App.css", APP_CSS)
            );
        }
        if last.starts_with("AUTOPILOT CHECK FAILED") {
            note("diagnostic-request", last);
            return "Fixing the prop type.\n<tool>{\"name\":\"edit_file\",\"args\":{\"path\":\"src/App.tsx\",\"old\":\"value=\\\"48,210\\\"\",\"new\":\"value={stats[0].value}\"}}</tool>".into();
        }
        if last.starts_with("<tool_result name=\"edit_file\"") && !last.contains("dark") {
            return "<final>Fixed the type error; the dashboard builds.</final>".into();
        }
        // ---- request 2 (follow-up)
        if last.contains("USER MESSAGE:\nAdd a dark mode toggle") {
            let edit = json!({ "name": "edit_file", "args": { "path": "src/App.tsx", "edits": [
                { "old": "import './App.css';", "new": "import './App.css';\nimport './dark.css';" },
                { "old": "const [region, setRegion] = useState('all');", "new": "const [region, setRegion] = useState('all');\n  const [dark, setDark] = useState(false);" },
                { "old": "<main className=\"dashboard\">", "new": "<main className={dark ? 'dashboard dark' : 'dashboard'}>" },
                { "old": "<h1>Sales dashboard</h1>", "new": "<h1>Sales dashboard</h1>\n      <button type=\"button\" aria-pressed={dark} onClick={() => setDark(!dark)}>Dark mode</button>" }
            ] } })
            .to_string();
            let w = json!({ "name": "write_file", "args": { "path": "src/dark.css", "content": DARK_CSS } }).to_string();
            return format!("Adding the toggle.\n<tool>{w}</tool>\n<tool>{edit}</tool>");
        }
        if last.starts_with("<tool_result") && last.contains("dark") {
            return "<final>Added a dark mode toggle.</final>".into();
        }
        note("unexpected", last);
        "<final>unexpected turn</final>".into()
    };

    // ---- environment: mock provider, key, real project folder
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { script: Box::new(script), log: log.clone() }).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/coder-1".into());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    s.approvals.dependency_install = Policy::Auto; // unattended
    s.approvals.apply_ai_edits = Policy::Auto;
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-dashboard-test-key-0123456789").unwrap();
    let p = g.create_project("Sales Dashboard", "A multi-widget sales dashboard").unwrap();
    let pid = p.id.clone();
    let ws = g.workspace(&pid).unwrap();
    for (path, content) in genius_core::scaffold::files(&p.name, "A multi-widget sales dashboard") {
        ws.write_text(&path, &content).unwrap();
    }
    let conv = g.db.create_conversation(&pid, "New conversation").unwrap();

    // ================================================ 1-4: build, real diagnostic, fix
    let r = run_agent(&g, &pid, &conv.id, "Build a dashboard with stat cards, an SVG line chart, a table, an activity feed and filters").await;
    let obs = notes.lock().unwrap().clone();
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}\nnotes: {obs:#?}");
    assert!(!obs.iter().any(|(k, _)| k == "unexpected"), "unexpected model turn: {obs:#?}");
    assert_eq!(r["detail"]["result"]["applied"], true, "{r}");

    let steps = g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    let summary: Vec<(String, bool)> = steps.iter().map(|s| (s.tool.clone(), s.ok)).collect();
    eprintln!("STEPS ({}): {summary:?}", steps.len());
    for s in &steps {
        eprintln!("  {} ok={} {}", s.tool, s.ok, s.summary.lines().next().unwrap_or(""));
    }
    assert!(steps.len() < 32, "bounded: {} steps: {summary:?}", steps.len());
    assert!(log.lock().unwrap().len() < 16, "model round trips bounded: {}", log.lock().unwrap().len());

    // Malformed dialects were normalised and wrote real files.
    assert_eq!(steps.iter().filter(|s| s.tool == "write_file" && s.ok).count(), 2, "string-arguments + OpenAI form both accepted: {summary:?}");
    assert!(steps.iter().any(|s| s.tool == "write_files" && s.ok), "object-map files accepted: {summary:?}");
    assert_eq!(ws.read_text("src/types.ts").unwrap().trim_end(), TYPES_TS.trim_end(), "raw-newline string arguments preserved the content");
    assert!(ws.read_text("src/data.ts").unwrap().contains("export const stats"));
    assert!(ws.read_text("src/components/LineChart.tsx").unwrap().contains("<polyline"));
    // The genuinely wrong call was rejected with guidance, and the repeat got the stronger message.
    let fb: Vec<&(String, String)> = obs.iter().filter(|(k, _)| k.starts_with("wrong-call-feedback")).collect();
    assert_eq!(fb.len(), 2, "wrong call fed back exactly twice, not retried endlessly: {obs:#?}");
    assert!(fb[0].1.contains("Correct form"), "guidance with an example: {}", fb[0].1);
    assert!(fb[1].1.contains("will not accept this form again") || fb[1].1.contains("BLOCKED"), "second attempt escalates: {}", fb[1].1);
    assert!(steps.iter().filter(|s| s.tool == "write_file" && !s.ok).count() <= 2, "{summary:?}");

    // Real tsc diagnostic reached the model (code + file + the offending text).
    let diag: Vec<&(String, String)> = obs.iter().filter(|(k, _)| k == "diagnostic-request").collect();
    assert_eq!(diag.len(), 1, "exactly one failing check was handed back: {obs:#?}");
    assert!(diag[0].1.contains("TS2322"), "actual diagnostic code: {}", diag[0].1);
    assert!(diag[0].1.contains("src/App.tsx"), "actual diagnostic file: {}", diag[0].1);
    assert!(diag[0].1.contains("48,210") || diag[0].1.contains("string"), "diagnostic detail / code excerpt: {}", diag[0].1);

    // Fixed with edit_file; typecheck and production build pass; dist has the dashboard.
    assert!(steps.iter().any(|s| s.tool == "edit_file" && s.ok), "{summary:?}");
    let app_v1 = ws.read_text("src/App.tsx").unwrap();
    assert!(app_v1.contains("value={stats[0].value}") && !app_v1.contains("48,210"));
    let msgs = g.db.list_conversation_messages(&conv.id, 20).unwrap();
    let checks = msgs.last().unwrap().meta["report"]["autopilot"]["checks"].as_array().unwrap().clone();
    assert_eq!(checks.first().map(|c| (c["check"].clone(), c["ok"].clone())), Some((json!("typecheck"), json!(false))), "{checks:?}");
    assert!(checks.iter().any(|c| c["check"] == "build" && c["ok"] == true), "vite build verified: {checks:?}");
    assert!(ws.exists("dist/index.html"), "dist built");
    let js = dist_js(ws.root());
    assert!(js.contains("Recent activity") && js.contains("Sales dashboard"), "dist JS contains dashboard text");
    assert!(g.project(&pid).unwrap().status["lastBuild"]["status"] == "succeeded");

    // ================================================ 5: real dev server over HTTP
    let url = g.start_preview(&pid, None).await.expect("dev server starts");
    let html = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(html.contains("<div id=\"root\"></div>"), "dev server serves the app HTML: {html}");
    let module = reqwest::get(format!("{url}src/App.tsx")).await.unwrap().text().await.unwrap();
    assert!(module.contains("Sales dashboard"), "dev server serves the generated App module");

    // ================================================ 6: follow-up request
    let app_before_followup = ws.read_text("src/App.tsx").unwrap();
    let r2 = run_agent(&g, &pid, &conv.id, "Add a dark mode toggle").await;
    let obs = notes.lock().unwrap().clone();
    assert_eq!(r2["status"], json!(EventStatus::Completed), "{r2}\nnotes: {obs:#?}");
    assert!(!obs.iter().any(|(k, _)| k == "unexpected"), "unexpected model turn: {obs:#?}");
    assert_eq!(r2["detail"]["result"]["applied"], true, "{r2}");
    let app_v2 = ws.read_text("src/App.tsx").unwrap();
    assert!(app_v2.contains("Dark mode") && app_v2.contains("./dark.css"), "{app_v2}");
    assert!(ws.exists("src/dark.css"));
    let msgs = g.db.list_conversation_messages(&conv.id, 20).unwrap();
    let last = msgs.last().unwrap();
    let checks2 = last.meta["report"]["autopilot"]["checks"].as_array().unwrap().clone();
    assert!(checks2.iter().all(|c| c["ok"] == true) && checks2.iter().any(|c| c["check"] == "build"), "follow-up typechecked and rebuilt: {checks2:?}");
    assert!(dist_js(ws.root()).contains("Dark mode"), "rebuilt dist contains the toggle");
    let module = reqwest::get(format!("{url}src/App.tsx")).await.unwrap().text().await.unwrap();
    assert!(module.contains("Dark mode"), "dev server serves the updated App");

    // ================================================ 7: rollback of the follow-up changeset(s)
    let ids: Vec<String> = last.meta["changesetIds"].as_array().expect("follow-up changesets").iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert!(!ids.is_empty());
    let reverted = g.revert_checkpoints(&pid, &ids).unwrap();
    assert_eq!(reverted, ids.len());
    assert_eq!(ws.read_text("src/App.tsx").unwrap(), app_before_followup, "App.tsx restored to the pre-follow-up content");
    assert!(!ws.exists("src/dark.css"), "file added by the follow-up is removed");
    assert_eq!(ws.read_text("src/types.ts").unwrap().trim_end(), TYPES_TS.trim_end(), "earlier work is untouched");
    assert!(ws.read_text("src/components/DataTable.tsx").unwrap().contains("Recent orders"));

    // Rewinding the whole conversation restores the project to its pre-dashboard state.
    g.stop_preview(&pid).await.unwrap();
    let first = msgs[0].id.clone();
    let rw = g.rewind_conversation(&conv.id, &first, true).unwrap();
    assert!(rw["revertedChangesets"].as_u64().unwrap() >= 1, "{rw}");
    assert!(!ws.exists("src/components/StatCard.tsx") && !ws.exists("src/data.ts"), "dashboard files removed by rewind");
    assert!(ws.read_text("src/App.tsx").unwrap().contains("Genius template"), "template App restored");

    // Nothing in the whole exchange leaked a key.
    for body in log.lock().unwrap().iter() {
        assert!(!body.to_string().contains("nvapi-dashboard-test-key"));
    }
}

// =====================================================================================
// Upgraded acceptance: realistic multi-widget dashboard, real npm/tsc/vite/dev server and a
// REAL headless browser (src/browser.rs). Fixture sources live in tests/fixtures/dashboard.
// =====================================================================================

use genius_core::browser::{find_browser, CheckOptions, RunLimits, Session, Step, TestOptions};

const F_TYPES: &str = include_str!("fixtures/dashboard/types.ts");
const F_DATA: &str = include_str!("fixtures/dashboard/data.ts");
const F_HOOKS: &str = include_str!("fixtures/dashboard/hooks.ts");
const F_WIDGETS: &str = include_str!("fixtures/dashboard/widgets.tsx");
const F_APP: &str = include_str!("fixtures/dashboard/App.tsx");
const F_CSS: &str = include_str!("fixtures/dashboard/App.css");
const F_DENSITY: &str = include_str!("fixtures/dashboard/density.ts");
const F_COMPACT: &str = include_str!("fixtures/dashboard/compact.css");

type Notes = Arc<Mutex<Vec<(String, String)>>>;

struct Env {
    g: Arc<Genius>,
    pid: String,
    conv: String,
    log: Arc<Mutex<Vec<Value>>>,
    _server: MockServer,
    _tmp: tempfile::TempDir,
}

async fn setup(script: impl Fn(&str, &str, &[Value]) -> String + Send + Sync + 'static) -> Env {
    let server = MockServer::start().await;
    let log = Arc::new(Mutex::new(Vec::new()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { script: Box::new(script), log: log.clone() }).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.model = Some("mock/coder-1".into());
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    s.approvals.dependency_install = Policy::Auto;
    s.approvals.apply_ai_edits = Policy::Auto;
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-dashboard-test-key-0123456789").unwrap();
    let p = g.create_project("Team Dashboard", "A multi-widget dashboard").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    for (path, content) in genius_core::scaffold::files(&p.name, "A multi-widget dashboard") {
        ws.write_text(&path, &content).unwrap();
    }
    let conv = g.db.create_conversation(&p.id, "New conversation").unwrap();
    Env { g, pid: p.id, conv: conv.id, log, _server: server, _tmp: tmp }
}

async fn ask(e: &Env, msg: &str, autopilot: bool) -> Value {
    let t = e.g.start_agent(&e.pid, AgentRequest { conversation_id: e.conv.clone(), message: msg.into(), mode: AgentMode::Build, effort: Some("max".into()), autopilot: Some(autopilot), ..Default::default() }).unwrap();
    for _ in 0..4800 {
        if e.g.active_tasks(Some(&e.pid)).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(e.g.active_tasks(Some(&e.pid)).is_empty(), "agent task did not finish in time");
    let ev = e.g.bus.list(Some(&e.pid), 5000).unwrap().into_iter().find(|x| x.task_id.as_deref() == Some(t.id.as_str()) && x.kind == EventKind::Task && x.detail.get("taskKind").is_some()).unwrap();
    json!({ "taskId": t.id, "status": ev.status, "detail": ev.detail })
}

fn checks_of(e: &Env) -> Vec<Value> {
    let msgs = e.g.db.list_conversation_messages(&e.conv, 50).unwrap();
    msgs.last().unwrap().meta["report"]["autopilot"]["checks"].as_array().cloned().unwrap_or_default()
}

fn steps_of(v: Value) -> Vec<Step> {
    serde_json::from_value(v).unwrap()
}

fn tool(name: &str, args: Value) -> String {
    format!("<tool>{}</tool>", json!({ "name": name, "args": args }))
}

fn build_reply() -> String {
    let files = tool(
        "write_files",
        json!({ "files": [
        { "path": "src/types.ts", "content": F_TYPES }, { "path": "src/data.ts", "content": F_DATA },
        { "path": "src/hooks/storage.ts", "content": F_HOOKS }, { "path": "src/components/widgets.tsx", "content": F_WIDGETS } ] }),
    );
    format!("Building the dashboard.\n{files}\n{}\n{}", tool("write_file", json!({ "path": "src/App.tsx", "content": F_APP })), tool("write_file", json!({ "path": "src/App.css", "content": F_CSS })))
}

struct Acc(Vec<(&'static str, String)>);
impl Acc {
    fn pass(&mut self, section: &'static str, msg: impl Into<String>) {
        let m = msg.into();
        eprintln!("[{section}] PASS {m}");
        self.0.push((section, m));
    }
    fn summary(&self, outstanding: &[&str]) {
        eprintln!("==== DASHBOARD ACCEPTANCE ====");
        for s in ["files written", "build", "runtime/preview", "interaction", "visual/a11y"] {
            let n = self.0.iter().filter(|(k, _)| *k == s).count();
            eprintln!("{s:16} {n} checks passed");
        }
        for o in outstanding {
            eprintln!("outstanding      {o}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dashboard_acceptance_real_build_preview_browser_followup_rollback() {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        eprintln!("SKIPPED: npm not available (or GENIUS_SKIP_E2E set)");
        return;
    }
    let notes: Notes = Arc::new(Mutex::new(Vec::new()));
    let n2 = notes.clone();
    let e = setup(move |system, last, _| {
        if system.contains("Reviewer subagent") {
            return "<final>No blocking issues found.</final>".into();
        }
        if last.starts_with("AUTOPILOT CHECK FAILED") {
            n2.lock().unwrap().push(("unexpected-failure".into(), last.to_string()));
            return "<final>stop</final>".into();
        }
        if last.contains("USER MESSAGE:\nBuild a team dashboard") {
            return build_reply();
        }
        if last.contains("USER MESSAGE:\nAdd a compact density toggle") {
            let edit = tool(
                "edit_file",
                json!({ "path": "src/App.tsx", "edits": [
                { "old": "import './App.css';", "new": "import './App.css';\nimport './compact.css';" },
                { "old": "import { useLocalStorage, useTheme } from './hooks/storage';", "new": "import { useLocalStorage, useTheme } from './hooks/storage';\nimport { useDensity } from './hooks/density';" },
                { "old": "const { theme, toggle } = useTheme();", "new": "const { theme, toggle } = useTheme();\n  const density = useDensity();" },
                { "old": "<div className=\"app\">", "new": "<div className=\"app\" data-density={density.compact ? 'compact' : 'comfortable'}>" },
                { "old": "<button type=\"submit\">Add widget</button>", "new": "<button type=\"submit\">Add widget</button>\n          <button type=\"button\" aria-pressed={density.compact} onClick={density.toggle}>Compact view</button>" } ] }),
            );
            return format!("{}\n{}\n{edit}", tool("write_file", json!({ "path": "src/hooks/density.ts", "content": F_DENSITY })), tool("write_file", json!({ "path": "src/compact.css", "content": F_COMPACT })));
        }
        if last.starts_with("<tool_result") {
            return "<final>Done.</final>".into();
        }
        n2.lock().unwrap().push(("unexpected".into(), last.to_string()));
        "<final>unexpected turn</final>".into()
    })
    .await;
    let ws = e.g.workspace(&e.pid).unwrap();
    let mut acc = Acc(vec![]);

    // ---- build through validated tools; real npm install, tsc, vite build
    let r = ask(&e, "Build a team dashboard with metrics, tasks, notes, an SVG chart, add/remove widgets and themes", true).await;
    let obs = notes.lock().unwrap().clone();
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}\n{obs:#?}");
    assert!(obs.is_empty(), "{obs:#?}");
    let steps = e.g.agent_steps(r["taskId"].as_str().unwrap()).unwrap();
    assert!(steps.iter().filter(|s| s.ok && s.tool.starts_with("write_file")).count() >= 2, "{:?}", steps.iter().map(|s| (&s.tool, s.ok)).collect::<Vec<_>>());
    for f in ["src/types.ts", "src/data.ts", "src/hooks/storage.ts", "src/components/widgets.tsx", "src/App.tsx", "src/App.css"] {
        assert!(ws.exists(f), "{f}");
    }
    acc.pass("files written", "6 files written through write_files/write_file");
    let checks = checks_of(&e);
    assert!(checks.iter().any(|c| c["check"] == "typecheck" && c["ok"] == true) && checks.iter().any(|c| c["check"] == "build" && c["ok"] == true), "{checks:?}");
    assert!(ws.exists("dist/index.html"));
    acc.pass("build", "real npm install + tsc + vite build passed");

    // ---- dev server
    let url = e.g.start_preview(&e.pid, None).await.expect("dev server starts");
    let html = reqwest::get(&url).await.unwrap().text().await.unwrap();
    assert!(html.contains("<div id=\"root\"></div>"));
    acc.pass("runtime/preview", format!("dev server serves the app at {url}"));

    let have_browser = find_browser().is_some();
    if have_browser {
        // ---- real browser: render, console, network, viewports, a11y
        let rep = e.g.browser_check(&e.pid, &url, CheckOptions { viewports: vec![375, 768, 1280], screenshot: true, ..Default::default() }).await.unwrap();
        eprintln!("{}", genius_core::browser::render_browser_report(&rep));
        assert!(rep.error.is_none(), "{:?}", rep.error);
        assert!(rep.ok(), "blocking problems: {:?}", rep.blocking_problems());
        assert_eq!(rep.inspection.viewports.len(), 3);
        assert!(rep.inspection.viewports.iter().all(|v| !v.rendered.blank && v.text.contains("Team dashboard")));
        acc.pass("runtime/preview", "real browser: page rendered at 375/768/1280, no console errors/exceptions/failed requests");
        assert!(rep.inspection.viewports.iter().all(|v| !v.layout.iter().any(|i| i.kind == "page-overflow-x")));
        acc.pass("visual/a11y", "no horizontal overflow at 375/768/1280");
        let a11y: Vec<_> = rep.inspection.viewports.iter().flat_map(|v| v.a11y.iter().map(|i| format!("{}: {}", i.kind, i.message))).collect();
        assert!(a11y.is_empty(), "a11y problems: {a11y:?}");
        acc.pass("visual/a11y", "inspector: 0 accessibility problems (labels, names, headings, lang, contrast)");
        assert!(rep.screenshots.len() == 3 && rep.screenshots.iter().all(|s| std::fs::metadata(&s.path).map(|m| m.len() > 500).unwrap_or(false)));
        acc.pass("visual/a11y", "3 screenshots saved (PNG files; not reviewed by a vision model)");

        // ---- interactions at desktop
        let steps = steps_of(json!([
            { "action": "assert_count", "selector": ".widget", "count": 4 },
            { "action": "assert_text", "text": "$48,210" },
            { "action": "assert_count", "selector": "svg .bar", "count": 7 },
            { "action": "select", "selector": "#period", "value": "Last 30 days" },
            { "action": "assert_text", "text": "$182,400" },
            { "action": "assert_count", "selector": "svg .bar", "count": 30 },
            { "action": "select", "selector": "#period", "value": "Last 7 days" },
            { "action": "select", "selector": "#add-kind", "value": "Metric" },
            { "action": "click", "text": "Add widget" },
            { "action": "assert_count", "selector": ".widget", "count": 5 },
            { "action": "assert_text", "text": "New metric 5" },
            { "action": "click", "text": "Remove New metric 5" },
            { "action": "assert_count", "selector": ".widget", "count": 4 },
            { "action": "assert_text", "text": "New metric 5", "absent": true },
            { "action": "select", "selector": "#add-kind", "value": "Notes" },
            { "action": "click", "text": "Add widget" },
            { "action": "assert_count", "selector": ".widget", "count": 5 },
            { "action": "type", "label": "New task", "text": "Write tests" },
            { "action": "press", "key": "Enter" },
            { "action": "assert_text", "text": "Write tests" },
            { "action": "click", "text": "Write tests" },
            { "action": "assert_count", "selector": ".task.done", "count": 1 },
            { "action": "type", "selector": "textarea[aria-label='Notes']", "text": "Ship the dashboard" },
            { "action": "assert_attr", "selector": "html", "attr": "data-theme", "equals": "light" },
            { "action": "click", "text": "Switch to dark theme" },
            { "action": "assert_attr", "selector": "html", "attr": "data-theme", "equals": "dark" },
            { "action": "assert_attr", "selector": "html", "attr": "class", "contains": "dark" },
            { "action": "reload" },
            { "action": "assert_attr", "selector": "html", "attr": "data-theme", "equals": "dark" },
            { "action": "assert_count", "selector": ".widget", "count": 5 },
            { "action": "assert_count", "selector": ".task.done", "count": 1 },
            { "action": "assert_attr", "selector": "textarea[aria-label='Notes']", "attr": "value", "equals": "Ship the dashboard" },
            { "action": "click", "text": "Remove New notes 5" },
            { "action": "assert_count", "selector": ".widget", "count": 4 },
            { "action": "assert_no_errors" }
        ]));
        let rep = e.g.browser_test(&e.pid, &url, TestOptions { steps, screenshot: true, ..Default::default() }).await.unwrap();
        eprintln!("{}", genius_core::browser::render_browser_report(&rep));
        assert!(rep.error.is_none() && rep.steps.iter().all(|s| s.ok), "{:?}", rep.steps.iter().find(|s| !s.ok));
        acc.pass("interaction", format!("{} scripted steps passed: period select, add/remove widget, tasks, notes, theme switch, reload persistence", rep.steps.len()));

        // ---- mobile: Session API directly
        let t = e.g.browser_target(&e.pid, &url).unwrap();
        let mut s = Session::launch(t, &RunLimits::default(), Duration::from_secs(60)).await.unwrap();
        s.set_viewport(375).await.unwrap();
        s.goto("/").await.unwrap();
        let w = s.eval("({sw: document.documentElement.scrollWidth, iw: window.innerWidth})").await.unwrap();
        assert!(w["sw"].as_i64() <= w["iw"].as_i64(), "horizontal overflow on mobile: {w}");
        for st in steps_of(json!([{ "action": "click", "text": "Add widget" }, { "action": "assert_count", "selector": ".widget", "count": 5 }])) {
            assert!(s.run_step(&st).await.unwrap().ok);
        }
        let w = s.eval("({sw: document.documentElement.scrollWidth, iw: window.innerWidth})").await.unwrap();
        assert!(w["sw"].as_i64() <= w["iw"].as_i64(), "overflow after adding a widget on mobile: {w}");
        let rep = s.finish("test").await;
        assert!(rep.console_errors() == 0 && rep.exceptions.is_empty());
        acc.pass("interaction", "mobile 375px: adding a widget works and the page never scrolls horizontally");
    }

    // ---- follow-up and rollback
    let before = ws.read_text("src/App.tsx").unwrap();
    let r2 = ask(&e, "Add a compact density toggle", true).await;
    assert_eq!(r2["status"], json!(EventStatus::Completed), "{r2}\n{:?}", notes.lock().unwrap());
    assert!(ws.read_text("src/App.tsx").unwrap().contains("Compact view"));
    let checks2 = checks_of(&e);
    assert!(checks2.iter().all(|c| c["ok"] == true) && checks2.iter().any(|c| c["check"] == "build"), "{checks2:?}");
    acc.pass("build", "follow-up typechecked and rebuilt");
    if have_browser {
        let steps = steps_of(json!([
            { "action": "click", "text": "Compact view" },
            { "action": "assert_attr", "selector": ".app", "attr": "data-density", "equals": "compact" },
            { "action": "reload" },
            { "action": "assert_attr", "selector": ".app", "attr": "data-density", "equals": "compact" },
            { "action": "assert_count", "selector": ".widget", "count": 4 },
            { "action": "assert_no_errors" }
        ]));
        let rep = e.g.browser_test(&e.pid, &url, TestOptions { steps, ..Default::default() }).await.unwrap();
        assert!(rep.steps.iter().all(|s| s.ok) && rep.error.is_none(), "{}", genius_core::browser::render_browser_report(&rep));
        acc.pass("interaction", "follow-up feature works in the browser and persists across reload");
    }
    let msgs = e.g.db.list_conversation_messages(&e.conv, 50).unwrap();
    let ids: Vec<String> = msgs.last().unwrap().meta["changesetIds"].as_array().expect("changesets").iter().map(|v| v.as_str().unwrap().to_string()).collect();
    e.g.revert_checkpoints(&e.pid, &ids).unwrap();
    assert_eq!(ws.read_text("src/App.tsx").unwrap(), before);
    assert!(!ws.exists("src/compact.css"));
    if have_browser {
        let steps = steps_of(json!([{ "action": "assert_text", "text": "Compact view", "absent": true }, { "action": "assert_count", "selector": ".widget", "count": 4 }, { "action": "assert_no_errors" }]));
        let rep = e.g.browser_test(&e.pid, &url, TestOptions { steps, ..Default::default() }).await.unwrap();
        assert!(rep.steps.iter().all(|s| s.ok), "{}", genius_core::browser::render_browser_report(&rep));
        acc.pass("interaction", "after rollback the toggle is gone in the real browser and the dashboard still works");
    }
    e.g.stop_preview(&e.pid).await.unwrap();
    for body in e.log.lock().unwrap().iter() {
        assert!(!body.to_string().contains("nvapi-dashboard-test-key"));
    }
    acc.pass("runtime/preview", "no key leaked to the model");
    let mut out = vec!["model is scripted (mock); live NVIDIA not exercised", "only Chromium-family tested; screenshots saved but not judged by a vision model"];
    if !have_browser {
        out.push("NO BROWSER INSTALLED: all real-browser checks were NOT run");
    }
    acc.summary(&out);
}

/// Scripted model: first reply writes `first`; a failing autopilot check is recorded and
/// answered with `repair`; everything else ends the turn.
async fn fail_then_repair(first: String, repair: String, codes: &[&str], texts: &[&str]) {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        eprintln!("SKIPPED: npm not available");
        return;
    }
    let notes: Notes = Arc::new(Mutex::new(Vec::new()));
    let n2 = notes.clone();
    let e = setup(move |system, last, _| {
        if system.contains("Reviewer subagent") {
            return "<final>No blocking issues found.</final>".into();
        }
        if last.starts_with("AUTOPILOT CHECK FAILED") {
            n2.lock().unwrap().push(("diag".into(), last.to_string()));
            return repair.clone();
        }
        if last.contains("USER MESSAGE:\nBuild it") {
            return first.clone();
        }
        if last.starts_with("<tool_result") {
            return "<final>Done.</final>".into();
        }
        n2.lock().unwrap().push(("unexpected".into(), last.to_string()));
        "<final>unexpected</final>".into()
    })
    .await;
    let r = ask(&e, "Build it", true).await;
    let obs = notes.lock().unwrap().clone();
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}\n{obs:#?}");
    let diags: Vec<_> = obs.iter().filter(|(k, _)| k == "diag").collect();
    assert_eq!(diags.len(), 1, "{obs:#?}");
    assert!(codes.iter().any(|c| diags[0].1.contains(c)), "real diagnostic code {codes:?}: {}", diags[0].1);
    for t in texts {
        assert!(diags[0].1.contains(t), "{t}: {}", diags[0].1);
    }
    assert!(!obs.iter().any(|(k, _)| k == "unexpected"), "{obs:#?}");
    let checks = checks_of(&e);
    assert!(checks.iter().any(|c| c["check"] == "build" && c["ok"] == true), "repaired and rebuilt: {checks:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_named_export_is_diagnosed_and_repaired() {
    let first = format!(
        "{}\n{}",
        tool("write_file", json!({ "path": "src/hooks/useWidgets.ts", "content": "export function useWidgetStore() { return [1, 2, 3]; }\n" })),
        tool("write_file", json!({ "path": "src/App.tsx", "content": "import { useWidgets } from './hooks/useWidgets';\nexport default function App() { return <main><h1>x</h1>{useWidgets().length}</main>; }\n" }))
    );
    let repair = tool("edit_file", json!({ "path": "src/App.tsx", "edits": [{ "old": "{ useWidgets }", "new": "{ useWidgetStore }" }, { "old": "{useWidgets().length}", "new": "{useWidgetStore().length}" }] }));
    fail_then_repair(first, repair, &["TS2305", "TS2724"], &["useWidgets", "src/App.tsx"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unresolved_import_missing_file_is_diagnosed_and_repaired() {
    let first = tool("write_file", json!({ "path": "src/App.tsx", "content": "import Chart from './components/Chart';\nexport default function App() { return <main><h1>x</h1><Chart /></main>; }\n" }));
    let repair = tool("write_file", json!({ "path": "src/components/Chart.tsx", "content": "export default function Chart() { return <svg role=\"img\" aria-label=\"c\" />; }\n" }));
    fail_then_repair(first, repair, &["TS2307"], &["./components/Chart"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_hook_return_contract_is_diagnosed_and_repaired() {
    let first = format!(
        "{}\n{}",
        tool("write_file", json!({ "path": "src/useTheme.ts", "content": "export function useTheme() { return ['light', () => {}] as const; }\n" })),
        tool("write_file", json!({ "path": "src/App.tsx", "content": "import { useTheme } from './useTheme';\nexport default function App() { const { theme, toggle } = useTheme(); return <main><h1>{theme}</h1><button onClick={toggle}>t</button></main>; }\n" }))
    );
    let repair = tool("edit_file", json!({ "path": "src/App.tsx", "old": "const { theme, toggle }", "new": "const [theme, toggle]" }));
    fail_then_repair(first, repair, &["TS2339"], &["theme"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn missing_dependency_is_diagnosed_and_installed() {
    let first = tool("write_file", json!({ "path": "src/App.tsx", "content": "import dayjs from 'dayjs';\nexport default function App() { return <main><h1>{dayjs('2020-01-02').format('YYYY')}</h1></main>; }\n" }));
    let repair = tool("add_dependency", json!({ "packages": ["dayjs"] }));
    fail_then_repair(first, repair, &["TS2307"], &["dayjs"]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn preview_start_failures_are_reported_honestly() {
    // No npm needed: dependencies are not installed.
    let notes: Notes = Arc::new(Mutex::new(Vec::new()));
    let n2 = notes.clone();
    let e = setup(move |system, last, _| {
        if system.contains("Reviewer subagent") {
            return "<final>ok</final>".into();
        }
        if last.contains("USER MESSAGE:\nLook at it") {
            return tool("browser_check", json!({}));
        }
        if last.starts_with("<tool_result") {
            n2.lock().unwrap().push(("result".into(), last.to_string()));
        }
        "<final>reported</final>".into()
    })
    .await;
    let err = e.g.start_preview(&e.pid, None).await.err().expect("must fail").to_string();
    assert!(err.contains("not installed"), "{err}");
    assert!(e.g.preview_url(&e.pid).is_none());
    let r = ask(&e, "Look at it", false).await;
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}");
    let obs = notes.lock().unwrap().clone();
    let res = &obs.iter().find(|(k, _)| k == "result").expect("tool result").1;
    assert!(res.contains("ok=\"false\""), "{res}");
    if find_browser().is_some() {
        assert!(res.contains("dev server could not be started"), "{res}");
    } else {
        assert!(res.contains("NOT run"), "{res}");
    }

    // With npm: a broken vite.config.ts makes the dev server exit; then it is repaired.
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        return;
    }
    let out = e.g.run_operation(&e.pid, None, genius_core::process::Operation::Install, "test", true, &tokio_util::sync::CancellationToken::new()).await.unwrap();
    assert!(out.ok(), "{}", out.output_tail);
    let ws = e.g.workspace(&e.pid).unwrap();
    let good = ws.read_text("vite.config.ts").unwrap();
    ws.write_text("vite.config.ts", "export default {{{ broken\n").unwrap();
    let err = e.g.start_preview(&e.pid, None).await.err().expect("broken config must fail").to_string();
    eprintln!("preview failure message: {err}");
    assert!(e.g.preview_url(&e.pid).is_none());
    ws.write_text("vite.config.ts", &good).unwrap();
    let url = e.g.start_preview(&e.pid, None).await.expect("repaired config starts");
    assert!(e.g.preview_url(&e.pid).is_some());
    if find_browser().is_some() {
        let rep = e.g.browser_check(&e.pid, &url, CheckOptions { viewports: vec![800], ..Default::default() }).await.unwrap();
        assert!(rep.error.is_none() && rep.inspection.viewports[0].text.contains("Project template"), "{}", genius_core::browser::render_browser_report(&rep));
    }
    e.g.stop_preview(&e.pid).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn runtime_exception_found_by_browser_check_and_repaired() {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() || find_browser().is_none() {
        eprintln!("SKIPPED: needs npm and a browser");
        return;
    }
    let notes: Notes = Arc::new(Mutex::new(Vec::new()));
    let n2 = notes.clone();
    let calls = Arc::new(Mutex::new(0usize));
    let e = setup(move |system, last, _| {
        if system.contains("Reviewer subagent") {
            return "<final>ok</final>".into();
        }
        if last.contains("USER MESSAGE:\nBuild it") {
            return tool("write_file", json!({ "path": "src/App.tsx", "content": "import { useEffect, useState } from 'react';\ntype Item = { id: number; name: string };\nexport default function App() {\n  const [items, setItems] = useState<Item[]>([]);\n  useEffect(() => { fetch('/api/items').then((r) => r.json()).then((d: { items: Item[] }) => setItems(d.items)); }, []);\n  return <main><h1>Items</h1><ul>{items.map((i) => <li key={i.id}>{i.name}</li>)}</ul></main>;\n}\n" }));
        }
        if last.starts_with("<tool_result name=\"browser_check\"") {
            n2.lock().unwrap().push(("report".into(), last.to_string()));
            let mut c = calls.lock().unwrap();
            *c += 1;
            if last.contains("[exception]") && *c == 1 {
                return tool("edit_file", json!({ "path": "src/App.tsx", "old": "fetch('/api/items').then((r) => r.json()).then((d: { items: Item[] }) => setItems(d.items));", "new": "setItems([{ id: 1, name: 'Alpha' }]);" }));
            }
            return "<final>Verified in the browser.</final>".into();
        }
        if last.starts_with("<tool_result name=\"write_file\"") {
            return tool("run_check", json!({ "check": "build" }));
        }
        if last.starts_with("<tool_result") {
            return tool("browser_check", json!({ "viewports": [800] }));
        }
        "<final>done</final>".into()
    })
    .await;
    let r = ask(&e, "Build it", true).await;
    let obs = notes.lock().unwrap().clone();
    assert_eq!(r["status"], json!(EventStatus::Completed), "{r}\n{obs:#?}");
    let reports: Vec<&String> = obs.iter().filter(|(k, _)| k == "report").map(|(_, v)| v).collect();
    assert!(reports.len() >= 2, "{obs:#?}");
    assert!(reports[0].contains("[exception]") && reports[0].contains("RESULT: ") && !reports[0].contains("RESULT: OK"), "{}", reports[0]);
    assert!(reports.last().unwrap().contains("RESULT: OK"), "{}", reports.last().unwrap());
    e.g.stop_preview(&e.pid).await.unwrap();
}
