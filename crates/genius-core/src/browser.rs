//! Real browser automation ("Browser Vision Loop").
//!
//! Genius opens the generated app in a REAL headless Chromium-family browser (Chrome, Chromium
//! or Edge that is already installed; nothing is bundled, no Node/Playwright), independent of
//! the preview iframe in the UI, and talks to it over the Chrome DevTools Protocol (CDP) on a
//! WebSocket. It records what actually happened: console errors, uncaught exceptions, failed
//! and 4xx/5xx network requests, the page inspector's findings (render check, outline, overflow,
//! accessibility, contrast) at several viewport widths, screenshots saved to disk, and the
//! result of scripted interactions (click, type, press, wait, assert, scroll).
//!
//! A screenshot is never treated as proof: the report combines DOM, console, network and
//! interaction evidence, and says plainly when the browser could not be started (the visual
//! check was then NOT run).
//!
//! Safety:
//! * Only the project's own dev server origin (loopback + its port) and `file:` URLs inside the
//!   project's `dist` may be loaded. Navigation targets are checked before use and EVERY
//!   request the page makes (including redirects and sub-resources) is intercepted with the
//!   CDP `Fetch` domain and failed unless allowed. Blocked requests are listed in the report.
//! * Hard limits: total time, number of steps and viewports, and output size.
//! * The browser process (whole process group) and its temporary profile are removed on every
//!   exit path: normal finish, error, timeout, cancellation and drop.
//! * Page content is untrusted: everything that reaches the report is bounded and redacted.
//!
//! The WebSocket client is a minimal built-in implementation (text frames to a loopback peer),
//! so no extra dependency is needed.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use reqwest::Url;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::Genius;
use crate::error::{GeniusError, Result};
use crate::inspection::{self, ConsoleCounts, InspectionReport, ViewportReport};
use crate::redact::redact;

// ------------------------------------------------------------------------------------ limits

pub const DEFAULT_VIEWPORTS: &[u32] = &[375, 768, 1280];
const MAX_VIEWPORTS: usize = 4;
pub const MAX_STEPS: usize = 40;
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const TEST_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_TIMEOUT: Duration = Duration::from_secs(300);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Cold browser starts on a loaded machine (CI runners starting several at once) can take well over 25 s.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_WS_MESSAGE: u64 = 64 * 1024 * 1024;
const MAX_CONSOLE: usize = 60;
const MAX_EXCEPTIONS: usize = 30;
const MAX_FAILED: usize = 60;
const MAX_BLOCKED: usize = 30;
const MAX_TEXT: usize = 300;
/// Most text a rendered report gives the model.
pub const MAX_REPORT_CHARS: usize = 14_000;
const KEEP_SCREENSHOTS: usize = 40;
const ASSERT_RETRY: Duration = Duration::from_millis(1500);

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Single-line, redacted, bounded: how page-controlled text enters a report.
fn tidy(s: &str, max: usize) -> String {
    clip(&redact(&s.replace(['\n', '\r', '\t'], " ")), max)
}

fn proc_err(msg: impl Into<String>) -> GeniusError {
    GeniusError::Process(msg.into())
}

// ------------------------------------------------------------------------------ discovery

/// Where a Chromium-family browser may be installed, most preferred first. Pure (apart from
/// listing the Playwright cache directory); `os` is "windows" | "macos" | "linux".
pub fn browser_candidates(env: &dyn Fn(&str) -> Option<String>, os: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let Some(p) = env("GENIUS_BROWSER").filter(|p| !p.trim().is_empty()) {
        out.push(PathBuf::from(p.trim()));
    }
    match os {
        "windows" => {
            for (var, rel) in [
                ("ProgramFiles(x86)", r"Microsoft\Edge\Application\msedge.exe"),
                ("ProgramFiles", r"Microsoft\Edge\Application\msedge.exe"),
                ("ProgramFiles", r"Google\Chrome\Application\chrome.exe"),
                ("ProgramFiles(x86)", r"Google\Chrome\Application\chrome.exe"),
                ("LOCALAPPDATA", r"Google\Chrome\Application\chrome.exe"),
            ] {
                if let Some(base) = env(var).filter(|b| !b.is_empty()) {
                    out.push(PathBuf::from(base).join(rel));
                }
            }
        }
        "macos" => {
            out.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
            out.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
            out.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
            if let Some(home) = env("HOME").filter(|h| !h.is_empty()) {
                out.push(PathBuf::from(home).join("Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
            }
        }
        _ => {
            let pw = env("PLAYWRIGHT_BROWSERS_PATH").filter(|p| !p.trim().is_empty() && p.trim() != "0").unwrap_or_else(|| "/opt/pw-browsers".into());
            out.extend(playwright_chromiums(Path::new(&pw)));
            for p in ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable", "/usr/bin/microsoft-edge", "/snap/bin/chromium"] {
                out.push(p.into());
            }
        }
    }
    out
}

/// `chromium-*/chrome-linux*/chrome` below a Playwright browsers directory, newest first.
pub fn playwright_chromiums(base: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = match std::fs::read_dir(base) {
        Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("chromium-"))).collect(),
        Err(_) => return Vec::new(),
    };
    dirs.sort();
    dirs.reverse();
    let mut out = Vec::new();
    for d in dirs {
        if let Ok(rd) = std::fs::read_dir(&d) {
            let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("chrome-"))).collect();
            subs.sort();
            for s in subs {
                out.push(s.join(if cfg!(windows) { "chrome.exe" } else { "chrome" }));
            }
        }
    }
    out
}

/// The browser Genius would use, or `None` (then no browser check can run).
pub fn find_browser() -> Option<PathBuf> {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let env = |k: &str| std::env::var(k).ok();
    if let Some(p) = browser_candidates(&env, os).into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    if os == "linux" {
        for name in ["chromium", "chromium-browser", "google-chrome", "google-chrome-stable", "microsoft-edge", "chrome"] {
            if let Some(p) = crate::process::find_executable(name) {
                return Some(p);
            }
        }
    }
    None
}

pub const NO_BROWSER_MESSAGE: &str = "No Chromium-family browser (Google Chrome, Microsoft Edge or Chromium) was found on this computer, so the real-browser check was NOT run. Do not claim the app was visually verified or tested in a browser. Installing Chrome or Edge (or setting GENIUS_BROWSER to its executable) enables browser_check and browser_test.";

// ---------------------------------------------------------------------------- URL policy

/// Which URLs the browser may load: the project's dev server origin and files inside `dist`.
#[derive(Debug, Clone)]
pub struct UrlPolicy {
    port: u16,
    file_root: Option<PathBuf>,
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

/// Canonical form of `path` even when it does not exist yet: the nearest existing ancestor is
/// resolved (symlinks, and Windows' `\\?\` prefix) and the missing tail is appended, so existing
/// and not-yet-existing paths compare consistently.
fn canon_lenient(path: &Path) -> std::path::PathBuf {
    if let Ok(c) = path.canonicalize() {
        return c;
    }
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    while let Some(name) = cur.file_name().map(|n| n.to_os_string()) {
        tail.push(name);
        if !cur.pop() {
            break;
        }
        if let Ok(mut c) = cur.canonicalize() {
            for n in tail.iter().rev() {
                c.push(n);
            }
            return c;
        }
    }
    path.to_path_buf()
}

impl UrlPolicy {
    /// Allow the origin of `dev_url` (an http URL on a loopback host with a port).
    pub fn for_origin(dev_url: &str) -> Result<UrlPolicy> {
        let u = Url::parse(dev_url).map_err(|e| GeniusError::Validation(format!("invalid URL {dev_url}: {e}")))?;
        let host_ok = u.host_str().is_some_and(is_loopback_host);
        if u.scheme() != "http" || !host_ok || u.port().is_none() {
            return Err(GeniusError::Validation(format!("the browser may only open the project's own dev server on this computer (http://127.0.0.1:<port>), not {dev_url}")));
        }
        Ok(UrlPolicy { port: u.port().unwrap(), file_root: None })
    }

    /// A policy that only allows `file:` URLs inside `root` (a project's `dist`).
    pub fn for_files(root: &Path) -> UrlPolicy {
        UrlPolicy { port: 0, file_root: Some(canon_lenient(root)) }
    }

    pub fn with_file_root(mut self, root: &Path) -> UrlPolicy {
        self.file_root = Some(canon_lenient(root));
        self
    }

    /// May the page load this URL (navigation, redirect or sub-resource)?
    pub fn allows(&self, raw: &str) -> bool {
        let Ok(u) = Url::parse(raw) else { return false };
        match u.scheme() {
            "http" | "ws" => self.port != 0 && u.username().is_empty() && u.password().is_none() && u.host_str().is_some_and(is_loopback_host) && u.port() == Some(self.port),
            "file" => match (&self.file_root, u.to_file_path()) {
                (Some(root), Ok(p)) => {
                    let p = canon_lenient(&p);
                    !p.components().any(|c| matches!(c, std::path::Component::ParentDir)) && p.starts_with(root)
                }
                _ => false,
            },
            // Inline resources never leave the machine.
            "data" | "blob" | "about" => true,
            _ => false,
        }
    }
}

/// What to open and where its evidence goes.
#[derive(Debug, Clone)]
pub struct Target {
    pub base: Url,
    pub policy: UrlPolicy,
    /// Where screenshots are saved (created on demand); `None` disables screenshots.
    pub shots_dir: Option<PathBuf>,
}

impl Target {
    /// `base` must itself satisfy `policy`.
    pub fn new(base: &str, policy: UrlPolicy, shots_dir: Option<PathBuf>) -> Result<Target> {
        let base_url = Url::parse(base).map_err(|e| GeniusError::Validation(format!("invalid URL {base}: {e}")))?;
        if !policy.allows(base_url.as_str()) {
            return Err(GeniusError::Validation(format!("{base} is not allowed: the browser may only open the project's own dev server or files inside its dist folder")));
        }
        Ok(Target { base: base_url, policy, shots_dir })
    }

    /// A path ("/", "/settings?tab=1") or absolute URL → a URL the policy allows.
    pub fn resolve(&self, path_or_url: &str) -> Result<String> {
        let raw = path_or_url.trim();
        let raw = if raw.is_empty() { "/" } else { raw };
        let url = if raw.contains("://") || raw.starts_with("//") || raw.starts_with("data:") || raw.starts_with("javascript:") || raw.starts_with("blob:") { Url::parse(raw).or_else(|_| self.base.join(raw)) } else { self.base.join(raw) };
        let url = url.map_err(|e| GeniusError::Validation(format!("invalid path {raw}: {e}")))?;
        if !matches!(url.scheme(), "http" | "file") || !self.policy.allows(url.as_str()) {
            return Err(GeniusError::Validation(format!("navigation to {} is blocked: the browser may only open the project's own dev server (or files inside its dist folder)", clip(url.as_str(), 120))));
        }
        Ok(url.to_string())
    }
}

// ------------------------------------------------------------------------ WebSocket client

fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = vec![0x80 | opcode];
    let len = payload.len();
    if len < 126 {
        out.push(0x80 | len as u8);
    } else if len <= u16::MAX as usize {
        out.push(0x80 | 126);
        out.extend((len as u16).to_be_bytes());
    } else {
        out.push(0x80 | 127);
        out.extend((len as u64).to_be_bytes());
    }
    out.extend(mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

#[derive(Debug, PartialEq)]
struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

/// Parse one frame from the front of `buf`: `Ok(None)` when more bytes are needed.
fn parse_frame(buf: &[u8]) -> std::result::Result<Option<(Frame, usize)>, String> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let fin = buf[0] & 0x80 != 0;
    let opcode = buf[0] & 0x0f;
    let masked = buf[1] & 0x80 != 0;
    let mut len = (buf[1] & 0x7f) as u64;
    let mut pos = 2;
    if len == 126 {
        if buf.len() < 4 {
            return Ok(None);
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as u64;
        pos = 4;
    } else if len == 127 {
        if buf.len() < 10 {
            return Ok(None);
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap());
        pos = 10;
    }
    if len > MAX_WS_MESSAGE {
        return Err(format!("WebSocket frame of {len} bytes exceeds the limit"));
    }
    let mask = if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let m = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(m)
    } else {
        None
    };
    let len = len as usize;
    if buf.len() < pos + len {
        return Ok(None);
    }
    let mut payload = buf[pos..pos + len].to_vec();
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Ok(Some((Frame { fin, opcode, payload }, pos + len)))
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        let u = uuid::Uuid::new_v4();
        for b in u.as_bytes() {
            if i < N {
                out[i] = *b;
                i += 1;
            }
        }
    }
    out
}

// ------------------------------------------------------------------------------------ CDP

#[derive(Debug, Clone)]
struct Event {
    method: String,
    params: Value,
    session_id: Option<String>,
}

enum Incoming {
    Response { id: u64, result: std::result::Result<Value, String> },
    Event(Event),
    Ignore,
}

fn parse_incoming(text: &str) -> Incoming {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Incoming::Ignore };
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        let result = match v.get("error") {
            Some(e) => Err(e.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_string()),
            None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        };
        return Incoming::Response { id, result };
    }
    match v.get("method").and_then(Value::as_str) {
        Some(m) => Incoming::Event(Event { method: m.to_string(), params: v.get("params").cloned().unwrap_or(Value::Null), session_id: v.get("sessionId").and_then(Value::as_str).map(String::from) }),
        None => Incoming::Ignore,
    }
}

type Pending = Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, String>>>>;

struct CdpInner {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    pending: Pending,
    next: AtomicU64,
}

#[derive(Clone)]
struct Cdp {
    inner: Arc<CdpInner>,
}

