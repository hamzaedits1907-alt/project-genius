//! Real-browser tests: a headless Chromium-family browser is launched, a tiny local site is
//! opened over HTTP and the evidence (console, exceptions, network, inspector, interactions,
//! screenshots) is checked. Skips with a clear message when no browser is installed.

use genius_core::app::Genius;
use genius_core::browser::{find_browser, run_check, run_test, CheckOptions, RunLimits, Step, Target, TestOptions, UrlPolicy};
use genius_core::events::NullEmitter;
use genius_core::secrets::MemoryStore;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PAGE: &str = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Fixture</title>
<style>body{font-family:sans-serif;margin:8px} #wide{width:900px;height:24px;background:#cde}</style></head>
<body><main>
<h1>Fixture page</h1>
<button id="b" onclick="this.textContent='Clicked'">Press me</button>
<label>Name <input id="name"></label> <p id="out" role="status"></p>
<p id="entered"></p>
<select id="theme" aria-label="Theme"><option value="light">Light</option><option value="dark">Dark</option></select>
<button id="add">Add item</button><ul id="items"></ul>
<div id="wide">wide content</div>
<img src="http://example.invalid/blocked.png" alt="blocked">
</main>
<script>
console.error('boom-console');
fetch('/missing').catch(function(){});
setTimeout(function(){ throw new Error('late-exception'); }, 50);
var out = document.getElementById('out'), nameEl = document.getElementById('name');
nameEl.addEventListener('input', function(e){ out.textContent = 'Hi ' + e.target.value; });
nameEl.addEventListener('keydown', function(e){ if (e.key === 'Enter') document.getElementById('entered').textContent = 'Entered ' + nameEl.value; });
document.getElementById('theme').addEventListener('change', function(e){ document.documentElement.dataset.theme = e.target.value; });
var items = document.getElementById('items');
function render(){ items.innerHTML = ''; var n = +(localStorage.getItem('n') || 0); for (var i = 0; i < n; i++) { var li = document.createElement('li'); li.className = 'item'; li.textContent = 'Item ' + (i + 1); items.appendChild(li); } }
document.getElementById('add').addEventListener('click', function(){ localStorage.setItem('n', (+(localStorage.getItem('n') || 0)) + 1); render(); });
render();
</script></body></html>"#;

/// A tiny HTTP server: `/` is the fixture page, `/slow` never answers, anything else is 404.
async fn serve() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    let Ok(k) = sock.read(&mut buf[n..]).await else { return };
                    if k == 0 {
                        return;
                    }
                    n += k;
                    if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") || n == buf.len() {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = head.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
                let (status, ctype, body) = match path.as_str() {
                    "/" => ("200 OK", "text/html; charset=utf-8", PAGE.to_string()),
                    "/slow" => {
                        tokio::time::sleep(Duration::from_secs(600)).await;
                        return;
                    }
                    _ => ("404 Not Found", "text/plain", "not found".to_string()),
                };
                let resp = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

fn skip() -> bool {
    if find_browser().is_none() {
        eprintln!("SKIPPED: no Chromium-family browser found (set GENIUS_BROWSER or PLAYWRIGHT_BROWSERS_PATH=/opt/pw-browsers)");
        return true;
    }
    false
}

fn target(port: u16, shots: Option<&std::path::Path>) -> Target {
    let base = format!("http://127.0.0.1:{port}/");
    Target::new(&base, UrlPolicy::for_origin(&base).unwrap(), shots.map(|p| p.to_path_buf())).unwrap()
}

fn step(v: serde_json::Value) -> Step {
    serde_json::from_value(v).unwrap()
}

