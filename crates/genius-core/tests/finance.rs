#![allow(clippy::err_expect)]
//! Personal Finance Tracker regression for the whole Code Mode pipeline.
//!
//! The reported defect: Genius showed a tree of "draft" files, said the build succeeded and the
//! preview still displayed the starter page ("Application code has not been generated yet").
//! This test drives the real path — plan → generate → apply → install → type check + build →
//! dev server → real headless browser → follow-up change — against a REAL project folder with
//! real `npm`, `tsc`, `vite`, `vitest` and Chrome/Edge. Only the model is a scripted stand-in,
//! and it reproduces the failure that caused the defect: the first answer is cut off and leaves
//! out `src/App.tsx`, the file that wires the application together.
//!
//! Skips (with a message) when npm or the registry is unavailable; the browser part is skipped
//! when no Chromium-family browser is installed.

use genius_core::app::Genius;
use genius_core::browser::{Step, TestOptions};
use genius_core::events::{EventKind, EventStatus, NullEmitter};
use genius_core::process::{find_executable, Operation};
use genius_core::secrets::MemoryStore;
use genius_core::settings::Policy;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const TYPES_TS: &str = include_str!("fixtures/finance/types.ts");
const FORMAT_TS: &str = include_str!("fixtures/finance/lib/format.ts");
const SUMMARY_TS: &str = include_str!("fixtures/finance/lib/summary.ts");
const SUMMARY_TEST_TS: &str = include_str!("fixtures/finance/lib/summary.test.ts");
const STORAGE_TS: &str = include_str!("fixtures/finance/lib/storage.ts");
const USE_TX_TS: &str = include_str!("fixtures/finance/hooks/useTransactions.ts");
const CARDS_TSX: &str = include_str!("fixtures/finance/components/SummaryCards.tsx");
const CHART_TSX: &str = include_str!("fixtures/finance/components/CategoryChart.tsx");
const FORM_TSX: &str = include_str!("fixtures/finance/components/TransactionForm.tsx");
const LIST_TSX: &str = include_str!("fixtures/finance/components/TransactionList.tsx");
const APP_TSX: &str = include_str!("fixtures/finance/App.tsx");
const INDEX_CSS: &str = include_str!("fixtures/finance/index.css");

fn block(path: &str, content: &str) -> String {
    format!("<<<FILE: {path}>>>\n{content}<<<END FILE>>>\n")
}

fn sse(text: &str, finish: &str) -> ResponseTemplate {
    let mut s = String::new();
    for chunk in text.chars().collect::<Vec<_>>().chunks(97) {
        let c: String = chunk.iter().collect();
        s.push_str(&format!("data: {}\n\n", json!({"choices":[{"index":0,"delta":{"content":c},"finish_reason":null}]})));
    }
    s.push_str(&format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"index":0,"delta":{},"finish_reason":finish}]})));
    ResponseTemplate::new(200).set_body_raw(s, "text/event-stream")
}

fn plan_json() -> String {
    json!({
        "name": "Personal Finance Tracker", "purpose": "Track income and expenses, see the balance and where the money goes.",
        "features": [
            {"name":"Dashboard","description":"Income, expenses and balance"},
            {"name":"Transactions","description":"Create, edit, delete, search and filter"},
            {"name":"Categories","description":"Spending by category with a chart"}
        ],
        "pages": [{"name":"Dashboard","description":"Summary","components":["SummaryCards","CategoryChart"]}],
        "components": [{"name":"SummaryCards","description":"Totals"},{"name":"TransactionForm","description":"Add and edit"}],
        "dataModels": [{"name":"Transaction","fields":["id: string","amount: number","type: income|expense"],"relations":[]}],
        "architecture": "Single page app, React state, localStorage persistence, SVG charts.",
        "dependencies": [], "milestones": ["Data layer","UI"], "testStrategy": "Vitest for the summary maths",
        "risks": [], "assumptions": ["Single currency (USD)"],
        "files": [{"path":"src/App.tsx","purpose":"root"},{"path":"src/components/CategoryChart.tsx","purpose":"chart"}]
    })
    .to_string()
}

#[derive(Default)]
struct Log {
    /// (kind, last user message) of every model request, in order.
    calls: Vec<(String, String)>,
}

struct Scripted {
    log: Arc<Mutex<Log>>,
    /// `forgetful`: the first answer is cut off and leaves out App.tsx (the reported failure).
    /// `never`: the model never writes App.tsx at all.
    mode: &'static str,
}