impl Cdp {
    /// Connect to `ws://127.0.0.1:<port><path>`. Returns the connection, the event stream and
    /// the reader task (abort it to disconnect).
    async fn connect(port: u16, path: &str) -> Result<(Cdp, mpsc::UnboundedReceiver<Event>, JoinHandle<()>)> {
        let mut stream = tokio::time::timeout(Duration::from_secs(10), TcpStream::connect(("127.0.0.1", port))).await.map_err(|_| proc_err("timed out connecting to the browser"))?.map_err(|e| proc_err(format!("could not connect to the browser: {e}")))?;
        let key = base64::engine::general_purpose::STANDARD.encode(random_bytes::<16>());
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n");
        stream.write_all(req.as_bytes()).await.map_err(|e| proc_err(format!("browser handshake failed: {e}")))?;
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 4096];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            if buf.len() > 16 * 1024 {
                return Err(proc_err("browser handshake response too large"));
            }
            let n = tokio::time::timeout(Duration::from_secs(10), stream.read(&mut chunk)).await.map_err(|_| proc_err("timed out in the browser handshake"))?.map_err(|e| proc_err(format!("browser handshake failed: {e}")))?;
            if n == 0 {
                return Err(proc_err("the browser closed the connection during the handshake"));
            }
            buf.extend_from_slice(&chunk[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        if !head.starts_with("HTTP/1.1 101") {
            return Err(proc_err(format!("the browser refused the DevTools connection: {}", head.lines().next().unwrap_or(""))));
        }
        let rest = buf.split_off(head_end);
        let (rd, wr) = stream.into_split();
        let inner = Arc::new(CdpInner { writer: tokio::sync::Mutex::new(wr), pending: Mutex::new(HashMap::new()), next: AtomicU64::new(1) });
        let (tx, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(reader_loop(rd, rest, inner.clone(), tx));
        Ok((Cdp { inner }, rx, task))
    }

    async fn send_raw(&self, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
        let frame = encode_frame(opcode, payload, random_bytes::<4>());
        let mut w = self.inner.writer.lock().await;
        w.write_all(&frame).await
    }

    async fn call(&self, method: &str, params: Value, session: Option<&str>) -> Result<Value> {
        let id = self.inner.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        if let Err(e) = self.send_raw(1, msg.to_string().as_bytes()).await {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(proc_err(format!("lost the connection to the browser: {e}")));
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(proc_err(format!("{method}: {e}"))),
            Ok(Err(_)) => Err(proc_err("the browser closed the connection (it may have crashed)")),
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err(proc_err(format!("{method} timed out")))
            }
        }
    }
}

async fn reader_loop(mut rd: OwnedReadHalf, mut buf: Vec<u8>, inner: Arc<CdpInner>, events: mpsc::UnboundedSender<Event>) {
    let mut message: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    'outer: loop {
        loop {
            match parse_frame(&buf) {
                Ok(Some((f, n))) => {
                    buf.drain(..n);
                    match f.opcode {
                        0 | 1 | 2 => {
                            if f.opcode != 0 {
                                message.clear();
                            }
                            message.extend_from_slice(&f.payload);
                            if message.len() as u64 > MAX_WS_MESSAGE {
                                break 'outer;
                            }
                            if f.fin {
                                let text = String::from_utf8_lossy(&message).to_string();
                                message.clear();
                                match parse_incoming(&text) {
                                    Incoming::Response { id, result } => {
                                        if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
                                            let _ = tx.send(result);
                                        }
                                    }
                                    Incoming::Event(e) => {
                                        let _ = events.send(e);
                                    }
                                    Incoming::Ignore => {}
                                }
                            }
                        }
                        9 => {
                            let pong = encode_frame(10, &f.payload, random_bytes::<4>());
                            let _ = inner.writer.lock().await.write_all(&pong).await;
                        }
                        8 => break 'outer,
                        _ => {}
                    }
                }
                Ok(None) => break,
                Err(_) => break 'outer,
            }
        }
        match rd.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    // Wake every waiter: dropping the senders reports "connection closed".
    inner.pending.lock().unwrap().clear();
}

// ---------------------------------------------------------------------- browser process

struct BrowserProc {
    child: Option<tokio::process::Child>,
    pid: u32,
    profile: PathBuf,
    profile_removed: bool,
}

fn kill_tree(pid: u32) {
    #[cfg(unix)]
    unsafe {
        // The browser runs in its own process group: this takes its helpers with it.
        libc::kill(-(pid as i32), libc::SIGKILL);
        libc::kill(pid as i32, libc::SIGKILL);
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill").args(["/F", "/T", "/PID", &pid.to_string()]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    }
}

impl BrowserProc {
    async fn close(&mut self) {
        kill_tree(self.pid);
        if let Some(mut c) = self.child.take() {
            let _ = c.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), c.wait()).await;
        }
        for _ in 0..20 {
            if !self.profile.exists() || std::fs::remove_dir_all(&self.profile).is_ok() {
                self.profile_removed = true;
                return;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

impl Drop for BrowserProc {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            kill_tree(self.pid);
            let _ = c.start_kill();
        }
        if !self.profile_removed && self.profile.exists() {
            let dir = self.profile.clone();
            // The dying browser may still be writing; retry for a few seconds off-thread.
            let _ = std::thread::Builder::new().name("genius-browser-cleanup".into()).spawn(move || {
                for _ in 0..40 {
                    if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(250));
                }
            });
        }
    }
}