/// The browser process must be gone (Linux: its /proc entry disappears).
async fn assert_dead(pid: u32) {
    if !cfg!(target_os = "linux") {
        return;
    }
    for _ in 0..40 {
        // a zombie still has a /proc entry; its state says so
        let alive = std::fs::read_to_string(format!("/proc/{pid}/stat")).map(|s| !s.contains(") Z ")).unwrap_or(false);
        if !alive {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("browser process {pid} is still running");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn check_reports_console_exception_network_blocked_overflow_and_screenshots() {
    if skip() {
        return;
    }
    let port = serve().await;
    let shots = tempfile::tempdir().unwrap();
    let t = target(port, Some(shots.path()));
    let r = run_check(&t, &CheckOptions { viewports: vec![375, 1280], screenshot: true, ..Default::default() }).await.unwrap();
    let text = genius_core::browser::render_browser_report(&r);
    eprintln!("{text}");

    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(r.browser.contains("Chrome") || r.browser.contains("Edg") || r.browser.contains("Chromium"), "{}", r.browser);
    // console
    assert!(r.console.iter().any(|c| c.level == "error" && c.text.contains("boom-console")), "{:?}", r.console);
    // uncaught exception
    assert!(r.exceptions.iter().any(|e| e.text.contains("late-exception")), "{:?}", r.exceptions);
    // failed request: the 404, recorded precisely (not as a vague console line)
    assert!(r.failed_requests.iter().any(|f| f.url.ends_with("/missing") && f.status == Some(404)), "{:?}", r.failed_requests);
    // off-origin request blocked by policy, not reported as an app failure
    assert!(r.blocked_requests.iter().any(|b| b.contains("example.invalid")), "{:?}", r.blocked_requests);
    assert!(!r.failed_requests.iter().any(|f| f.url.contains("example.invalid")), "{:?}", r.failed_requests);
    // inspector ran at both widths; only the phone overflows horizontally
    assert_eq!(r.inspection.viewports.len(), 2);
    let (mobile, desktop) = (&r.inspection.viewports[0], &r.inspection.viewports[1]);
    assert_eq!((mobile.label.as_str(), mobile.viewport.width, desktop.viewport.width), ("mobile", 375, 1280));
    assert!(mobile.layout.iter().any(|i| i.kind == "page-overflow-x"), "{:?}", mobile.layout);
    assert!(!desktop.layout.iter().any(|i| i.kind == "page-overflow-x"), "{:?}", desktop.layout);
    assert!(!mobile.rendered.blank && mobile.text.contains("Fixture page"));
    // screenshots exist on disk and are PNGs
    assert_eq!(r.screenshots.len(), 2);
    for s in &r.screenshots {
        let bytes = std::fs::read(&s.path).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{}", s.path);
        assert!(bytes.len() > 500);
        assert!(std::path::Path::new(&s.path).starts_with(shots.path()));
    }
    // verdict combines the evidence classes
    let problems = r.blocking_problems().join("; ");
    for needle in ["uncaught exception", "console error", "failed network request", "horizontal page overflow at 375px"] {
        assert!(problems.contains(needle), "{needle}: {problems}");
    }
    assert!(text.contains("RESULT: ") && !text.contains("RESULT: OK"));
    // the browser is gone
    assert_dead(r.browser_pid.unwrap()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_runs_real_interactions_and_stops_at_the_first_failure() {
    if skip() {
        return;
    }
    let port = serve().await;
    let shots = tempfile::tempdir().unwrap();
    let t = target(port, Some(shots.path()));
    let steps = vec![
        step(json!({ "action": "assert_text", "text": "Fixture page" })),
        step(json!({ "action": "click", "text": "Press me" })),
        step(json!({ "action": "assert_text", "text": "Clicked" })),
        step(json!({ "action": "type", "selector": "#name", "text": "Ada" })),
        step(json!({ "action": "assert_text", "text": "Hi Ada", "selector": "#out" })),
        step(json!({ "action": "press", "key": "Enter" })),
        step(json!({ "action": "assert_text", "text": "Entered Ada" })),
        step(json!({ "action": "type", "label": "Name", "text": "Bo" })),
        step(json!({ "action": "assert_text", "text": "Hi Bo" })),
        step(json!({ "action": "select", "selector": "#theme", "value": "Dark" })),
        step(json!({ "action": "assert_attr", "selector": "html", "attr": "data-theme", "equals": "dark" })),
        step(json!({ "action": "assert_count", "selector": ".item", "count": 0 })),
        step(json!({ "action": "click", "text": "Add item" })),
        step(json!({ "action": "assert_count", "selector": ".item", "count": 1 })),
        step(json!({ "action": "reload" })),
        step(json!({ "action": "assert_count", "selector": ".item", "count": 1 })),
        step(json!({ "action": "click", "text": "Add item" })),
        step(json!({ "action": "wait_for", "selector": ".item:nth-child(2)" })),
        step(json!({ "action": "scroll", "to": "bottom" })),
        step(json!({ "action": "assert_text", "text": "Item 2" })),
        step(json!({ "action": "assert_text", "text": "this text does not exist", "timeoutMs": 300 })),
        step(json!({ "action": "click", "text": "never reached" })),
    ];
    let r = run_test(&t, &TestOptions { steps, screenshot: false, ..Default::default() }).await.unwrap();
    let text = genius_core::browser::render_browser_report(&r);
    eprintln!("{text}");
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(r.steps.len(), 21, "stops at the failing step: {:?}", r.steps.iter().map(|s| (s.action.clone(), s.ok)).collect::<Vec<_>>());
    for s in &r.steps[..20] {
        assert!(s.ok, "{s:?}");
    }
    let last = r.steps.last().unwrap();
    assert!(!last.ok && last.evidence.contains("this text does not exist") && last.evidence.contains("Fixture page"), "failure carries what the page showed: {last:?}");
    assert!(r.steps[1].evidence.contains("clicked") && r.steps[1].evidence.contains("DOM changed"), "{:?}", r.steps[1]);
    assert!(r.steps[3].evidence.contains("field now contains \"Ada\""), "{:?}", r.steps[3]);
    // a failed step saves a screenshot automatically
    assert_eq!(r.screenshots.len(), 1, "{:?}", r.screenshots);
    assert!(std::path::Path::new(&r.screenshots[0].path).is_file());
    assert!(text.contains("21. FAIL") && text.contains("RESULT: "), "{text}");
    assert_dead(r.browser_pid.unwrap()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn click_fails_with_evidence_when_nothing_matches() {
    if skip() {
        return;
    }
    let port = serve().await;
    let t = target(port, None);
    let r = run_test(&t, &TestOptions { steps: vec![step(json!({ "action": "click", "selector": "#nope" })), step(json!({ "action": "assert_text", "text": "x" }))], ..Default::default() }).await.unwrap();
    assert_eq!(r.steps.len(), 1);
    assert!(!r.steps[0].ok && r.steps[0].evidence.contains("no visible element matches selector #nope"), "{:?}", r.steps[0]);
    assert!(r.screenshots.is_empty(), "no screenshots without a directory");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_origin_navigation_is_refused_before_a_browser_starts() {
    let port = 4999;
    let t = target(port, None);
    for bad in ["https://example.com/", "//example.com/", "http://127.0.0.1:1/", "file:///etc/passwd"] {
        let e = run_test(&t, &TestOptions { steps: vec![step(json!({ "action": "goto", "path": bad }))], ..Default::default() }).await;
        assert!(e.is_err(), "{bad}");
        assert!(e.err().unwrap().to_string().contains("blocked"), "{bad}");
    }
    assert!(run_check(&t, &CheckOptions { path: Some("https://example.com/".into()), ..Default::default() }).await.is_err());
    let e = run_test(&t, &TestOptions { steps: vec![], ..Default::default() }).await.err().unwrap().to_string();
    assert!(e.contains("at least one step"), "{e}");
    let many: Vec<Step> = (0..41).map(|_| step(json!({ "action": "reload" }))).collect();
    assert!(run_test(&t, &TestOptions { steps: many, ..Default::default() }).await.err().unwrap().to_string().contains("at most 40"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn timeout_and_cancellation_kill_the_browser() {
    if skip() {
        return;
    }
    let port = serve().await;
    let t = target(port, None);

    // Hard total timeout: /slow never answers; the report is partial and says so.
    let pid = Arc::new(AtomicU32::new(0));
    let r = run_check(&t, &CheckOptions { path: Some("/slow".into()), viewports: vec![800], limits: RunLimits { timeout: Some(Duration::from_secs(4)), pid_out: Some(pid.clone()), ..Default::default() }, ..Default::default() }).await.unwrap();
    assert!(r.error.as_deref().is_some_and(|e| e.contains("time limit")), "{:?}", r.error);
    assert!(r.duration_ms < 15_000, "{}", r.duration_ms);
    assert!(!r.ok());
    assert_ne!(pid.load(Ordering::SeqCst), 0);
    assert_dead(pid.load(Ordering::SeqCst)).await;

    // Cancellation mid-run.
    let pid = Arc::new(AtomicU32::new(0));
    let cancel = tokio_util::sync::CancellationToken::new();
    let c2 = cancel.clone();
    let p2 = pid.clone();
    let t2 = t.clone();
    let h = tokio::spawn(async move { run_check(&t2, &CheckOptions { path: Some("/slow".into()), viewports: vec![800], limits: RunLimits { cancel: Some(c2), pid_out: Some(p2), ..Default::default() }, ..Default::default() }).await });
    for _ in 0..100 {
        if pid.load(Ordering::SeqCst) != 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let started = pid.load(Ordering::SeqCst);
    assert_ne!(started, 0, "browser started");
    tokio::time::sleep(Duration::from_millis(800)).await;
    cancel.cancel();
    let res = tokio::time::timeout(Duration::from_secs(10), h).await.expect("returns promptly after cancel").unwrap();
    assert!(matches!(res, Err(genius_core::GeniusError::Cancelled)), "{:?}", res.map(|r| r.error));
    assert_dead(started).await;

    // Dropping the future (task abort) also kills it.
    let pid = Arc::new(AtomicU32::new(0));
    let p2 = pid.clone();
    let t3 = t.clone();
    let h = tokio::spawn(async move { run_check(&t3, &CheckOptions { path: Some("/slow".into()), viewports: vec![800], limits: RunLimits { pid_out: Some(p2), ..Default::default() }, ..Default::default() }).await });
    for _ in 0..100 {
        if pid.load(Ordering::SeqCst) != 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.abort();
    let _ = h.await;
    assert_dead(pid.load(Ordering::SeqCst)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn genius_only_opens_its_own_dev_server_or_dist() {
    let tmp = tempfile::tempdir().unwrap();
    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let mut s = g.settings().unwrap();
    s.workspace_dir = Some(tmp.path().join("projects").to_string_lossy().to_string());
    g.save_settings(&s).unwrap();
    let p = g.create_project("Browser Guard", "x").unwrap();
    // no dev server → refused, whatever the URL
    let e = g.browser_check(&p.id, "http://127.0.0.1:5173/", CheckOptions::default()).await.err().unwrap().to_string();
    assert!(e.contains("not running"), "{e}");
    // file outside dist
    let e = g.browser_check(&p.id, "file:///etc/passwd", CheckOptions::default()).await.err().unwrap().to_string();
    assert!(e.contains("not allowed"), "{e}");
    let e = g.browser_test(&p.id, "https://example.com/", TestOptions { steps: vec![step(json!({ "action": "reload" }))], ..Default::default() }).await.err().unwrap().to_string();
    assert!(e.contains("not running") || e.contains("own dev server"), "{e}");
    assert!(g.browser_check("nope", "http://127.0.0.1:1/", CheckOptions::default()).await.is_err());
    assert_eq!(g.browser_available(), find_browser().is_some());
}