fn leaf_files() -> String {
    [
        block("src/types.ts", TYPES_TS),
        block("src/lib/format.ts", FORMAT_TS),
        block("src/lib/summary.ts", SUMMARY_TS),
        block("src/lib/summary.test.ts", SUMMARY_TEST_TS),
        block("src/lib/storage.ts", STORAGE_TS),
        block("src/hooks/useTransactions.ts", USE_TX_TS),
        block("src/components/SummaryCards.tsx", CARDS_TSX),
        block("src/components/TransactionForm.tsx", FORM_TSX),
        block("src/components/TransactionList.tsx", LIST_TSX),
        block("src/index.css", INDEX_CSS),
    ]
    .concat()
}

impl Respond for Scripted {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let msgs = body["messages"].as_array().unwrap();
        let system = msgs[0]["content"].as_str().unwrap_or("");
        let last = msgs.last().unwrap()["content"].as_str().unwrap_or("").to_string();
        let kind = if system.contains("planning engine") {
            "plan"
        } else if system.contains("implementation engine") {
            "generate"
        } else if system.contains("code-editing engine") {
            "change"
        } else if system.contains("Build Doctor") {
            "doctor"
        } else {
            "other"
        };
        self.log.lock().unwrap().calls.push((kind.into(), last.clone()));
        match kind {
            "plan" => sse(&plan_json(), "stop"),
            "generate" if last.starts_with("Implement this approved specification") => {
                let regenerated = self.log.lock().unwrap().calls.iter().filter(|(k, l)| k == "generate" && l.starts_with("Implement this approved")).count() > 1;
                if regenerated {
                    // a later regeneration of a project that already works: complete, App.tsx changed
                    let app = format!("{APP_TSX}// regenerated\n");
                    return sse(&format!("Regenerating.\n{}{}{}", leaf_files(), block("src/components/CategoryChart.tsx", CHART_TSX), block("src/App.tsx", &app)), "stop");
                }
                match self.mode {
                    // The reported failure: the leaf files are written, CategoryChart.tsx is cut off
                    // by the length limit, and App.tsx (written last) never arrives.
                    "forgetful" => {
                        let chart = CHART_TSX.lines().take(6).collect::<Vec<_>>().join("\n");
                        sse(&format!("Building a personal finance tracker with a dashboard, transactions and a category chart.\n{}<<<FILE: src/components/CategoryChart.tsx>>>\n{chart}\n", leaf_files()), "length")
                    }
                    // The model finishes "normally" but never writes the application entry.
                    _ => sse(&format!("All done.\n{}", leaf_files()), "stop"),
                }
            }
            "generate" => {
                // a completion round: Genius named what is missing
                if self.mode == "forgetful" {
                    sse(&format!("Here are the missing files.\n{}{}", block("src/components/CategoryChart.tsx", CHART_TSX), block("src/App.tsx", APP_TSX)), "stop")
                } else {
                    sse("I have nothing more to add.", "stop")
                }
            }
            "change" => {
                let summary = SUMMARY_TS
                    .replace("  balance: number;\n}", "  balance: number;\n  savingsRate: number;\n}")
                    .replace("balance: round(income - expenses) };", "balance: round(income - expenses), savingsRate: income > 0 ? (income - expenses) / income : 0 };");
                let cards = CARDS_TSX
                    .replace("import { money } from '../lib/format';", "import { money, percent } from '../lib/format';")
                    .replace("    </section>", "      <article className=\"card savings\">\n        <h3>Savings rate</h3>\n        <p className=\"figure\" data-testid=\"savings-rate\">{percent(summary.savingsRate)}</p>\n      </article>\n    </section>");
                sse(&format!("Added a savings rate card.\n{}{}", block("src/lib/summary.ts", &summary), block("src/components/SummaryCards.tsx", &cards)), "stop")
            }
            _ => sse("ready", "stop"),
        }
    }
}