fn needs_no_sandbox() -> bool {
    if std::env::var("GENIUS_BROWSER_NO_SANDBOX").is_ok() {
        return true;
    }
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn launch_args(profile: &Path, no_sandbox: bool) -> Vec<String> {
    let mut a: Vec<String> = [
        "--headless=new",
        "--remote-debugging-port=0",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-gpu",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--disable-default-apps",
        "--disable-popup-blocking",
        "--mute-audio",
        "--hide-crash-restore-bubble",
        "--window-size=1280,800",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    a.push(format!("--user-data-dir={}", profile.display()));
    if no_sandbox {
        a.push("--no-sandbox".into());
    }
    if cfg!(target_os = "linux") {
        a.push("--disable-dev-shm-usage".into());
    }
    a.push("about:blank".into());
    a
}

/// Read `DevToolsActivePort`: the first line is the port, the second the browser WS path.
fn parse_active_port(content: &str) -> Option<(u16, String)> {
    let mut lines = content.lines();
    let port = lines.next()?.trim().parse::<u16>().ok()?;
    let path = lines.next()?.trim().to_string();
    (port != 0 && path.starts_with("/devtools/")).then_some((port, path))
}

async fn launch(exe: &Path) -> Result<(BrowserProc, u16, String)> {
    let profile = std::env::temp_dir().join(format!("genius-browser-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&profile)?;
    let stderr = std::fs::File::create(profile.join("genius-stderr.log")).map(std::process::Stdio::from).unwrap_or_else(|_| std::process::Stdio::null());
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(launch_args(&profile, needs_no_sandbox())).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(stderr).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&profile);
            return Err(proc_err(format!("could not start the browser {}: {e}", exe.display())));
        }
    };
    let pid = child.id().unwrap_or(0);
    let mut proc = BrowserProc { child: Some(child), pid, profile: profile.clone(), profile_removed: false };
    let started = Instant::now();
    loop {
        if let Ok(content) = std::fs::read_to_string(profile.join("DevToolsActivePort")) {
            if let Some((port, path)) = parse_active_port(&content) {
                return Ok((proc, port, path));
            }
        }
        if let Some(c) = proc.child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                let log = std::fs::read_to_string(profile.join("genius-stderr.log")).unwrap_or_default();
                let tail: String = log.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
                proc.close().await;
                return Err(proc_err(format!("the browser exited right after starting ({status}): {}", clip(&redact(&tail), 400))));
            }
        }
        if started.elapsed() > LAUNCH_TIMEOUT {
            proc.close().await;
            return Err(proc_err("the browser did not open its DevTools port in time"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ----------------------------------------------------------------------------- collection

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ConsoleEntry {
    /// error | warning
    pub level: String,
    pub text: String,
    /// console | log:<source> (e.g. log:security)
    pub source: String,
    pub location: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionEntry {
    pub text: String,
    pub location: Option<String>,
    pub count: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct FailedRequest {
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub error: Option<String>,
    pub resource_type: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Parsed {
    Console { level: String, text: String, location: Option<String> },
    Exception { text: String, location: Option<String> },
    LogEntry { level: String, text: String, source: String, location: Option<String> },
    RequestStart { id: String, url: String, method: String, rtype: String, redirect: bool },
    Response { id: String, status: u16, url: String },
    Finished { id: String },
    Failed { id: String, error: String, canceled: bool },
    Load,
    Dialog { message: String, kind: String },
    Paused { id: String, url: String },
    Crashed,
    Other,
}

fn remote_text(o: &Value) -> String {
    if let Some(v) = o.get("value") {
        return match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    for k in ["description", "unserializableValue"] {
        if let Some(s) = o.get(k).and_then(Value::as_str) {
            return s.to_string();
        }
    }
    o.get("type").and_then(Value::as_str).unwrap_or("").to_string()
}

fn location_of(url: Option<&str>, line: Option<i64>) -> Option<String> {
    let url = url.filter(|u| !u.is_empty())?;
    Some(match line {
        Some(l) => format!("{}:{}", tidy(url, 160), l + 1),
        None => tidy(url, 160),
    })
}

fn parse_event(method: &str, p: &Value) -> Parsed {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    match method {
        "Runtime.consoleAPICalled" => {
            let level = match p["type"].as_str().unwrap_or("") {
                "error" | "assert" => "error",
                "warning" => "warning",
                _ => return Parsed::Other,
            };
            let text = p["args"].as_array().map(|a| a.iter().map(remote_text).collect::<Vec<_>>().join(" ")).unwrap_or_default();
            let frame = &p["stackTrace"]["callFrames"][0];
            Parsed::Console { level: level.into(), text, location: location_of(frame["url"].as_str(), frame["lineNumber"].as_i64()) }
        }
        "Runtime.exceptionThrown" => {
            let d = &p["exceptionDetails"];
            let desc = d["exception"]["description"].as_str().map(|x| x.lines().take(3).collect::<Vec<_>>().join(" ")).filter(|x| !x.is_empty());
            Parsed::Exception { text: desc.unwrap_or_else(|| s(&d["text"])), location: location_of(d["url"].as_str(), d["lineNumber"].as_i64()) }
        }
        "Log.entryAdded" => {
            let e = &p["entry"];
            let level = match e["level"].as_str().unwrap_or("") {
                "error" => "error",
                "warning" => "warning",
                _ => return Parsed::Other,
            };
            let source = s(&e["source"]);
            // Network failures are recorded precisely from the Network domain.
            if source == "network" || source == "console-api" {
                return Parsed::Other;
            }
            Parsed::LogEntry { level: level.into(), text: s(&e["text"]), source, location: location_of(e["url"].as_str(), e["lineNumber"].as_i64()) }
        }
        "Network.requestWillBeSent" => Parsed::RequestStart { id: s(&p["requestId"]), url: s(&p["request"]["url"]), method: s(&p["request"]["method"]), rtype: s(&p["type"]), redirect: p.get("redirectResponse").is_some_and(|r| !r.is_null()) },
        "Network.responseReceived" => Parsed::Response { id: s(&p["requestId"]), status: p["response"]["status"].as_u64().unwrap_or(0) as u16, url: s(&p["response"]["url"]) },
        "Network.loadingFinished" => Parsed::Finished { id: s(&p["requestId"]) },
        "Network.loadingFailed" => Parsed::Failed { id: s(&p["requestId"]), error: s(&p["errorText"]), canceled: p["canceled"].as_bool().unwrap_or(false) },
        "Page.loadEventFired" => Parsed::Load,
        "Page.javascriptDialogOpening" => Parsed::Dialog { message: s(&p["message"]), kind: s(&p["type"]) },
        "Fetch.requestPaused" => Parsed::Paused { id: s(&p["requestId"]), url: s(&p["request"]["url"]) },
        "Inspector.targetCrashed" => Parsed::Crashed,
        _ => Parsed::Other,
    }
}

#[derive(Default)]
struct Collected {
    console: Vec<ConsoleEntry>,
    exceptions: Vec<ExceptionEntry>,
    failed: Vec<FailedRequest>,
    blocked: Vec<String>,
    notes: Vec<String>,
    requests: HashMap<String, (String, String, String)>,
    total_requests: u32,
    error_count: u32,
    warning_count: u32,
    inflight: i64,
    loads: u32,
    last_net: Option<Instant>,
    favicon_ignored: u32,
    crashed: bool,
}

impl Collected {
    fn apply(&mut self, p: Parsed) {
        match p {
            Parsed::Console { level, text, location } => self.add_console(level, tidy(&text, MAX_TEXT), "console".into(), location),
            Parsed::LogEntry { level, text, source, location } => self.add_console(level, tidy(&text, MAX_TEXT), format!("log:{source}"), location),
            Parsed::Exception { text, location } => {
                self.error_count += 1;
                let text = tidy(&text, MAX_TEXT);
                if let Some(e) = self.exceptions.iter_mut().find(|e| e.text == text && e.location == location) {
                    e.count += 1;
                } else if self.exceptions.len() < MAX_EXCEPTIONS {
                    self.exceptions.push(ExceptionEntry { text, location, count: 1 });
                }
            }
            Parsed::RequestStart { id, url, method, rtype, redirect } => {
                self.last_net = Some(Instant::now());
                if !redirect {
                    self.inflight += 1;
                    self.total_requests += 1;
                }
                if self.requests.len() > 2000 {
                    self.requests.clear();
                }
                self.requests.insert(id, (url, method, rtype));
            }
            Parsed::Response { id, status, url } => {
                if status >= 400 {
                    let (u, m, t) = self.requests.get(&id).cloned().unwrap_or((url.clone(), "GET".into(), String::new()));
                    let u = if u.is_empty() { url } else { u };
                    if status == 404 && u.split(['?', '#']).next().is_some_and(|x| x.ends_with("/favicon.ico")) {
                        self.favicon_ignored += 1;
                    } else {
                        self.add_failed(FailedRequest { method: m, url: tidy(&u, 200), status: Some(status), error: None, resource_type: t });
                    }
                }
            }
            Parsed::Finished { id } => {
                self.last_net = Some(Instant::now());
                self.inflight = (self.inflight - 1).max(0);
                self.requests.remove(&id);
            }
            Parsed::Failed { id, error, canceled } => {
                self.last_net = Some(Instant::now());
                self.inflight = (self.inflight - 1).max(0);
                let (u, m, t) = self.requests.remove(&id).unwrap_or_default();
                // Aborted fetches (navigation, unmount) and our own policy blocks are not app failures.
                if (canceled && error.contains("ERR_ABORTED")) || error.contains("BLOCKED_BY_CLIENT") {
                    return;
                }
                if u.split(['?', '#']).next().is_some_and(|x| x.ends_with("/favicon.ico")) {
                    self.favicon_ignored += 1;
                    return;
                }
                self.add_failed(FailedRequest { method: m, url: tidy(&u, 200), status: None, error: Some(tidy(&error, 80)), resource_type: t });
            }
            Parsed::Load => self.loads += 1,
            Parsed::Dialog { message, kind } => self.notes.push(format!("The page opened a {kind} dialog (\"{}\"); it was dismissed automatically.", tidy(&message, 120))),
            Parsed::Crashed => {
                self.crashed = true;
                self.error_count += 1;
            }
            Parsed::Paused { .. } | Parsed::Other => {}
        }
    }

    fn add_console(&mut self, level: String, text: String, source: String, location: Option<String>) {
        if level == "error" {
            self.error_count += 1;
        } else {
            self.warning_count += 1;
        }
        if let Some(e) = self.console.iter_mut().find(|e| e.level == level && e.text == text && e.source == source) {
            e.count += 1;
        } else if self.console.len() < MAX_CONSOLE {
            self.console.push(ConsoleEntry { level, text, source, location, count: 1 });
        }
    }

    fn add_failed(&mut self, f: FailedRequest) {
        if self.failed.iter().any(|x| x.method == f.method && x.url == f.url && x.status == f.status && x.error == f.error) {
            return;
        }
        if self.failed.len() < MAX_FAILED {
            self.failed.push(f);
        }
    }
}

async fn collector(cdp: Cdp, sid: String, mut rx: mpsc::UnboundedReceiver<Event>, state: Arc<Mutex<Collected>>, policy: UrlPolicy) {
    while let Some(ev) = rx.recv().await {
        if ev.session_id.as_deref() != Some(sid.as_str()) {
            continue;
        }
        match parse_event(&ev.method, &ev.params) {
            Parsed::Paused { id, url } => {
                if policy.allows(&url) {
                    let _ = cdp.call("Fetch.continueRequest", json!({ "requestId": id }), Some(&sid)).await;
                } else {
                    {
                        let mut st = state.lock().unwrap();
                        let u = tidy(&url, 200);
                        if st.blocked.len() < MAX_BLOCKED && !st.blocked.contains(&u) {
                            st.blocked.push(u);
                        }
                    }
                    let _ = cdp.call("Fetch.failRequest", json!({ "requestId": id, "errorReason": "BlockedByClient" }), Some(&sid)).await;
                }
            }
            Parsed::Dialog { message, kind } => {
                state.lock().unwrap().apply(Parsed::Dialog { message, kind });
                let _ = cdp.call("Page.handleJavaScriptDialog", json!({ "accept": true }), Some(&sid)).await;
            }
            other => state.lock().unwrap().apply(other),
        }
    }
}

// --------------------------------------------------------------------------------- steps

fn lenient<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => None,
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        other => return Err(serde::de::Error::custom(format!("expected text, got {other}"))),
    })
}

/// One scripted interaction. Locate elements with `selector` (CSS) or visible `text`/`label`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Step {
    /// click | type | select | press | wait_for | assert_text | assert_attr | assert_count |
    /// assert_no_errors | scroll | goto | reload
    pub action: String,
    pub selector: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub text: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub label: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub value: Option<String>,
    pub key: Option<String>,
    #[serde(alias = "url")]
    pub path: Option<String>,
    pub attr: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub equals: Option<String>,
    #[serde(deserialize_with = "lenient")]
    pub contains: Option<String>,
    pub count: Option<u32>,
    pub min: Option<u32>,
    pub max: Option<u32>,
    /// assert_text: the text must NOT be present. assert_attr: the value must NOT contain `contains`.
    #[serde(alias = "not")]
    pub absent: bool,
    /// wait_for: wait until the element/text is gone.
    pub gone: bool,
    /// type: append instead of replacing the field's content.
    pub append: bool,
    /// scroll: "top" | "bottom"
    pub to: Option<String>,
    pub y: Option<i64>,
    #[serde(alias = "timeout")]
    pub timeout_ms: Option<u64>,
}

const ACTIONS: &[&str] = &["click", "type", "select", "press", "wait_for", "assert_text", "assert_attr", "assert_count", "assert_no_errors", "scroll", "goto", "reload"];

impl Step {
    fn describe(&self) -> String {
        let mut parts = vec![self.action.clone()];
        for (k, v) in [("selector", &self.selector), ("text", &self.text), ("label", &self.label), ("value", &self.value), ("key", &self.key), ("path", &self.path), ("attr", &self.attr), ("equals", &self.equals), ("contains", &self.contains)] {
            if let Some(v) = v {
                parts.push(format!("{k}={:?}", clip(v, 60)));
            }
        }
        parts.join(" ")
    }

    /// Reject malformed steps before a browser is started.
    pub fn validate(&self) -> Result<()> {
        let a = self.action.to_ascii_lowercase();
        if !ACTIONS.contains(&a.as_str()) {
            return Err(GeniusError::Validation(format!("unknown step action \"{}\". Use one of: {}", self.action, ACTIONS.join(", "))));
        }
        for v in [&self.selector, &self.text, &self.label, &self.value, &self.key, &self.path, &self.attr, &self.equals, &self.contains].into_iter().flatten() {
            if v.len() > 500 {
                return Err(GeniusError::Validation("a step field is longer than 500 characters".into()));
            }
        }
        let has_loc = self.selector.is_some() || self.text.is_some() || self.label.is_some();
        let need = |ok: bool, what: &str| if ok { Ok(()) } else { Err(GeniusError::Validation(format!("step {a} needs {what}"))) };
        match a.as_str() {
            "click" => need(has_loc, "a selector or the visible text/label of the element"),
            "type" => need((self.selector.is_some() || self.label.is_some()) && (self.text.is_some() || self.value.is_some()), "a selector (or label) and the text to type"),
            "select" => need((self.selector.is_some() || self.label.is_some()) && self.value.is_some(), "a selector (or label) and a value"),
            "press" => need(self.key.is_some(), "a key (Enter, Escape, Tab, ArrowDown, a letter, ...)"),
            "wait_for" => need(has_loc, "a selector or text"),
            "assert_text" => need(self.text.is_some(), "text"),
            "assert_attr" => need(self.selector.is_some() && self.attr.is_some() && (self.equals.is_some() || self.contains.is_some()), "selector, attr and equals or contains"),
            "assert_count" => need(self.selector.is_some() && (self.count.is_some() || self.min.is_some() || self.max.is_some()), "selector and count (or min/max)"),
            "goto" => need(self.path.is_some(), "a path or url"),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct StepResult {
    pub index: usize,
    pub action: String,
    pub description: String,
    pub ok: bool,
    /// What was observed (on failure: why, plus what the page showed instead).
    pub evidence: String,
    pub ms: u64,
}

struct Key {
    key: &'static str,
    code: &'static str,
    vk: u32,
    text: Option<&'static str>,
}

fn key_params(name: &str) -> Option<(String, String, u32, Option<String>)> {
    let named: &[(&str, Key)] = &[
        ("enter", Key { key: "Enter", code: "Enter", vk: 13, text: Some("\r") }),
        ("return", Key { key: "Enter", code: "Enter", vk: 13, text: Some("\r") }),
        ("tab", Key { key: "Tab", code: "Tab", vk: 9, text: None }),
        ("escape", Key { key: "Escape", code: "Escape", vk: 27, text: None }),
        ("esc", Key { key: "Escape", code: "Escape", vk: 27, text: None }),
        ("backspace", Key { key: "Backspace", code: "Backspace", vk: 8, text: None }),
        ("delete", Key { key: "Delete", code: "Delete", vk: 46, text: None }),
        ("arrowup", Key { key: "ArrowUp", code: "ArrowUp", vk: 38, text: None }),
        ("arrowdown", Key { key: "ArrowDown", code: "ArrowDown", vk: 40, text: None }),
        ("arrowleft", Key { key: "ArrowLeft", code: "ArrowLeft", vk: 37, text: None }),
        ("arrowright", Key { key: "ArrowRight", code: "ArrowRight", vk: 39, text: None }),
        ("home", Key { key: "Home", code: "Home", vk: 36, text: None }),
        ("end", Key { key: "End", code: "End", vk: 35, text: None }),
        ("pageup", Key { key: "PageUp", code: "PageUp", vk: 33, text: None }),
        ("pagedown", Key { key: "PageDown", code: "PageDown", vk: 34, text: None }),
        ("space", Key { key: " ", code: "Space", vk: 32, text: Some(" ") }),
    ];
    let lower = name.trim().to_ascii_lowercase();
    if let Some((_, k)) = named.iter().find(|(n, _)| *n == lower) {
        return Some((k.key.into(), k.code.into(), k.vk, k.text.map(String::from)));
    }
    let mut chars = name.chars();
    let c = chars.next()?;
    if chars.next().is_none() && !c.is_control() {
        let up = c.to_ascii_uppercase();
        let (code, vk) = if c.is_ascii_alphabetic() { (format!("Key{up}"), up as u32) } else if c.is_ascii_digit() { (format!("Digit{c}"), c as u32) } else if c == ' ' { ("Space".into(), 32) } else { (String::new(), 0) };
        return Some((c.to_string(), code, vk, Some(c.to_string())));
    }
    None
}

// JavaScript run inside the page. Parameters travel as JSON, never spliced into code.

const JS_PRELUDE: &str = r#"
var ws=function(s){return String(s==null?'':s).replace(/\s+/g,' ').trim();};
var norm=function(s){return ws(s).toLowerCase();};
var vis=function(el){try{var r=el.getBoundingClientRect();var s=getComputedStyle(el);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'&&s.opacity!=='0';}catch(e){return false;}};
var desc=function(el){if(!el)return 'nothing';var d=el.tagName.toLowerCase();if(el.id)d+='#'+el.id;var t=ws(el.innerText||el.value||el.getAttribute('aria-label')||'').slice(0,40);return t?d+' "'+t+'"':d;};
var labels=function(el){var o=[];['innerText','value','placeholder'].forEach(function(k){try{var v=el[k];if(typeof v==='string'&&v.trim())o.push(norm(v));}catch(e){}});['aria-label','title','alt','name'].forEach(function(k){var v=el.getAttribute&&el.getAttribute(k);if(v)o.push(norm(v));});if(el.labels){for(var i=0;i<el.labels.length;i++)o.push(norm(el.labels[i].innerText));}return o;};
var INTER='button,a,input,select,textarea,summary,[role=button],[role=tab],[role=menuitem],[role=switch],[role=checkbox],[role=option],[tabindex]';
var find=function(spec){
  if(spec.selector){var all=[].slice.call(document.querySelectorAll(spec.selector));var v=all.filter(vis);return {el:v[0]||null,total:all.length,visible:v.length};}
  var want=norm(spec.text||spec.label);if(!want)return {el:null,total:0,visible:0};
  var best=null,bs=1e9,n=0,els=document.querySelectorAll('body *');
  for(var i=0;i<els.length&&i<6000;i++){var el=els[i];var tag=el.tagName;if(tag==='SCRIPT'||tag==='STYLE'||tag==='NOSCRIPT'||!vis(el))continue;
    var ls=labels(el);if(!ls.length)continue;
    var exact=ls.some(function(l){return l===want;});var part=exact||ls.some(function(l){return l.indexOf(want)>=0;});if(!part)continue;n++;
    var inter=el.matches(INTER);var score=(exact?0:10000)+(inter?0:2000)+Math.min.apply(null,ls.map(function(l){return l.length;}));
    if(score<bs){bs=score;best=el;}}
  return {el:best,total:n,visible:n};
};
var why=function(f,spec){return f.total>0&&f.visible===0?'matched '+f.total+' element(s) but none is visible':'no visible element matches '+(spec.selector?'selector '+spec.selector:'text "'+(spec.text||spec.label)+'"');};
"#;

const JS_LOCATE_CLICK: &str = r#"var f=find(spec);if(!f.el)return {ok:false,error:why(f,spec)};var el=f.el;
if(el.disabled||el.getAttribute('aria-disabled')==='true')return {ok:false,error:desc(el)+' is disabled'};
el.scrollIntoView({block:'center',inline:'center'});var r=el.getBoundingClientRect();var x=r.left+r.width/2,y=r.top+r.height/2;
var top=document.elementFromPoint(x,y);
if(!top||!(el===top||el.contains(top)||top.contains(el)))return {ok:false,error:desc(el)+' is covered by '+desc(top)+' at ('+Math.round(x)+', '+Math.round(y)+')'};
return {ok:true,x:x,y:y,desc:desc(el),matches:f.total};"#;

const JS_FOCUS_FIELD: &str = r#"var f=find(spec);if(!f.el)return {ok:false,error:why(f,spec)};var el=f.el;var tag=el.tagName;
if(!(tag==='INPUT'||tag==='TEXTAREA'||el.isContentEditable))return {ok:false,error:desc(el)+' is not a text field'};
if(tag==='INPUT'&&['checkbox','radio','button','submit','file','range','color','reset','image'].indexOf((el.type||'text').toLowerCase())>=0)return {ok:false,error:desc(el)+' (type '+el.type+') cannot be typed into'};
el.scrollIntoView({block:'center'});el.focus();
if(spec.append){try{var l=(el.value||'').length;el.setSelectionRange(l,l);}catch(e){}}else{try{el.select();}catch(e){try{document.execCommand('selectAll');}catch(e2){}}}
return {ok:true,desc:desc(el)};"#;

const JS_FIELD_VALUE: &str = r#"var el=document.activeElement;if(!el)return {ok:false};var v=el.value!=null?el.value:ws(el.innerText);
return {ok:true,value:String(v).slice(0,300),secret:(el.type||'')==='password'};"#;

const JS_SELECT: &str = r#"var f=find(spec);if(!f.el)return {ok:false,error:why(f,spec)};var el=f.el;
if(el.tagName!=='SELECT')return {ok:false,error:desc(el)+' is not a <select>'};
var want=ws(spec.value),opt=null;for(var i=0;i<el.options.length;i++){var o=el.options[i];if(o.value===want||ws(o.text)===want){opt=o;break;}}
if(!opt)return {ok:false,error:'no option "'+want+'"; options: '+[].map.call(el.options,function(o){return ws(o.text)||o.value;}).slice(0,12).join(', ')};
el.value=opt.value;el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));
return {ok:true,desc:desc(el),chosen:ws(opt.text)||opt.value};"#;

const JS_EXISTS: &str = r#"var f=find(spec);return {found:!!f.el,total:f.total,visible:f.visible};"#;

const JS_TEXT: &str = r#"var scope=spec.selector?document.querySelector(spec.selector):document.body;
if(!scope)return {ok:false,missing:true};var t=ws(scope.innerText||scope.textContent);
return {ok:t.indexOf(ws(spec.text))>=0,sample:t.slice(0,300)};"#;

const JS_ATTR: &str = r#"var el=document.querySelector(spec.selector);if(!el)return {missing:true};
var v=el.getAttribute(spec.attr);if(v===null&&spec.attr in el&&typeof el[spec.attr]!=='object'&&typeof el[spec.attr]!=='function')v=String(el[spec.attr]);
return {value:v===null?null:String(v).slice(0,300)};"#;

const JS_COUNT: &str = r#"var all=document.querySelectorAll(spec.selector);return {total:all.length,visible:[].filter.call(all,vis).length};"#;

const JS_SCROLL: &str = r#"var out={};
if(spec.selector||spec.text||spec.label){var f=find(spec);if(!f.el)return {ok:false,error:why(f,spec)};f.el.scrollIntoView({block:'center'});out.target=desc(f.el);}
else if(spec.to==='bottom'){window.scrollTo(0,document.documentElement.scrollHeight);}
else if(spec.to==='top'){window.scrollTo(0,0);}
else if(typeof spec.y==='number'){window.scrollBy(0,spec.y);}
else return {ok:false,error:'scroll needs a selector, text, to: top|bottom, or y'};
out.ok=true;out.y=Math.round(window.scrollY);out.max=Math.max(0,document.documentElement.scrollHeight-window.innerHeight);return out;"#;

const JS_SIG: &str = r#"var t=ws(document.body?document.body.innerText:'');var h=5381;for(var i=0;i<t.length;i++)h=((h<<5)+h+t.charCodeAt(i))|0;
return {els:document.getElementsByTagName('*').length,len:t.length,hash:h,sample:t.slice(0,300),title:document.title,url:location.href};"#;

const JS_SETTLE: &str = "new Promise(function(r){var d=false;var f=function(){if(!d){d=true;r(1);}};try{requestAnimationFrame(function(){requestAnimationFrame(function(){setTimeout(f,60);});});}catch(e){}setTimeout(f,400);})";

fn page_fn(body: &str, spec: &Value) -> String {
    format!("(function(spec){{{JS_PRELUDE}{body}}})({spec})")
}

// ---------------------------------------------------------------------------------- report

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Screenshot {
    pub label: String,
    pub width: u32,
    pub height: u32,
    /// Absolute path of the PNG on disk.
    pub path: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserReport {
    /// check | test
    pub mode: String,
    pub browser: String,
    pub url: String,
    pub duration_ms: u64,
    pub load_ms: Option<u64>,
    pub console: Vec<ConsoleEntry>,
    pub exceptions: Vec<ExceptionEntry>,
    pub failed_requests: Vec<FailedRequest>,
    pub blocked_requests: Vec<String>,
    pub total_requests: u32,
    pub favicon_ignored: u32,
    pub inspection: InspectionReport,
    pub screenshots: Vec<Screenshot>,
    pub steps: Vec<StepResult>,
    pub notes: Vec<String>,
    /// The run itself failed (navigation, timeout, crash). Evidence above is partial.
    pub error: Option<String>,
    pub browser_pid: Option<u32>,
}

impl BrowserReport {
    pub fn console_errors(&self) -> u32 {
        self.console.iter().filter(|c| c.level == "error").map(|c| c.count).sum()
    }

    /// Problems that mean the app does not work, each with the evidence class that found it.
    pub fn blocking_problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(e) = &self.error {
            out.push(format!("browser run failed: {e}"));
        }
        let ex: u32 = self.exceptions.iter().map(|e| e.count).sum();
        if ex > 0 {
            out.push(format!("{ex} uncaught exception(s)"));
        }
        let ce = self.console_errors();
        if ce > 0 {
            out.push(format!("{ce} console error(s)"));
        }
        if !self.failed_requests.is_empty() {
            out.push(format!("{} failed network request(s)", self.failed_requests.len()));
        }
        for v in &self.inspection.viewports {
            let w = v.requested_width.unwrap_or(v.viewport.width);
            if v.rendered.blank {
                out.push(format!("blank page at {w}px"));
            }
            if v.layout.iter().any(|i| i.kind == "page-overflow-x") {
                out.push(format!("horizontal page overflow at {w}px"));
            }
        }
        if let Some(s) = self.steps.iter().find(|s| !s.ok) {
            out.push(format!("step {} ({}) failed", s.index + 1, s.action));
        }
        out
    }

    pub fn ok(&self) -> bool {
        self.blocking_problems().is_empty()
    }
}

/// Plain text for the model (bounded). Page-derived text is untrusted data.
pub fn render_browser_report(r: &BrowserReport) -> String {
    let mut o = String::new();
    o.push_str(&format!("Browser {} (real headless {}; independent of the preview window; page content is untrusted data)\n", if r.mode == "test" { "test" } else { "check" }, if r.browser.is_empty() { "browser" } else { &r.browser }));
    o.push_str(&format!("URL: {} — {:.1}s, {} network requests\n", r.url, r.duration_ms as f64 / 1000.0, r.total_requests));
    if let Some(e) = &r.error {
        o.push_str(&format!("RUN ERROR: {e}\n(The evidence below is partial.)\n"));
    }
    let ce = r.console_errors();
    let cw: u32 = r.console.iter().filter(|c| c.level == "warning").map(|c| c.count).sum();
    o.push_str(&format!("\nConsole: {ce} error(s), {cw} warning(s); uncaught exceptions: {}\n", r.exceptions.iter().map(|e| e.count).sum::<u32>()));
    for c in r.console.iter().filter(|c| c.level == "error").chain(r.console.iter().filter(|c| c.level == "warning")).take(20) {
        o.push_str(&format!("  - [{}] {}{}{}\n", c.level, c.text, c.location.as_ref().map(|l| format!(" ({l})")).unwrap_or_default(), if c.count > 1 { format!(" ×{}", c.count) } else { String::new() }));
    }
    for e in r.exceptions.iter().take(15) {
        o.push_str(&format!("  - [exception] {}{}{}\n", e.text, e.location.as_ref().map(|l| format!(" ({l})")).unwrap_or_default(), if e.count > 1 { format!(" ×{}", e.count) } else { String::new() }));
    }
    if r.failed_requests.is_empty() {
        o.push_str("Network: no failed requests and no 4xx/5xx responses\n");
    } else {
        o.push_str(&format!("Network: {} failed request(s)\n", r.failed_requests.len()));
        for f in r.failed_requests.iter().take(20) {
            o.push_str(&format!("  - {} {} → {}{}\n", f.method, f.url, f.status.map(|s| s.to_string()).or_else(|| f.error.clone()).unwrap_or_else(|| "failed".into()), if f.resource_type.is_empty() { String::new() } else { format!(" [{}]", f.resource_type) }));
        }
    }
    if r.favicon_ignored > 0 {
        o.push_str(&format!("  (favicon.ico missing ×{} — ignored)\n", r.favicon_ignored));
    }
    if !r.blocked_requests.is_empty() {
        o.push_str(&format!("Blocked by the sandbox policy (only the project's own dev server may be contacted; not an app error, but the page depends on these): {}\n", r.blocked_requests.iter().take(8).cloned().collect::<Vec<_>>().join(", ")));
    }
    if !r.inspection.viewports.is_empty() {
        let mut insp = r.inspection.clone();
        if r.mode == "test" {
            // After an interaction test only the final state matters; the outline is enough.
            insp.viewports.truncate(1);
        }
        o.push('\n');
        // Console and network above come from the browser itself and are exact: drop the
        // inspector's own header and its per-page console counter.
        for line in inspection::render_inspection(&insp).lines().skip(1).filter(|l| !l.starts_with("Console since load:")) {
            o.push_str(line);
            o.push('\n');
        }
    }
    if !r.screenshots.is_empty() {
        o.push_str("\nScreenshots (PNG files on disk; not attached to this prompt, the DOM evidence above is what was verified):\n");
        for s in &r.screenshots {
            o.push_str(&format!("  - {} {}px: {}\n", s.label, s.width, s.path));
        }
    }
    if !r.steps.is_empty() {
        o.push_str("\nInteraction steps:\n");
        for s in &r.steps {
            o.push_str(&format!("  {}. {} {} — {}\n", s.index + 1, if s.ok { "PASS" } else { "FAIL" }, s.description, s.evidence));
        }
    }
    for n in &r.notes {
        o.push_str(&format!("Note: {n}\n"));
    }
    let problems = r.blocking_problems();
    let verdict = if problems.is_empty() {
        let what = if r.mode == "test" { format!("all {} step(s) passed, no console errors, exceptions or failed requests", r.steps.len()) } else { "page rendered, no console errors, exceptions or failed requests, no horizontal overflow".to_string() };
        format!("\nRESULT: OK — {what} (as observed in a real browser run).\n")
    } else {
        format!("\nRESULT: {} blocking problem(s): {}\n", problems.len(), problems.join("; "))
    };
    let mut body = o;
    if body.len() + verdict.len() > MAX_REPORT_CHARS {
        body = clip(&body, MAX_REPORT_CHARS - verdict.len());
        body.push_str("\n(report truncated)");
    }
    body.push_str(&verdict);
    body
}

// ---------------------------------------------------------------------------------- session

/// Options shared by check and test runs.
#[derive(Clone, Default)]
pub struct RunLimits {
    /// Total time for the whole run (default 60 s for a check, 90 s for a test; max 300 s).
    pub timeout: Option<Duration>,
    pub cancel: Option<CancellationToken>,
    /// Receives the browser process id once it started (tests use it to prove cleanup).
    pub pid_out: Option<Arc<AtomicU32>>,
}

/// A running headless browser with one page. Dropping it kills the browser.
pub struct Session {
    cdp: Cdp,
    sid: String,
    state: Arc<Mutex<Collected>>,
    target: Target,
    proc: BrowserProc,
    tasks: Vec<JoinHandle<()>>,
    started: Instant,
    deadline: Instant,
    cancel: Option<CancellationToken>,
    version: String,
    load_ms: Option<u64>,
    inspection: InspectionReport,
    screenshots: Vec<Screenshot>,
    steps: Vec<StepResult>,
    notes: Vec<String>,
    viewport: (u32, u32),
    shot_seq: u32,
    run_error: Option<String>,
    last_url: String,
}

impl Drop for Session {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

fn viewport_label(w: u32) -> &'static str {
    if w <= 480 {
        "mobile"
    } else if w <= 900 {
        "tablet"
    } else {
        "desktop"
    }
}

/// Widths to test: defaults when empty, clamped to 240..=2560, deduplicated, at most 4.
pub fn normalize_viewports(widths: &[u32]) -> Vec<u32> {
    let src = if widths.is_empty() { DEFAULT_VIEWPORTS } else { widths };
    let mut out: Vec<u32> = Vec::new();
    for w in src.iter().map(|w| (*w).clamp(240, 2560)) {
        if !out.contains(&w) && out.len() < MAX_VIEWPORTS {
            out.push(w);
        }
    }
    out
}

impl Session {
    /// Start the browser, open a blank page and enable the CDP domains Genius reads.
    pub async fn launch(target: Target, limits: &RunLimits, default_timeout: Duration) -> Result<Session> {
        let exe = find_browser().ok_or_else(|| GeniusError::MissingTool(NO_BROWSER_MESSAGE.into()))?;
        let total = limits.timeout.unwrap_or(default_timeout).min(MAX_TIMEOUT);
        let started = Instant::now();
        let (mut proc, port, ws_path) = launch(&exe).await?;
        if let Some(p) = &limits.pid_out {
            p.store(proc.pid, Ordering::SeqCst);
        }
        let setup = async {
            let (cdp, rx, reader) = Cdp::connect(port, &ws_path).await?;
            let mut tasks = vec![reader];
            let r: Result<(Cdp, String, String, Arc<Mutex<Collected>>)> = async {
                let version = cdp.call("Browser.getVersion", json!({}), None).await.ok().and_then(|v| v["product"].as_str().map(String::from)).unwrap_or_default();
                let t = cdp.call("Target.createTarget", json!({ "url": "about:blank" }), None).await?;
                let target_id = t["targetId"].as_str().ok_or_else(|| proc_err("the browser did not create a page"))?.to_string();
                let a = cdp.call("Target.attachToTarget", json!({ "targetId": target_id, "flatten": true }), None).await?;
                let sid = a["sessionId"].as_str().ok_or_else(|| proc_err("the browser did not attach to the page"))?.to_string();
                let state = Arc::new(Mutex::new(Collected::default()));
                tasks.push(tokio::spawn(collector(cdp.clone(), sid.clone(), rx, state.clone(), target.policy.clone())));
                for (m, p) in [("Page.enable", json!({})), ("Runtime.enable", json!({})), ("Log.enable", json!({})), ("Network.enable", json!({})), ("Network.setCacheDisabled", json!({ "cacheDisabled": true })), ("Fetch.enable", json!({ "patterns": [{ "urlPattern": "*" }] }))] {
                    cdp.call(m, p, Some(&sid)).await?;
                }
                Ok((cdp.clone(), sid, version, state))
            }
            .await;
            r.map(|(c, s, v, st)| (c, s, v, st, tasks))
        };
        match tokio::time::timeout(Duration::from_secs(30), setup).await {
            Ok(Ok((cdp, sid, version, state, tasks))) => {
                let mut s = Session {
                    cdp,
                    sid,
                    state,
                    target,
                    proc,
                    tasks,
                    started,
                    deadline: started + total,
                    cancel: limits.cancel.clone(),
                    version,
                    load_ms: None,
                    inspection: InspectionReport::default(),
                    screenshots: Vec::new(),
                    steps: Vec::new(),
                    notes: Vec::new(),
                    viewport: (1280, 800),
                    shot_seq: 0,
                    run_error: None,
                    last_url: String::new(),
                };
                // The time limit covers the run itself; a slow browser start must not eat into it.
                s.started = Instant::now();
                s.deadline = s.started + total;
                s.set_viewport(1280).await?;
                Ok(s)
            }
            Ok(Err(e)) => {
                proc.close().await;
                Err(e)
            }
            Err(_) => {
                proc.close().await;
                Err(proc_err("timed out setting up the browser"))
            }
        }
    }

    pub fn browser_pid(&self) -> u32 {
        self.proc.pid
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    async fn bounded<T>(&self, fut: impl Future<Output = Result<T>>) -> Result<T> {
        let rem = self.deadline.saturating_duration_since(Instant::now());
        if rem.is_zero() {
            return Err(proc_err(format!("the browser run exceeded its {}s time limit", (self.deadline - self.started).as_secs())));
        }
        let cancelled = async {
            match &self.cancel {
                Some(c) => c.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            r = fut => r,
            _ = tokio::time::sleep(rem) => Err(proc_err(format!("the browser run exceeded its {}s time limit", (self.deadline - self.started).as_secs()))),
            _ = cancelled => Err(GeniusError::Cancelled),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        self.bounded(self.cdp.call(method, params, Some(&self.sid))).await
    }

    async fn sleep(&self, d: Duration) -> Result<()> {
        self.bounded(async {
            tokio::time::sleep(d).await;
            Ok(())
        })
        .await
    }

    /// Evaluate JavaScript in the page and return its (JSON) value.
    pub async fn eval(&self, expression: &str) -> Result<Value> {
        let r = self.call("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true, "timeout": 15000 })).await?;
        if let Some(d) = r.get("exceptionDetails") {
            let msg = d["exception"]["description"].as_str().or_else(|| d["text"].as_str()).unwrap_or("script error");
            return Err(proc_err(format!("page script failed: {}", clip(msg, 200))));
        }
        Ok(r["result"].get("value").cloned().unwrap_or(Value::Null))
    }

    async fn eval_fn(&self, body: &str, spec: &Value) -> Result<Value> {
        self.eval(&page_fn(body, spec)).await
    }

    pub async fn set_viewport(&mut self, width: u32) -> Result<()> {
        let width = width.clamp(240, 2560);
        let height = if width >= 700 { 800 } else { 667 };
        self.call("Emulation.setDeviceMetricsOverride", json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false })).await?;
        self.viewport = (width, height);
        Ok(())
    }

    /// Load a path or allowed URL and wait for the load event and for the network to go quiet.
    pub async fn goto(&mut self, path_or_url: &str) -> Result<String> {
        let url = self.target.resolve(path_or_url)?;
        let (loads_before, t0) = (self.state.lock().unwrap().loads, Instant::now());
        let r = self.call("Page.navigate", json!({ "url": url })).await?;
        if let Some(err) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
            return Err(proc_err(format!("could not load {url}: {err}")));
        }
        let load_deadline = Instant::now() + LOAD_TIMEOUT;
        while self.state.lock().unwrap().loads <= loads_before {
            if Instant::now() > load_deadline {
                self.notes.push(format!("{} did not finish loading within {}s; continuing with what rendered.", clip(&url, 100), LOAD_TIMEOUT.as_secs()));
                break;
            }
            self.sleep(Duration::from_millis(50)).await?;
        }
        if self.load_ms.is_none() {
            self.load_ms = Some(t0.elapsed().as_millis() as u64);
        }
        // Network quiet for 400 ms (at most 4 s): late fetches and module loads finish.
        let idle_deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let quiet = {
                let st = self.state.lock().unwrap();
                st.inflight == 0 && st.last_net.is_none_or(|t| t.elapsed() > Duration::from_millis(400))
            };
            if quiet || Instant::now() > idle_deadline {
                break;
            }
            self.sleep(Duration::from_millis(50)).await?;
        }
        self.settle().await;
        self.last_url = url.clone();
        Ok(url)
    }

    /// Let pending layout/animation frames and timers run.
    async fn settle(&self) {
        let _ = self.eval(JS_SETTLE).await;
    }

    /// Run the page inspector at the current viewport and add it to the report.
    pub async fn inspect(&mut self, label: Option<&str>) -> Result<ViewportReport> {
        let expr = format!("({})(window,false).inspectPage(window)", crate::inspector::PAGE_INSPECTOR_SOURCE.trim_end());
        let v = self.eval(&expr).await?;
        let mut vr: ViewportReport = serde_json::from_value(v.clone()).map_err(|e| proc_err(format!("unreadable inspection result: {e}")))?;
        vr.label = label.map(String::from).unwrap_or_else(|| viewport_label(self.viewport.0).into());
        vr.requested_width = Some(self.viewport.0);
        if self.inspection.url.is_empty() {
            self.inspection.url = tidy(v["url"].as_str().unwrap_or(""), 300);
            self.inspection.title = tidy(v["title"].as_str().unwrap_or(""), 200);
            self.inspection.lang = v["lang"].as_str().map(String::from);
        }
        self.inspection.viewports.push(vr.clone());
        Ok(vr)
    }

    /// Save a PNG of the current viewport under the target's screenshots directory.
    pub async fn screenshot(&mut self, label: &str) -> Result<Option<Screenshot>> {
        let Some(dir) = self.target.shots_dir.clone() else { return Ok(None) };
        let r = self.call("Page.captureScreenshot", json!({ "format": "png", "fromSurface": true })).await?;
        let data = r["data"].as_str().ok_or_else(|| proc_err("the browser returned no screenshot"))?;
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| proc_err(format!("unreadable screenshot: {e}")))?;
        std::fs::create_dir_all(&dir)?;
        self.shot_seq += 1;
        let safe: String = label.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(40).collect();
        let path = dir.join(format!("{}-{}-{}.png", chrono::Utc::now().format("%Y%m%d-%H%M%S"), self.shot_seq, if safe.is_empty() { "page".to_string() } else { safe }));
        std::fs::write(&path, &bytes)?;
        prune_screenshots(&dir);
        let shot = Screenshot { label: label.to_string(), width: self.viewport.0, height: self.viewport.1, path: path.to_string_lossy().to_string(), bytes: bytes.len() };
        self.screenshots.push(shot.clone());
        Ok(Some(shot))
    }

    async fn dom_sig(&self) -> Value {
        self.eval_fn(JS_SIG, &json!({})).await.unwrap_or(Value::Null)
    }

    fn dom_change(before: &Value, after: &Value) -> String {
        if before.is_null() || after.is_null() {
            return "DOM change unknown".into();
        }
        let (be, ae) = (before["els"].as_i64().unwrap_or(0), after["els"].as_i64().unwrap_or(0));
        let text_changed = before["hash"] != after["hash"];
        if be == ae && !text_changed {
            "no visible DOM change".into()
        } else {
            format!("DOM changed: {be}→{ae} elements, visible text {}", if text_changed { format!("changed ({}→{} chars)", before["len"], after["len"]) } else { "unchanged".into() })
        }
    }

    /// Run one step; failures are results, only a broken run (timeout, crash, cancel) is `Err`.
    pub async fn run_step(&mut self, step: &Step) -> Result<StepResult> {
        let index = self.steps.len();
        let t0 = Instant::now();
        let outcome = self.exec_step(step).await?;
        let (ok, evidence) = match outcome {
            Ok(e) => (true, e),
            Err(e) => (false, e),
        };
        let r = StepResult { index, action: step.action.to_ascii_lowercase(), description: step.describe(), ok, evidence: tidy(&evidence, 500), ms: t0.elapsed().as_millis() as u64 };
        self.steps.push(r.clone());
        Ok(r)
    }

    async fn exec_step(&mut self, step: &Step) -> Result<std::result::Result<String, String>> {
        if let Err(e) = step.validate() {
            return Ok(Err(e.to_string()));
        }
        let action = step.action.to_ascii_lowercase();
        // For type/select `text` is the content, never a locator.
        let locator_text = if matches!(action.as_str(), "type" | "select") { None } else { step.text.clone() };
        let spec = json!({ "selector": step.selector, "text": locator_text, "label": step.label, "value": step.value, "append": step.append, "to": step.to, "y": step.y });
        let timeout = Duration::from_millis(step.timeout_ms.unwrap_or(5000).clamp(100, 20_000));
        match action.as_str() {
            "goto" => {
                let url = match self.target.resolve(step.path.as_deref().unwrap_or("/")) {
                    Ok(u) => u,
                    Err(e) => return Ok(Err(e.to_string())),
                };
                match self.goto(&url).await {
                    Ok(u) => Ok(Ok(format!("loaded {u}"))),
                    Err(GeniusError::Validation(m)) => Ok(Err(m)),
                    Err(e @ GeniusError::Cancelled) => Err(e),
                    Err(e) => Ok(Err(e.to_string())),
                }
            }
            "reload" => {
                let u = if self.last_url.is_empty() { "/".to_string() } else { self.last_url.clone() };
                match self.goto(&u).await {
                    Ok(u) => Ok(Ok(format!("reloaded {u}"))),
                    Err(e @ GeniusError::Cancelled) => Err(e),
                    Err(e) => Ok(Err(e.to_string())),
                }
            }
            "click" => {
                let before = self.dom_sig().await;
                let loc = self.eval_fn(JS_LOCATE_CLICK, &spec).await?;
                if loc["ok"] != json!(true) {
                    return Ok(Err(self.not_found(loc["error"].as_str().unwrap_or("could not click")).await));
                }
                let (x, y) = (loc["x"].as_f64().unwrap_or(0.0), loc["y"].as_f64().unwrap_or(0.0));
                for (t, extra) in [("mouseMoved", json!({})), ("mousePressed", json!({ "button": "left", "buttons": 1, "clickCount": 1 })), ("mouseReleased", json!({ "button": "left", "buttons": 0, "clickCount": 1 }))] {
                    let mut p = json!({ "type": t, "x": x, "y": y });
                    if let Some(o) = extra.as_object() {
                        for (k, v) in o {
                            p[k] = v.clone();
                        }
                    }
                    self.call("Input.dispatchMouseEvent", p).await?;
                }
                self.settle().await;
                let after = self.dom_sig().await;
                Ok(Ok(format!("clicked {} at ({:.0}, {:.0}); {}", loc["desc"].as_str().unwrap_or("element"), x, y, Self::dom_change(&before, &after))))
            }
            "type" => {
                let typed = step.text.clone().or_else(|| step.value.clone()).unwrap_or_default();
                let loc = self.eval_fn(JS_FOCUS_FIELD, &spec).await?;
                if loc["ok"] != json!(true) {
                    return Ok(Err(self.not_found(loc["error"].as_str().unwrap_or("could not focus the field")).await));
                }
                if typed.is_empty() {
                    self.press_key("Backspace").await?;
                } else {
                    self.call("Input.insertText", json!({ "text": typed })).await?;
                }
                self.settle().await;
                let cur = self.eval_fn(JS_FIELD_VALUE, &json!({})).await?;
                let shown = if cur["secret"] == json!(true) { "(hidden)".to_string() } else { cur["value"].as_str().unwrap_or("").to_string() };
                if !step.append && cur["secret"] != json!(true) && cur["value"].as_str() != Some(typed.as_str()) {
                    return Ok(Err(format!("typed into {} but the field contains \"{}\" instead of \"{}\"", loc["desc"].as_str().unwrap_or("field"), clip(&shown, 80), clip(&typed, 80))));
                }
                Ok(Ok(format!("typed {} chars into {}; field now contains \"{}\"", typed.chars().count(), loc["desc"].as_str().unwrap_or("field"), clip(&shown, 80))))
            }
            "select" => {
                let r = self.eval_fn(JS_SELECT, &spec).await?;
                if r["ok"] != json!(true) {
                    return Ok(Err(self.not_found(r["error"].as_str().unwrap_or("could not select")).await));
                }
                self.settle().await;
                Ok(Ok(format!("selected \"{}\" in {}", r["chosen"].as_str().unwrap_or(""), r["desc"].as_str().unwrap_or("select"))))
            }
            "press" => {
                let key = step.key.clone().unwrap_or_default();
                if key_params(&key).is_none() {
                    return Ok(Err(format!("unsupported key \"{key}\". Use Enter, Escape, Tab, Backspace, Delete, Arrow keys, Home, End, PageUp, PageDown, Space or a single character")));
                }
                if step.selector.is_some() || step.label.is_some() {
                    let loc = self.eval_fn(JS_LOCATE_CLICK, &spec).await?;
                    if loc["ok"] != json!(true) {
                        return Ok(Err(self.not_found(loc["error"].as_str().unwrap_or("could not focus")).await));
                    }
                    let _ = self.eval_fn("var f=find(spec);if(f.el&&f.el.focus)f.el.focus();return 1;", &spec).await;
                }
                let before = self.dom_sig().await;
                self.press_key(&key).await?;
                self.settle().await;
                let after = self.dom_sig().await;
                let active = self.eval_fn("return {a:desc(document.activeElement)};", &json!({})).await.ok().and_then(|v| v["a"].as_str().map(String::from)).unwrap_or_default();
                Ok(Ok(format!("pressed {key}; focus on {active}; {}", Self::dom_change(&before, &after))))
            }
            "wait_for" => {
                let end = Instant::now() + timeout;
                loop {
                    let r = self.eval_fn(JS_EXISTS, &spec).await?;
                    let found = r["found"] == json!(true);
                    if found != step.gone {
                        return Ok(Ok(if step.gone { "element is gone".to_string() } else { format!("found ({} match(es), {} visible)", r["total"], r["visible"]) }));
                    }
                    if Instant::now() > end {
                        let sig = self.dom_sig().await;
                        return Ok(Err(format!("timed out after {}ms waiting for {} {}; page shows \"{}\"", timeout.as_millis(), step.selector.clone().or(step.text.clone()).or(step.label.clone()).unwrap_or_default(), if step.gone { "to disappear" } else { "to appear" }, clip(sig["sample"].as_str().unwrap_or(""), 160))));
                    }
                    self.sleep(Duration::from_millis(100)).await?;
                }
            }
            "assert_text" => {
                let end = Instant::now() + ASSERT_RETRY;
                loop {
                    let r = self.eval_fn(JS_TEXT, &spec).await?;
                    if r["missing"] == json!(true) {
                        return Ok(Err(format!("no element matches selector {}", step.selector.clone().unwrap_or_default())));
                    }
                    let present = r["ok"] == json!(true);
                    if present != step.absent {
                        return Ok(Ok(if step.absent { format!("\"{}\" is not present", clip(step.text.as_deref().unwrap_or(""), 80)) } else { format!("found \"{}\" in the page text", clip(step.text.as_deref().unwrap_or(""), 80)) }));
                    }
                    if Instant::now() > end {
                        return Ok(Err(format!("expected {}\"{}\" but the {} text is: \"{}\"", if step.absent { "NOT to find " } else { "to find " }, clip(step.text.as_deref().unwrap_or(""), 80), step.selector.as_deref().unwrap_or("page"), clip(r["sample"].as_str().unwrap_or(""), 200))));
                    }
                    self.sleep(Duration::from_millis(100)).await?;
                }
            }
            "assert_attr" => {
                let end = Instant::now() + ASSERT_RETRY;
                let attr = step.attr.clone().unwrap_or_default();
                loop {
                    let r = self.eval_fn(JS_ATTR, &spec_with(&spec, &json!({ "selector": step.selector, "attr": attr }))).await?;
                    if r["missing"] == json!(true) {
                        return Ok(Err(format!("no element matches selector {}", step.selector.clone().unwrap_or_default())));
                    }
                    let value = r["value"].as_str();
                    let hit = match (&step.equals, &step.contains, value) {
                        (Some(eq), _, Some(v)) => v == eq,
                        (None, Some(c), Some(v)) => v.contains(c.as_str()),
                        _ => false,
                    };
                    let pass = hit != step.absent;
                    if pass {
                        return Ok(Ok(format!("{}[{}] = {}", step.selector.clone().unwrap_or_default(), attr, value.map(|v| format!("\"{}\"", clip(v, 100))).unwrap_or_else(|| "(not set)".into()))));
                    }
                    if Instant::now() > end {
                        return Ok(Err(format!("{}[{}] is {} but expected {}{}", step.selector.clone().unwrap_or_default(), attr, value.map(|v| format!("\"{}\"", clip(v, 100))).unwrap_or_else(|| "(not set)".into()), if step.absent { "NOT " } else { "" }, step.equals.as_ref().map(|e| format!("= \"{e}\"")).or(step.contains.as_ref().map(|c| format!("to contain \"{c}\""))).unwrap_or_default())));
                    }
                    self.sleep(Duration::from_millis(100)).await?;
                }
            }
            "assert_count" => {
                let end = Instant::now() + ASSERT_RETRY;
                loop {
                    let r = self.eval_fn(JS_COUNT, &spec).await?;
                    let n = r["total"].as_u64().unwrap_or(0) as u32;
                    let pass = step.count.is_none_or(|c| n == c) && step.min.is_none_or(|m| n >= m) && step.max.is_none_or(|m| n <= m);
                    if pass {
                        return Ok(Ok(format!("{} element(s) match {}", n, step.selector.clone().unwrap_or_default())));
                    }
                    if Instant::now() > end {
                        return Ok(Err(format!("{} element(s) match {} but expected {}", n, step.selector.clone().unwrap_or_default(), [step.count.map(|c| format!("exactly {c}")), step.min.map(|c| format!("at least {c}")), step.max.map(|c| format!("at most {c}"))].into_iter().flatten().collect::<Vec<_>>().join(" and "))));
                    }
                    self.sleep(Duration::from_millis(100)).await?;
                }
            }
            "assert_no_errors" => {
                let st = self.state.lock().unwrap();
                let (e, f) = (st.error_count, st.failed.len());
                let first = st.exceptions.first().map(|x| x.text.clone()).or_else(|| st.console.iter().find(|c| c.level == "error").map(|c| c.text.clone())).unwrap_or_default();
                if e == 0 && f == 0 {
                    Ok(Ok("no console errors, exceptions or failed requests so far".into()))
                } else {
                    Ok(Err(format!("{e} console error(s)/exception(s) and {f} failed request(s) so far; first: {}", clip(&first, 160))))
                }
            }
            "scroll" => {
                let r = self.eval_fn(JS_SCROLL, &spec).await?;
                if r["ok"] != json!(true) {
                    return Ok(Err(self.not_found(r["error"].as_str().unwrap_or("could not scroll")).await));
                }
                self.settle().await;
                Ok(Ok(format!("scrolled{} to y={} of {}", r["target"].as_str().map(|t| format!(" to {t}")).unwrap_or_default(), r["y"], r["max"])))
            }
            _ => Ok(Err(format!("unknown action {action}"))),
        }
    }

    async fn not_found(&self, why: &str) -> String {
        let sig = self.dom_sig().await;
        format!("{why}; the page currently shows: \"{}\"", clip(sig["sample"].as_str().unwrap_or(""), 160))
    }

    async fn press_key(&self, key: &str) -> Result<()> {
        let (k, code, vk, text) = key_params(key).ok_or_else(|| proc_err(format!("unsupported key {key}")))?;
        let mut down = json!({ "type": if text.is_some() { "keyDown" } else { "rawKeyDown" }, "key": k, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk });
        if let Some(t) = &text {
            down["text"] = json!(t);
            down["unmodifiedText"] = json!(t);
        }
        self.call("Input.dispatchKeyEvent", down).await?;
        self.call("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": k, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk })).await?;
        Ok(())
    }

    /// Everything observed so far, without closing the browser.
    pub fn report(&self, mode: &str) -> BrowserReport {
        let st = self.state.lock().unwrap();
        let mut insp = self.inspection.clone();
        insp.console = ConsoleCounts { errors: st.error_count, warnings: st.warning_count };
        let mut notes = self.notes.clone();
        notes.extend(st.notes.iter().cloned());
        if st.crashed {
            notes.push("The page crashed during the run.".into());
        }
        BrowserReport {
            mode: mode.into(),
            browser: self.version.clone(),
            url: self.last_url.clone(),
            duration_ms: self.started.elapsed().as_millis() as u64,
            load_ms: self.load_ms,
            console: st.console.clone(),
            exceptions: st.exceptions.clone(),
            failed_requests: st.failed.clone(),
            blocked_requests: st.blocked.clone(),
            total_requests: st.total_requests,
            favicon_ignored: st.favicon_ignored,
            inspection: inspection::sanitize(insp),
            screenshots: self.screenshots.clone(),
            steps: self.steps.clone(),
            notes,
            error: self.run_error.clone(),
            browser_pid: Some(self.proc.pid),
        }
    }

    /// Record that the run itself failed (kept in the report as partial evidence).
    pub fn fail(&mut self, e: &GeniusError) {
        self.run_error = Some(tidy(&e.to_string(), 300));
    }

    /// Final report; the browser and its profile are removed before this returns.
    pub async fn finish(mut self, mode: &str) -> BrowserReport {
        let r = self.report(mode);
        self.proc.close().await;
        r
    }
}

fn spec_with(base: &Value, extra: &Value) -> Value {
    let mut out = base.clone();
    if let (Some(o), Some(e)) = (out.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    out
}

fn prune_screenshots(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "png")).collect();
    if files.len() <= KEEP_SCREENSHOTS {
        return;
    }
    files.sort();
    for p in &files[..files.len() - KEEP_SCREENSHOTS] {
        let _ = std::fs::remove_file(p);
    }
}

// ------------------------------------------------------------------------- check and test

#[derive(Clone, Default)]
pub struct CheckOptions {
    /// Page to open (default "/").
    pub path: Option<String>,
    /// Widths to test (default 375, 768, 1280).
    pub viewports: Vec<u32>,
    pub screenshot: bool,
    pub limits: RunLimits,
}

#[derive(Clone, Default)]
pub struct TestOptions {
    pub path: Option<String>,
    pub steps: Vec<Step>,
    pub viewport: Option<u32>,
    /// Save a screenshot at the end (always saved when a step fails).
    pub screenshot: bool,
    pub limits: RunLimits,
}

/// Open the page at each viewport and collect console, network, inspector and screenshot
/// evidence. Errors only when the browser could not run at all; a failure during the run is
/// reported inside the (partial) report.
pub async fn run_check(target: &Target, opts: &CheckOptions) -> Result<BrowserReport> {
    let widths = normalize_viewports(&opts.viewports);
    let start = target.resolve(opts.path.as_deref().unwrap_or("/"))?;
    let mut s = Session::launch(target.clone(), &opts.limits, CHECK_TIMEOUT).await?;
    let res: Result<()> = async {
        for w in &widths {
            s.set_viewport(*w).await?;
            s.goto(&start).await?;
            let vr = s.inspect(None).await?;
            if opts.screenshot {
                s.screenshot(&format!("{}-{}", vr.label, w)).await?;
            }
        }
        Ok(())
    }
    .await;
    finish_run(s, res, "check").await
}

/// Open the page and run scripted interactions, stopping at the first failing step.
pub async fn run_test(target: &Target, opts: &TestOptions) -> Result<BrowserReport> {
    if opts.steps.is_empty() {
        return Err(GeniusError::Validation("browser_test needs at least one step".into()));
    }
    if opts.steps.len() > MAX_STEPS {
        return Err(GeniusError::Validation(format!("browser_test accepts at most {MAX_STEPS} steps per call")));
    }
    for (i, st) in opts.steps.iter().enumerate() {
        st.validate().map_err(|e| GeniusError::Validation(format!("step {}: {e}", i + 1)))?;
        if matches!(st.action.to_ascii_lowercase().as_str(), "goto") {
            target.resolve(st.path.as_deref().unwrap_or("/")).map_err(|e| GeniusError::Validation(format!("step {}: {e}", i + 1)))?;
        }
    }
    let start = target.resolve(opts.path.as_deref().unwrap_or("/"))?;
    let mut s = Session::launch(target.clone(), &opts.limits, TEST_TIMEOUT).await?;
    let res: Result<()> = async {
        s.set_viewport(opts.viewport.unwrap_or(1280)).await?;
        s.goto(&start).await?;
        let mut failed = false;
        for st in &opts.steps {
            let r = s.run_step(st).await?;
            if !r.ok {
                failed = true;
                break;
            }
        }
        // Final state: outline, blank/overflow check, and a screenshot on failure or request.
        let _ = s.inspect(Some("final")).await;
        if failed || opts.screenshot {
            let _ = s.screenshot(if failed { "failed-step" } else { "final" }).await;
        }
        Ok(())
    }
    .await;
    finish_run(s, res, "test").await
}

async fn finish_run(mut s: Session, res: Result<()>, mode: &str) -> Result<BrowserReport> {
    match res {
        Ok(()) => Ok(s.finish(mode).await),
        Err(GeniusError::Cancelled) => {
            s.proc.close().await;
            Err(GeniusError::Cancelled)
        }
        Err(e) => {
            s.fail(&e);
            Ok(s.finish(mode).await)
        }
    }
}

// ---------------------------------------------------------------------------- Genius API

/// Arguments of the agent's `browser_check` tool.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CheckArgs {
    #[serde(alias = "url")]
    pub path: Option<String>,
    #[serde(alias = "widths", alias = "viewport")]
    pub viewports: Option<Viewports>,
    pub screenshot: Option<bool>,
}

/// A number or a list of numbers.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Viewports {
    One(u32),
    Many(Vec<u32>),
}

impl Viewports {
    pub fn list(&self) -> Vec<u32> {
        match self {
            Viewports::One(w) => vec![*w],
            Viewports::Many(v) => v.clone(),
        }
    }
}

/// Arguments of the agent's `browser_test` tool.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestArgs {
    pub steps: Vec<Step>,
    #[serde(default, alias = "width")]
    pub viewport: Option<u32>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub screenshot: Option<bool>,
}

