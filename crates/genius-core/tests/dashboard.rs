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