async fn wait_tasks(g: &Arc<Genius>, pid: &str) {
    for _ in 0..4800 {
        if g.active_tasks(Some(pid)).is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("tasks did not finish");
}

fn task_event(g: &Arc<Genius>, pid: &str, task_id: &str) -> genius_core::events::ActivityEvent {
    let ev = g.bus.list(Some(pid), 5000).unwrap().into_iter().find(|e| e.task_id.as_deref() == Some(task_id) && e.kind == EventKind::Task && e.detail.get("taskKind").is_some()).expect("task event");
    assert!(ev.status.is_terminal(), "task finished");
    ev
}

async fn setup(mode: &'static str) -> (Arc<Genius>, MockServer, Arc<Mutex<Log>>, tempfile::TempDir) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"object":"list","data":[{"id":"mock/coder-1","object":"model","owned_by":"mock"}]}"#))
        .mount(&server)
        .await;
    let log = Arc::new(Mutex::new(Log::default()));
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(Scripted { log: log.clone(), mode }).mount(&server).await;
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.base_url = format!("http://127.0.0.1:{}/v1", server.address().port());
    s.approvals.dependency_install = Policy::Auto; // unattended test; the default is Ask
    g.save_settings(&s).unwrap();
    g.save_api_key("nvapi-finance-test-key-0123456789").unwrap();
    g.list_models(true).await.unwrap();
    g.select_model("mock/coder-1").unwrap();
    (g, server, log, tmp)
}

fn step(action: &str) -> Step {
    Step { action: action.into(), ..Default::default() }
}
fn click(sel: &str) -> Step {
    Step { selector: Some(sel.into()), ..step("click") }
}
fn type_into(sel: &str, text: &str) -> Step {
    Step { selector: Some(sel.into()), text: Some(text.into()), ..step("type") }
}
fn select(sel: &str, value: &str) -> Step {
    Step { selector: Some(sel.into()), value: Some(value.into()), ..step("select") }
}
fn text_in(sel: &str, text: &str) -> Step {
    Step { selector: Some(sel.into()), text: Some(text.into()), ..step("assert_text") }
}
fn page_text(text: &str) -> Step {
    Step { text: Some(text.into()), ..step("assert_text") }
}
fn count(sel: &str, n: u32) -> Step {
    Step { selector: Some(sel.into()), count: Some(n), ..step("assert_count") }
}

fn assert_browser(report: &genius_core::browser::BrowserReport, what: &str) {
    let failed: Vec<String> = report.steps.iter().filter(|s| !s.ok).map(|s| format!("{} → {}", s.description, s.evidence)).collect();
    assert!(failed.is_empty(), "{what}: browser steps failed: {failed:?}");
    assert!(report.ok(), "{what}: blocking problems {:?}", report.blocking_problems());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn personal_finance_tracker_is_generated_applied_built_served_and_works_in_a_real_browser() {
    if find_executable("npm").is_none() || std::env::var("GENIUS_SKIP_E2E").is_ok() {
        eprintln!("SKIPPED: npm not available (or GENIUS_SKIP_E2E set)");
        return;
    }
    let (g, _server, log, _tmp) = setup("forgetful").await;
    let p = g.create_project("Personal Finance Tracker", "").unwrap();
    let pid = p.id.clone();

    eprintln!("[finance] plan");
    // ---- plan
    let t = g.start_plan(&pid, "Build a Personal Finance Tracker: dashboard with income, expenses and balance, transactions with create/edit/delete, categories, search and filtering, charts, persistent data, responsive.", "").unwrap();
    wait_tasks(&g, &pid).await;
    assert_eq!(task_event(&g, &pid, &t.id).status, EventStatus::Completed);

    eprintln!("[finance] generate");
    // ---- generate: the first answer is cut off and has no App.tsx; Genius must notice and ask again
    let t = g.start_generate(&pid).unwrap();
    wait_tasks(&g, &pid).await;
    let ev = task_event(&g, &pid, &t.id);
    assert_eq!(ev.status, EventStatus::Completed, "generation failed: {}", ev.detail);
    let gen = ev.detail["result"].clone();

    // The model was asked a second time, and told exactly what was missing.
    let completion = {
        let l = log.lock().unwrap();
        let gens: Vec<&(String, String)> = l.calls.iter().filter(|(k, _)| k == "generate").collect();
        assert!(gens.len() >= 2, "Genius must ask the model again instead of accepting a partial project (calls: {})", gens.len());
        gens[1].1.clone()
    };
    assert!(completion.contains("src/App.tsx") && completion.contains("src/components/CategoryChart.tsx"), "the follow-up names the missing and the cut-off file: {completion}");

    // The real application is on disk, not the starter page.
    let files: Vec<String> = g.list_files(&pid).unwrap().into_iter().map(|f| f.path).collect();
    for f in ["src/App.tsx", "src/types.ts", "src/lib/summary.ts", "src/lib/storage.ts", "src/hooks/useTransactions.ts", "src/components/TransactionForm.tsx", "src/components/TransactionList.tsx", "src/components/CategoryChart.tsx", "src/components/SummaryCards.tsx", "package.json"] {
        assert!(files.contains(&f.to_string()), "{f} is on disk: {files:?}");
    }
    let app = g.read_file(&pid, "src/App.tsx").unwrap();
    assert!(app.contains("Personal Finance Tracker") && !app.contains("has not been generated yet"), "App.tsx is the generated application");
    assert!(g.read_file(&pid, "src/components/CategoryChart.tsx").unwrap().contains("</svg>"), "the cut-off file was rewritten in full");

    // Every stage ran for real and is reported as what it is.
    assert_eq!(gen["applied"], true);
    assert_eq!(gen["verify"]["ok"], true, "real install + type check + build passed: {gen}");
    assert!(gen["gaps"]["mustWrite"].as_array().unwrap().is_empty(), "nothing left missing: {gen}");
    let builds = g.db.list_runs(&pid, 20).unwrap();
    assert!(builds.iter().any(|r| r.op == "build" && r.status == "succeeded"), "a real build ran and passed");
    let url = gen["verify"]["previewUrl"].as_str().expect("preview url").to_string();
    assert_eq!(gen["runtime"]["servesGeneratedApp"], true, "the dev server serves the generated App module: {gen}");
    let module = reqwest::get(format!("{url}src/App.tsx")).await.unwrap().text().await.unwrap();
    assert!(module.contains("Personal Finance Tracker") && !module.contains("has not been generated yet"));
    let browser = g.browser_available();
    if browser {
        assert_eq!(gen["state"], "verified", "opened in a real browser without errors: {}", gen["runtime"]);
    } else {
        eprintln!("NOTE: no Chrome/Edge installed — browser verification not run");
        assert_eq!(gen["state"], "built", "without a browser the result is 'built', never 'verified'");
    }
    let status = g.project(&pid).unwrap().status;
    assert_eq!(status["stage"], "generated");
    assert!(status["pendingChangeset"].is_null());

    eprintln!("[finance] generated; state={} runtime={}", gen["state"], gen["runtime"]["status"]);
    // ---- the project's own unit tests (generated) pass for real
    let out = g.run_operation(&pid, None, Operation::Test, "finance", true, &Default::default()).await.unwrap();
    assert!(out.ok(), "vitest passed: {}", out.output_tail);

    eprintln!("[finance] vitest ok");
    // ---- interactions in a real browser
    if browser {
        let steps = vec![
            page_text("Personal Finance Tracker"),
            text_in("[data-testid=income]", "$3,650.00"),
            text_in("[data-testid=expenses]", "$1,319.50"),
            text_in("[data-testid=balance]", "$2,330.50"),
            count("[data-testid=chart-bar]", 3),
            // create
            type_into("#tx-description", "Coffee beans"),
            type_into("#tx-amount", "20.5"),
            select("#tx-type", "expense"),
            select("#tx-category", "Food"),
            click("#tx-submit"),
            text_in("[data-testid=balance]", "$2,310.00"),
            text_in("[data-testid=expenses]", "$1,340.00"),
            page_text("Coffee beans"),
            // navigation + list + search
            Step { text: Some("Transactions".into()), ..step("click") },
            count("[data-testid=tx-row]", 6),
            type_into("#tx-search", "Rent"),
            count("[data-testid=tx-row]", 1),
            type_into("#tx-search", ""),
            select("#tx-filter-type", "income"),
            count("[data-testid=tx-row]", 2),
            select("#tx-filter-type", "all"),
            // edit
            click("[aria-label='Edit Groceries']"),
            type_into("#tx-amount", "200"),
            click("#tx-submit"),
            Step { text: Some("Dashboard".into()), ..step("click") },
            text_in("[data-testid=balance]", "$2,274.50"),
            // delete
            Step { text: Some("Transactions".into()), ..step("click") },
            click("[aria-label='Delete Coffee beans']"),
            count("[data-testid=tx-row]", 5),
            Step { text: Some("Dashboard".into()), ..step("click") },
            text_in("[data-testid=balance]", "$2,295.00"),
            // persistence across a reload (localStorage)
            step("reload"),
            text_in("[data-testid=balance]", "$2,295.00"),
            // categories view
            Step { text: Some("Categories".into()), ..step("click") },
            count("[data-testid=category-row]", 3),
            step("assert_no_errors"),
        ];
        let report = g.browser_test(&pid, &url, TestOptions { steps, viewport: Some(1280), screenshot: true, ..Default::default() }).await.unwrap();
        assert_browser(&report, "finance tracker interactions");
        // mobile layout: no horizontal overflow
        let check = g.browser_check(&pid, &url, genius_core::browser::CheckOptions { viewports: vec![375, 768], ..Default::default() }).await.unwrap();
        assert!(check.ok(), "responsive check: {:?}", check.blocking_problems());
    }

    eprintln!("[finance] browser part done");
    // ---- regenerate a project that already has real code: nothing is written until the user applies
    let t = g.start_generate(&pid).unwrap();
    wait_tasks(&g, &pid).await;
    let ev = task_event(&g, &pid, &t.id);
    assert_eq!(ev.status, EventStatus::Completed, "{}", ev.detail);
    let re = ev.detail["result"].clone();
    assert_eq!(re["state"], "awaiting_review", "default policy requires review for real code: {re}");
    assert_eq!(re["applied"], false);
    assert!(!g.read_file(&pid, "src/App.tsx").unwrap().contains("// regenerated"), "drafts are not on disk");
    let pending = g.project(&pid).unwrap().status["pendingChangeset"].as_str().expect("pending changeset recorded").to_string();
    assert_eq!(pending, re["changesetId"].as_str().unwrap());
    g.apply_changeset(&pending).unwrap();
    wait_tasks(&g, &pid).await; // build + check of the applied files
    assert!(g.read_file(&pid, "src/App.tsx").unwrap().contains("// regenerated"));
    let status = g.project(&pid).unwrap().status;
    assert!(status["pendingChangeset"].is_null(), "no longer pending once applied");
    assert_eq!(status["generation"]["state"], if browser { "verified" } else { "built" });

    eprintln!("[finance] regenerate+apply done");
    // applying the regenerated files restarted the preview: use the server that is running now
    let url = g.preview_url(&pid).expect("the preview is running after the review was applied");
    // ---- follow-up change: a proposal first, then the preview shows it
    let t = g.start_change(&pid, "Add a savings rate card to the dashboard", vec![], vec![]).unwrap();
    wait_tasks(&g, &pid).await;
    let ev = task_event(&g, &pid, &t.id);
    assert_eq!(ev.status, EventStatus::Completed, "{}", ev.detail);
    let ch = ev.detail["result"].clone();
    assert_eq!(ch["applied"], false);
    assert!(!g.read_file(&pid, "src/components/SummaryCards.tsx").unwrap().contains("Savings rate"));
    g.apply_changeset(ch["changesetId"].as_str().unwrap()).unwrap();
    wait_tasks(&g, &pid).await;
    assert!(g.read_file(&pid, "src/components/SummaryCards.tsx").unwrap().contains("Savings rate"));
    assert_eq!(g.project(&pid).unwrap().status["lastBuild"]["status"], "succeeded");
    let module = reqwest::get(format!("{url}src/components/SummaryCards.tsx")).await.unwrap().text().await.unwrap();
    assert!(module.contains("Savings rate"), "the running preview serves the changed file");
    if browser {
        let report = g.browser_test(&pid, &url, TestOptions { steps: vec![text_in("[data-testid=savings-rate]", "64%"), step("assert_no_errors")], viewport: Some(1280), ..Default::default() }).await.unwrap();
        assert_browser(&report, "follow-up change visible in the preview");
    }

    // The doctor was never needed: no model call asked for a repair.
    assert!(!log.lock().unwrap().calls.iter().any(|(k, _)| k == "doctor"), "no Build Doctor round was needed");
    g.shutdown().await;
}

/// A model that never writes App.tsx must produce a visible FAILURE, never "build succeeded".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_whose_app_was_never_written_fails_instead_of_reporting_the_starter_as_built() {
    let (g, _server, log, _tmp) = setup("never").await;
    let p = g.create_project("Personal Finance Tracker", "").unwrap();
    let pid = p.id.clone();
    g.start_plan(&pid, "Build a Personal Finance Tracker", "").unwrap();
    wait_tasks(&g, &pid).await;
    let t = g.start_generate(&pid).unwrap();
    wait_tasks(&g, &pid).await;
    let ev = task_event(&g, &pid, &t.id);
    assert_eq!(ev.status, EventStatus::Failed, "must not be reported as completed: {}", ev.detail);
    let msg = ev.detail["error"]["message"].as_str().unwrap();
    assert!(msg.contains("starter page") && msg.contains("NOT generated"), "{msg}");
    assert!(g.read_file(&pid, "src/App.tsx").unwrap().contains("has not been generated yet"), "the starter is still what is on disk");
    let status = g.project(&pid).unwrap().status;
    assert_eq!(status["stage"], "generation_incomplete");
    assert!(g.db.list_runs(&pid, 20).unwrap().iter().all(|r| r.op != "build"), "no build was run and presented as success");
    // bounded: it asked a few times, not forever
    let gens = log.lock().unwrap().calls.iter().filter(|(k, _)| k == "generate").count();
    assert!((2..=4).contains(&gens), "bounded retries, got {gens}");
    g.shutdown().await;
}