impl Genius {
    /// Is a Chromium-family browser installed? (Without one no browser check can run.)
    pub fn browser_available(&self) -> bool {
        find_browser().is_some()
    }

    /// The browser Genius would launch, for display.
    pub fn browser_path(&self) -> Option<PathBuf> {
        find_browser()
    }

    /// Build the safe target for `url`: it must be the project's own running dev server or a
    /// file inside the project's `dist`.
    pub fn browser_target(&self, project_id: &str, url: &str) -> Result<Target> {
        let ws = self.workspace(project_id)?;
        let dist = ws.root().join("dist");
        let u = Url::parse(url).map_err(|e| GeniusError::Validation(format!("invalid URL {url}: {e}")))?;
        let policy = match u.scheme() {
            "file" => UrlPolicy::for_files(&dist),
            _ => {
                let dev = self.preview_url(project_id).ok_or_else(|| GeniusError::Validation("The project's dev server is not running, so there is nothing to open.".into()))?;
                UrlPolicy::for_origin(&dev)?.with_file_root(&dist)
            }
        };
        let shots = self.data_dir().join("screenshots").join(project_id);
        Target::new(url, policy, Some(shots))
    }

    /// The running preview's URL, starting the dev server first when needed.
    pub async fn ensure_preview(&self, project_id: &str, task_id: Option<&str>) -> Result<String> {
        if let Some(u) = self.preview_url(project_id) {
            return Ok(u);
        }
        self.start_preview(project_id, task_id).await.map_err(|e| GeniusError::Validation(format!("The dev server could not be started, so the browser check could not run: {e}")))
    }

    pub async fn browser_check(&self, project_id: &str, url: &str, opts: CheckOptions) -> Result<BrowserReport> {
        self.db.get_project(project_id)?;
        let target = self.browser_target(project_id, url)?;
        run_check(&target, &opts).await
    }

    pub async fn browser_test(&self, project_id: &str, url: &str, opts: TestOptions) -> Result<BrowserReport> {
        self.db.get_project(project_id)?;
        let target = self.browser_target(project_id, url)?;
        run_test(&target, &opts).await
    }

    pub fn render_browser_report(&self, r: &BrowserReport) -> String {
        render_browser_report(r)
    }

    /// Agent tool `browser_check`: (summary, text for the model, structured data).
    pub(crate) async fn tool_browser_check(&self, project_id: &str, task_id: &str, a: CheckArgs, cancel: &CancellationToken) -> Result<(String, String, Value)> {
        if !self.browser_available() {
            return Err(GeniusError::MissingTool(NO_BROWSER_MESSAGE.into()));
        }
        let base = self.ensure_preview(project_id, Some(task_id)).await?;
        let opts = CheckOptions { path: a.path, viewports: a.viewports.map(|v| v.list()).unwrap_or_default(), screenshot: a.screenshot.unwrap_or(true), limits: RunLimits { cancel: Some(cancel.clone()), ..Default::default() } };
        let r = self.browser_check(project_id, &base, opts).await?;
        Ok(report_outcome(&r))
    }

    /// Agent tool `browser_test`.
    pub(crate) async fn tool_browser_test(&self, project_id: &str, task_id: &str, a: TestArgs, cancel: &CancellationToken) -> Result<(String, String, Value)> {
        if !self.browser_available() {
            return Err(GeniusError::MissingTool(NO_BROWSER_MESSAGE.into()));
        }
        let base = self.ensure_preview(project_id, Some(task_id)).await?;
        let opts = TestOptions { path: a.path, steps: a.steps, viewport: a.viewport, screenshot: a.screenshot.unwrap_or(false), limits: RunLimits { cancel: Some(cancel.clone()), ..Default::default() } };
        let r = self.browser_test(project_id, &base, opts).await?;
        Ok(report_outcome(&r))
    }
}

fn report_outcome(r: &BrowserReport) -> (String, String, Value) {
    let problems = r.blocking_problems();
    let what = if r.mode == "test" { format!("Ran a {}-step browser test", r.steps.len()) } else { format!("Checked the app in a real browser at {} width(s)", r.inspection.viewports.len()) };
    let summary = if problems.is_empty() { format!("{what}: no blocking problems") } else { format!("{what}: {} blocking problem(s)", problems.len()) };
    let data = json!({
        "ok": problems.is_empty(),
        "problems": problems,
        "consoleErrors": r.console_errors(),
        "exceptions": r.exceptions.len(),
        "failedRequests": r.failed_requests.len(),
        "screenshots": r.screenshots.iter().map(|s| s.path.clone()).collect::<Vec<_>>(),
        "steps": r.steps.iter().map(|s| json!({ "action": s.action, "ok": s.ok })).collect::<Vec<_>>(),
    });
    (summary, render_browser_report(r), data)
}

// ------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn discovery_lists_platform_locations_in_order() {
        let win = browser_candidates(&env_of(&[("ProgramFiles(x86)", r"C:\Program Files (x86)"), ("ProgramFiles", r"C:\Program Files"), ("GENIUS_BROWSER", r"D:\b\chrome.exe")]), "windows");
        let s: Vec<String> = win.iter().map(|p| p.to_string_lossy().replace('/', "\\")).collect();
        assert_eq!(s[0], r"D:\b\chrome.exe", "GENIUS_BROWSER wins");
        assert!(s[1].contains("Program Files (x86)") && s[1].ends_with(r"Microsoft\Edge\Application\msedge.exe"), "{s:?}");
        assert!(s.iter().any(|p| p.ends_with(r"Google\Chrome\Application\chrome.exe") && p.starts_with(r"C:\Program Files\")));
        let mac = browser_candidates(&env_of(&[]), "macos");
        assert!(mac[0].to_string_lossy().contains("Google Chrome.app") && mac[1].to_string_lossy().contains("Microsoft Edge.app"));
        let lin = browser_candidates(&env_of(&[("PLAYWRIGHT_BROWSERS_PATH", "/nonexistent-pw")]), "linux");
        assert!(lin.iter().any(|p| p.ends_with("usr/bin/chromium")) && lin.iter().any(|p| p.ends_with("google-chrome")));
        assert!(browser_candidates(&env_of(&[("GENIUS_BROWSER", "  ")]), "linux").iter().all(|p| !p.as_os_str().is_empty()));
    }

    #[test]
    fn playwright_cache_is_scanned_newest_first() {
        let d = std::env::temp_dir().join(format!("genius-pw-test-{}", uuid::Uuid::new_v4()));
        for v in ["chromium-1100/chrome-linux", "chromium-1194/chrome-linux64", "chromium_headless_shell-1194/chrome-linux", "ffmpeg-1/x"] {
            std::fs::create_dir_all(d.join(v)).unwrap();
        }
        let found = playwright_chromiums(&d);
        let _ = std::fs::remove_dir_all(&d);
        let s: Vec<String> = found.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(s.len(), 2, "{s:?}");
        assert!(s[0].contains("chromium-1194/chrome-linux64") && s[1].contains("chromium-1100"), "{s:?}");
        assert!(!s.iter().any(|p| p.contains("headless_shell")));
        assert!(playwright_chromiums(Path::new("/definitely/not/here")).is_empty());
    }

    #[test]
    fn url_policy_allows_only_the_dev_origin_and_dist_files() {
        let p = UrlPolicy::for_origin("http://127.0.0.1:5173/").unwrap();
        for ok in ["http://127.0.0.1:5173/", "http://127.0.0.1:5173/src/main.tsx?t=1", "http://localhost:5173/x", "ws://127.0.0.1:5173/?token=a", "data:image/png;base64,AAAA", "about:blank", "blob:http://127.0.0.1:5173/uuid"] {
            assert!(p.allows(ok), "{ok}");
        }
        for bad in ["http://127.0.0.1:5174/", "http://127.0.0.1/", "https://127.0.0.1:5173/", "http://example.com/", "http://127.0.0.1.evil.com:5173/", "http://127.0.0.1:5173@evil.com/", "http://user:pw@127.0.0.1:5173/", "http://evil.com:5173/", "ftp://127.0.0.1:5173/", "file:///etc/passwd", "javascript:alert(1)", "chrome://settings", "not a url", "http://[::2]:5173/", "http://0.0.0.0:5173/"] {
            assert!(!p.allows(bad), "{bad}");
        }
        assert!(UrlPolicy::for_origin("https://example.com:443/").is_err());
        assert!(UrlPolicy::for_origin("http://example.com:5173/").is_err());
        assert!(UrlPolicy::for_origin("http://127.0.0.1/").is_err(), "an explicit port is required");
    }

    #[test]
    fn file_urls_are_confined_to_dist() {
        let d = std::env::temp_dir().join(format!("genius-dist-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(d.join("dist/assets")).unwrap();
        std::fs::write(d.join("dist/index.html"), "x").unwrap();
        std::fs::write(d.join("secret.txt"), "x").unwrap();
        let p = UrlPolicy::for_files(&d.join("dist"));
        let root = d.canonicalize().unwrap();
        let f = |rel: &str| Url::from_file_path(root.join(rel)).unwrap().to_string();
        assert!(p.allows(&f("dist/index.html")));
        assert!(p.allows(&f("dist/assets/app.js")), "not-yet-existing files inside dist are fine");
        assert!(!p.allows(&f("secret.txt")));
        assert!(!p.allows(&format!("{}/../secret.txt", f("dist").trim_end_matches('/'))));
        assert!(!p.allows("http://127.0.0.1:5173/"), "a file policy has no http origin");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn target_resolve_rejects_off_origin_navigation() {
        let policy = UrlPolicy::for_origin("http://127.0.0.1:4000/").unwrap();
        let t = Target::new("http://127.0.0.1:4000/", policy, None).unwrap();
        assert_eq!(t.resolve("/").unwrap(), "http://127.0.0.1:4000/");
        assert_eq!(t.resolve("").unwrap(), "http://127.0.0.1:4000/");
        assert_eq!(t.resolve("settings?tab=1").unwrap(), "http://127.0.0.1:4000/settings?tab=1");
        assert_eq!(t.resolve("http://localhost:4000/a").unwrap(), "http://localhost:4000/a");
        for bad in ["//evil.com/x", "https://evil.com/", "http://127.0.0.1:9/", "file:///etc/passwd", "javascript:alert(1)", "data:text/html,<script>1</script>", "http://evil.com\\@127.0.0.1:4000/"] {
            let e = t.resolve(bad);
            assert!(e.is_err(), "{bad} -> {e:?}");
        }
        assert!(Target::new("http://example.com/", UrlPolicy::for_origin("http://127.0.0.1:4000/").unwrap(), None).is_err());
    }

    #[test]
    fn websocket_frames_round_trip_and_handle_partial_input() {
        for len in [0usize, 5, 125, 126, 300, 70_000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let f = encode_frame(1, &payload, [1, 2, 3, 4]);
            // every strict prefix needs more bytes
            if len < 200 {
                for cut in 0..f.len() {
                    assert_eq!(parse_frame(&f[..cut]).unwrap(), None, "len {len} cut {cut}");
                }
            }
            let (frame, used) = parse_frame(&f).unwrap().unwrap();
            assert_eq!(used, f.len());
            assert_eq!((frame.fin, frame.opcode, frame.payload), (true, 1, payload));
        }
        // unmasked server frame + trailing bytes of the next one
        let mut buf = vec![0x81, 0x02, b'h', b'i', 0x81];
        let (f, n) = parse_frame(&buf).unwrap().unwrap();
        assert_eq!((f.payload.as_slice(), n), (&b"hi"[..], 4));
        buf.drain(..n);
        assert_eq!(parse_frame(&buf).unwrap(), None);
        // oversize announcement is an error, not an allocation
        let mut big = vec![0x81, 127];
        big.extend((u64::MAX).to_be_bytes());
        assert!(parse_frame(&big).is_err());
    }

    #[test]
    fn cdp_messages_parse_into_responses_and_events() {
        match parse_incoming(r#"{"id":7,"result":{"targetId":"T"}}"#) {
            Incoming::Response { id, result } => assert_eq!((id, result.unwrap()["targetId"].clone()), (7, json!("T"))),
            _ => panic!(),
        }
        match parse_incoming(r#"{"id":8,"error":{"code":-32000,"message":"No target"}}"#) {
            Incoming::Response { id, result } => assert_eq!((id, result.unwrap_err()), (8, "No target".to_string())),
            _ => panic!(),
        }
        match parse_incoming(r#"{"method":"Page.loadEventFired","params":{"timestamp":1},"sessionId":"S1"}"#) {
            Incoming::Event(e) => assert_eq!((e.method.as_str(), e.session_id.as_deref()), ("Page.loadEventFired", Some("S1"))),
            _ => panic!(),
        }
        assert!(matches!(parse_incoming("not json"), Incoming::Ignore));
        assert!(matches!(parse_incoming(r#"{"foo":1}"#), Incoming::Ignore));
    }

    #[test]
    fn events_are_classified_and_collected() {
        let console = parse_event("Runtime.consoleAPICalled", &json!({ "type": "error", "args": [{ "type": "string", "value": "boom" }, { "type": "number", "value": 3 }], "stackTrace": { "callFrames": [{ "url": "http://127.0.0.1:1/app.js", "lineNumber": 9 }] } }));
        assert_eq!(console, Parsed::Console { level: "error".into(), text: "boom 3".into(), location: Some("http://127.0.0.1:1/app.js:10".into()) });
        assert_eq!(parse_event("Runtime.consoleAPICalled", &json!({ "type": "log", "args": [] })), Parsed::Other);
        assert!(matches!(parse_event("Runtime.consoleAPICalled", &json!({ "type": "assert", "args": [] })), Parsed::Console { .. }));
        let ex = parse_event("Runtime.exceptionThrown", &json!({ "exceptionDetails": { "text": "Uncaught", "url": "u.js", "lineNumber": 1, "exception": { "description": "Error: late\n    at x\n    at y\n    at z" } } }));
        assert_eq!(ex, Parsed::Exception { text: "Error: late     at x     at y".into(), location: Some("u.js:2".into()) });
        assert_eq!(parse_event("Log.entryAdded", &json!({ "entry": { "source": "network", "level": "error", "text": "Failed to load resource" } })), Parsed::Other, "network failures come from the Network domain");
        assert!(matches!(parse_event("Log.entryAdded", &json!({ "entry": { "source": "security", "level": "warning", "text": "mixed" } })), Parsed::LogEntry { .. }));
        assert_eq!(parse_event("Fetch.requestPaused", &json!({ "requestId": "F1", "request": { "url": "http://x/" } })), Parsed::Paused { id: "F1".into(), url: "http://x/".into() });

        let mut c = Collected::default();
        for p in [
            parse_event("Network.requestWillBeSent", &json!({ "requestId": "1", "type": "Fetch", "request": { "url": "http://127.0.0.1:1/api/missing", "method": "GET" } })),
            parse_event("Network.responseReceived", &json!({ "requestId": "1", "response": { "status": 404, "url": "http://127.0.0.1:1/api/missing" } })),
            parse_event("Network.loadingFinished", &json!({ "requestId": "1" })),
            parse_event("Network.requestWillBeSent", &json!({ "requestId": "2", "type": "Image", "request": { "url": "http://127.0.0.1:1/favicon.ico", "method": "GET" } })),
            parse_event("Network.responseReceived", &json!({ "requestId": "2", "response": { "status": 404, "url": "http://127.0.0.1:1/favicon.ico" } })),
            parse_event("Network.loadingFinished", &json!({ "requestId": "2" })),
            parse_event("Network.requestWillBeSent", &json!({ "requestId": "3", "type": "Script", "request": { "url": "http://127.0.0.1:1/x.js", "method": "GET" } })),
            parse_event("Network.loadingFailed", &json!({ "requestId": "3", "errorText": "net::ERR_CONNECTION_REFUSED", "canceled": false })),
            parse_event("Network.requestWillBeSent", &json!({ "requestId": "4", "type": "Fetch", "request": { "url": "http://127.0.0.1:1/aborted", "method": "GET" } })),
            parse_event("Network.loadingFailed", &json!({ "requestId": "4", "errorText": "net::ERR_ABORTED", "canceled": true })),
            parse_event("Network.requestWillBeSent", &json!({ "requestId": "5", "type": "Image", "request": { "url": "https://cdn.example/a.png", "method": "GET" } })),
            parse_event("Network.loadingFailed", &json!({ "requestId": "5", "errorText": "net::ERR_BLOCKED_BY_CLIENT", "canceled": false })),
            console.clone(),
            console.clone(),
            ex,
            Parsed::Load,
        ] {
            c.apply(p);
        }
        assert_eq!(c.failed.len(), 2, "{:?}", c.failed);
        assert_eq!((c.failed[0].status, c.failed[0].url.as_str()), (Some(404), "http://127.0.0.1:1/api/missing"));
        assert_eq!(c.failed[1].error.as_deref(), Some("net::ERR_CONNECTION_REFUSED"));
        assert_eq!(c.favicon_ignored, 1);
        assert_eq!(c.console.len(), 1, "duplicates collapse");
        assert_eq!(c.console[0].count, 2);
        assert_eq!(c.error_count, 3, "2 console errors + 1 exception");
        assert_eq!((c.inflight, c.loads, c.total_requests), (0, 1, 5));
    }

    #[test]
    fn collected_text_is_redacted_and_bounded() {
        let mut c = Collected::default();
        c.apply(Parsed::Console { level: "error".into(), text: format!("key nvapi-abcdefghijklmnopqrstuvwxyz0123456789 {}", "x".repeat(2000)), location: None });
        assert!(c.console[0].text.len() <= MAX_TEXT + 4);
        assert!(!c.console[0].text.contains("nvapi-abcdefghijklmnopqrstuvwxyz0123456789"));
        for i in 0..(MAX_CONSOLE + 20) {
            c.apply(Parsed::Console { level: "warning".into(), text: format!("w{i}"), location: None });
        }
        assert_eq!(c.console.len(), MAX_CONSOLE);
        assert_eq!(c.warning_count as usize, MAX_CONSOLE + 20, "counts stay exact when entries are capped");
    }

    #[test]
    fn steps_accept_loose_json_and_validate() {
        let s: Step = serde_json::from_value(json!({ "action": "type", "selector": "#n", "text": 42 })).unwrap();
        assert_eq!(s.text.as_deref(), Some("42"));
        assert!(s.validate().is_ok());
        let s: Step = serde_json::from_value(json!({ "action": "goto", "url": "/a", "timeout": 100 })).unwrap();
        assert_eq!((s.path.as_deref(), s.timeout_ms), (Some("/a"), Some(100)));
        let s: Step = serde_json::from_value(json!({ "action": "assert_text", "text": "x", "not": true })).unwrap();
        assert!(s.absent);
        for (v, needle) in [
            (json!({ "action": "hover" }), "unknown step action"),
            (json!({ "action": "click" }), "selector"),
            (json!({ "action": "type", "selector": "#a" }), "text to type"),
            (json!({ "action": "press" }), "key"),
            (json!({ "action": "assert_attr", "selector": "a" }), "attr"),
            (json!({ "action": "assert_count", "selector": "li" }), "count"),
            (json!({ "action": "click", "text": "x".repeat(600) }), "500"),
        ] {
            let s: Step = serde_json::from_value(v).unwrap();
            let e = s.validate().unwrap_err().to_string();
            assert!(e.contains(needle), "{e}");
        }
        assert!(serde_json::from_value::<Step>(json!({ "action": "click", "text": { "a": 1 } })).is_err());
    }

    #[test]
    fn key_names_map_to_cdp_key_events() {
        assert_eq!(key_params("Enter").unwrap(), ("Enter".into(), "Enter".into(), 13, Some("\r".into())));
        assert_eq!(key_params("escape").unwrap().2, 27);
        assert_eq!(key_params("ArrowDown").unwrap().3, None);
        assert_eq!(key_params("a").unwrap(), ("a".into(), "KeyA".into(), 65, Some("a".into())));
        assert_eq!(key_params("7").unwrap().1, "Digit7");
        assert_eq!(key_params(" ").unwrap().1, "Space");
        assert!(key_params("Ctrl+A").is_none() && key_params("").is_none());
    }

    #[test]
    fn viewports_are_normalised() {
        assert_eq!(normalize_viewports(&[]), vec![375, 768, 1280]);
        assert_eq!(normalize_viewports(&[10, 375, 375, 99999, 800, 900, 1000]), vec![240, 375, 2560, 800]);
        assert_eq!(viewport_label(375), "mobile");
        assert_eq!(viewport_label(768), "tablet");
        assert_eq!(viewport_label(1280), "desktop");
    }

    #[test]
    fn launch_args_are_headless_and_sandbox_only_when_needed() {
        let a = launch_args(Path::new("/tmp/p"), false);
        assert!(a.contains(&"--headless=new".to_string()) && a.contains(&"--remote-debugging-port=0".to_string()));
        assert!(a.iter().any(|x| x == "--user-data-dir=/tmp/p"));
        assert!(!a.contains(&"--no-sandbox".to_string()));
        assert!(launch_args(Path::new("/tmp/p"), true).contains(&"--no-sandbox".to_string()));
        assert_eq!(parse_active_port("9222\n/devtools/browser/abc-123\n"), Some((9222, "/devtools/browser/abc-123".into())));
        assert_eq!(parse_active_port("9222\n"), None);
        assert_eq!(parse_active_port("0\n/devtools/browser/x"), None);
    }

    fn sample_report() -> BrowserReport {
        let mut r = BrowserReport { mode: "check".into(), browser: "HeadlessChrome/141".into(), url: "http://127.0.0.1:1/".into(), duration_ms: 2500, total_requests: 7, ..Default::default() };
        r.console.push(ConsoleEntry { level: "error".into(), text: "boom".into(), source: "console".into(), location: Some("app.js:3".into()), count: 2 });
        r.exceptions.push(ExceptionEntry { text: "Error: late".into(), location: None, count: 1 });
        r.failed_requests.push(FailedRequest { method: "GET".into(), url: "http://127.0.0.1:1/missing".into(), status: Some(404), error: None, resource_type: "Fetch".into() });
        r.blocked_requests.push("https://fonts.example/css".into());
        r.screenshots.push(Screenshot { label: "mobile".into(), width: 375, height: 667, path: "/data/screenshots/p/a.png".into(), bytes: 10 });
        r.steps.push(StepResult { index: 0, action: "click".into(), description: "click text=\"Go\"".into(), ok: false, evidence: "no visible element matches".into(), ms: 3 });
        r
    }

    #[test]
    fn report_rendering_lists_evidence_and_the_verdict() {
        let r = sample_report();
        let t = render_browser_report(&r);
        for needle in ["real headless HeadlessChrome/141", "1 failed request", "GET http://127.0.0.1:1/missing → 404", "[exception] Error: late", "[error] boom (app.js:3) ×2", "Blocked by the sandbox policy", "/data/screenshots/p/a.png", "1. FAIL click", "RESULT: ", "uncaught exception(s)", "failed network request(s)", "step 1 (click) failed"] {
            assert!(t.contains(needle), "missing {needle:?} in:\n{t}");
        }
        assert!(!r.ok());
        let clean = BrowserReport { mode: "check".into(), url: "http://x/".into(), ..Default::default() };
        let t = render_browser_report(&clean);
        assert!(t.contains("RESULT: OK") && t.contains("no failed requests"), "{t}");
        assert!(clean.ok());
        let mut run_failed = clean.clone();
        run_failed.error = Some("could not load".into());
        assert!(render_browser_report(&run_failed).contains("RUN ERROR: could not load") && !run_failed.ok());
    }

    #[test]
    fn blank_pages_and_horizontal_overflow_block() {
        let mut r = BrowserReport::default();
        let mut v = ViewportReport { requested_width: Some(375), ..Default::default() };
        v.layout.push(inspection::Issue { kind: "page-overflow-x".into(), message: "wide".into(), selector: None });
        r.inspection.viewports.push(v);
        r.inspection.viewports.push(ViewportReport { requested_width: Some(1280), rendered: inspection::Rendered { blank: true, ..Default::default() }, ..Default::default() });
        let p = r.blocking_problems();
        assert!(p.iter().any(|x| x == "horizontal page overflow at 375px") && p.iter().any(|x| x == "blank page at 1280px"), "{p:?}");
    }

    #[test]
    fn long_reports_are_bounded_but_keep_the_verdict() {
        let mut r = sample_report();
        for i in 0..40 {
            r.notes.push(format!("note {i} {}", "y".repeat(500)));
        }
        let t = render_browser_report(&r);
        assert!(t.len() <= MAX_REPORT_CHARS + 40, "{}", t.len());
        assert!(t.contains("(report truncated)") && t.trim_end().contains("RESULT: "));
    }

    #[test]
    fn page_helper_embeds_the_spec_as_json_not_code() {
        let evil = json!({ "selector": "x'); alert(1); ('" });
        let js = page_fn(JS_EXISTS, &evil);
        assert!(js.contains(r#""selector":"x'); alert(1); ('""#));
        assert!(js.starts_with("(function(spec){") && js.ends_with("})({\"selector\":\"x'); alert(1); ('\"})"));
    }

    #[test]
    fn missing_browser_is_reported_honestly() {
        assert!(NO_BROWSER_MESSAGE.contains("NOT run") && NO_BROWSER_MESSAGE.contains("Do not claim"));
    }
}
