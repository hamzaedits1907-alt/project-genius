//! Supervised child processes.
//!
//! * Only allowlisted [`Operation`]s can be started; arguments are fixed by Genius and
//!   passed directly (never through a shell), so model output can never become a command.
//! * Output is streamed line-by-line to the UI, redacted, ANSI-stripped and bounded.
//! * Each process runs in its own process group (Unix) / is killed with its tree
//!   (Windows, `taskkill /T`), so stopping a dev server also stops what it spawned.
//! * Every run is persisted with its exact command, cwd, exit code and output tail.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::db::{now, Db, RunRow};
use crate::error::{GeniusError, Result};
use crate::events::{channels, EventBus, EventKind, EventStatus};
use crate::redact::redact;

const TAIL_LINES: usize = 1500;
const MAX_LINE: usize = 4000;

/// Environment variables never passed to project processes.
const SCRUBBED_ENV: &[&str] = &[
    "NVIDIA_API_KEY", "NGC_API_KEY", "GITHUB_TOKEN", "GH_TOKEN", "GH_ENTERPRISE_TOKEN", "GITHUB_ENTERPRISE_TOKEN",
    "OPENAI_API_KEY", "ANTHROPIC_API_KEY", "NPM_TOKEN", "NODE_AUTH_TOKEN", "AWS_SECRET_ACCESS_KEY", "AWS_SESSION_TOKEN",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Install,
    /// `npm install --legacy-peer-deps`: retried automatically only when a normal install
    /// failed with a peer-dependency conflict (ERESOLVE). Same trust level as Install.
    InstallLegacyPeers,
    Dev,
    Build,
    Test,
    Typecheck,
}

impl Operation {
    pub fn script(&self) -> Option<&'static str> {
        match self {
            Operation::Install | Operation::InstallLegacyPeers => None,
            Operation::Dev => Some("dev"),
            Operation::Build => Some("build"),
            Operation::Test => Some("test"),
            Operation::Typecheck => Some("typecheck"),
        }
    }

    pub fn event_kind(&self) -> EventKind {
        match self {
            Operation::Install | Operation::InstallLegacyPeers => EventKind::Install,
            Operation::Dev => EventKind::Preview,
            Operation::Build | Operation::Typecheck => EventKind::Build,
            Operation::Test => EventKind::Test,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Operation::Install => "Install dependencies",
            Operation::InstallLegacyPeers => "Install dependencies (compatibility mode)",
            Operation::Dev => "Development server",
            Operation::Build => "Production build",
            Operation::Test => "Tests",
            Operation::Typecheck => "Type check",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Install | Operation::InstallLegacyPeers => "install",
            Operation::Dev => "dev",
            Operation::Build => "build",
            Operation::Test => "test",
            Operation::Typecheck => "typecheck",
        }
    }

    /// Fixed argument vector for npm. Dev servers are bound to loopback on a Genius-chosen port.
    pub fn npm_args(&self, port: Option<u16>, dev_config: Option<&str>) -> Vec<String> {
        match self {
            Operation::Install => vec!["install".into(), "--no-audit".into(), "--no-fund".into()],
            Operation::InstallLegacyPeers => vec!["install".into(), "--no-audit".into(), "--no-fund".into(), "--legacy-peer-deps".into()],
            Operation::Dev => {
                let mut a: Vec<String> = vec!["run".into(), "dev".into(), "--".into(), "--host".into(), "127.0.0.1".into(), "--strictPort".into()];
                if let Some(p) = port {
                    a.push("--port".into());
                    a.push(p.to_string());
                }
                if let Some(c) = dev_config {
                    a.push("--config".into());
                    a.push(c.into());
                }
                a
            }
            other => vec!["run".into(), other.script().unwrap_or_default().into()],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInfo {
    pub id: String,
    pub project_id: String,
    pub op: Operation,
    pub command: String,
    pub cwd: String,
    pub reason: String,
    pub started_at: String,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub id: String,
    pub op: Operation,
    /// "succeeded" | "failed" | "cancelled" | "spawn_failed"
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub output_tail: String,
}

impl RunOutcome {
    pub fn ok(&self) -> bool {
        self.status == "succeeded"
    }
}

struct RunState {
    info: RunInfo,
    pid: Option<u32>,
    cancel: CancellationToken,
    done: watch::Receiver<Option<RunOutcome>>,
}

/// How npm is started. On Windows npm ships as `npm.cmd`, a batch file that only runs through
/// `cmd.exe`. Genius prefers to bypass the batch file and run `node.exe <npm-cli.js>` directly
/// (no `cmd.exe` involved, so no batch-file argument parsing and a clean process tree); it falls
/// back to `npm.cmd`, which Rust's standard library launches through `cmd.exe` with escaped
/// arguments, after Genius has validated every argument against a strict character set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmLauncher {
    pub program: PathBuf,
    /// Arguments placed before the npm arguments (the npm-cli.js path for the node launcher).
    pub prefix: Vec<String>,
    /// "npm" | "npm.cmd" | "node npm-cli.js"
    pub kind: &'static str,
}

/// Resolve npm from `PATH` directories. Pure apart from file-existence checks, so the Windows
/// branch is unit-tested on every platform.
pub fn resolve_npm(dirs: &[PathBuf], windows: bool) -> Option<NpmLauncher> {
    if !windows {
        let p = dirs.iter().map(|d| d.join("npm")).find(|c| is_executable_file(c))?;
        return Some(NpmLauncher { program: p, prefix: vec![], kind: "npm" });
    }
    for d in dirs {
        let node = d.join("node.exe");
        let cli = d.join("node_modules").join("npm").join("bin").join("npm-cli.js");
        if node.is_file() && cli.is_file() {
            return Some(NpmLauncher { program: node, prefix: vec![cli.to_string_lossy().to_string()], kind: "node npm-cli.js" });
        }
    }
    // Only real launchers: never `npm` (a POSIX shell script) or `npm.ps1`.
    for d in dirs {
        for name in ["npm.cmd", "npm.exe"] {
            let c = d.join(name);
            if c.is_file() {
                return Some(NpmLauncher { program: c, prefix: vec![], kind: if name == "npm.cmd" { "npm.cmd" } else { "npm" } });
            }
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false);
    }
    #[allow(unreachable_code)]
    true
}

/// Every npm argument Genius passes must match this character set. It excludes whitespace,
/// quotes and every character with meaning to `cmd.exe` (`&|<>^%!"()` …), so even the
/// `npm.cmd` fallback can never turn an argument into a command.
pub fn validate_npm_arg(a: &str) -> Result<()> {
    let ok = !a.is_empty() && a.len() <= 200 && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:=@".contains(c));
    if ok {
        Ok(())
    } else {
        Err(GeniusError::Validation(format!("refusing to pass an unexpected argument to npm: {a:?}")))
    }
}

#[derive(Clone)]
pub struct ProcessManager {
    db: Arc<Db>,
    bus: EventBus,
    runs: Arc<Mutex<HashMap<String, RunState>>>,
    npm: Arc<Mutex<Option<NpmLauncher>>>,
    /// Every cleaned output line as (run id, line), for in-process watchers (preview).
    lines: tokio::sync::broadcast::Sender<(String, String)>,
}

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07]*\x07").unwrap());

pub fn clean_line(s: &str) -> String {
    let s = ANSI.replace_all(s, "");
    let s = redact(&s);
    if s.len() > MAX_LINE {
        let mut cut = MAX_LINE;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}… [line truncated]", &s[..cut])
    } else {
        s
    }
}

impl ProcessManager {
    pub fn new(db: Arc<Db>, bus: EventBus) -> Self {
        Self { db, bus, runs: Arc::new(Mutex::new(HashMap::new())), npm: Arc::new(Mutex::new(None)), lines: tokio::sync::broadcast::channel(2048).0 }
    }

    pub fn subscribe_lines(&self) -> tokio::sync::broadcast::Receiver<(String, String)> {
        self.lines.subscribe()
    }

    /// Final outcome of a run that has exited (None while it is running or unknown).
    pub fn outcome(&self, run_id: &str) -> Option<RunOutcome> {
        self.runs.lock().unwrap().get(run_id).and_then(|r| r.done.borrow().clone())
    }

    pub fn is_running(&self, run_id: &str) -> bool {
        self.runs.lock().unwrap().get(run_id).is_some_and(|r| r.done.borrow().is_none())
    }

    pub fn npm_launcher(&self) -> Result<NpmLauncher> {
        self.npm()
    }

    fn npm(&self) -> Result<NpmLauncher> {
        if let Some(p) = self.npm.lock().unwrap().clone() {
            return Ok(p);
        }
        let dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        let p = resolve_npm(&dirs, cfg!(windows)).ok_or_else(|| {
            GeniusError::MissingTool(
                "npm was not found on PATH. Install Node.js 20 or newer from https://nodejs.org and restart Genius.".into(),
            )
        })?;
        *self.npm.lock().unwrap() = Some(p.clone());
        Ok(p)
    }

    pub fn active(&self, project_id: Option<&str>) -> Vec<RunInfo> {
        self.runs
            .lock()
            .unwrap()
            .values()
            .filter(|r| r.done.borrow().is_none())
            .filter(|r| project_id.is_none_or(|p| r.info.project_id == p))
            .map(|r| r.info.clone())
            .collect()
    }

    pub fn dev_server(&self, project_id: &str) -> Option<RunInfo> {
        self.active(Some(project_id)).into_iter().find(|r| r.op == Operation::Dev)
    }

    /// Start an allowlisted npm operation.
    pub async fn start(&self, project_id: &str, task_id: Option<&str>, cwd: &Path, op: Operation, reason: &str) -> Result<RunInfo> {
        self.start_with(project_id, task_id, cwd, op, reason, None).await
    }

    /// Like [`start`], optionally pointing the dev server at a Genius-generated config wrapper
    /// (a fixed, validated relative path inside node_modules/.genius).
    pub async fn start_with(&self, project_id: &str, task_id: Option<&str>, cwd: &Path, op: Operation, reason: &str, dev_config: Option<&str>) -> Result<RunInfo> {
        self.start_env(project_id, task_id, cwd, op, reason, dev_config, &[]).await
    }

    /// Start an allowlisted npm operation with extra (Genius-defined) environment variables.
    #[allow(clippy::too_many_arguments)]
    pub async fn start_env(&self, project_id: &str, task_id: Option<&str>, cwd: &Path, op: Operation, reason: &str, dev_config: Option<&str>, env: &[(&str, String)]) -> Result<RunInfo> {
        if dev_config.is_some_and(|c| c != "node_modules/.genius/preview.config.mjs") {
            return Err(GeniusError::Validation("unexpected dev server config".into()));
        }
        if env.iter().any(|(k, _)| !k.starts_with("GENIUS_")) {
            return Err(GeniusError::Validation("only GENIUS_* variables may be added to a project process".into()));
        }
        let npm = self.npm()?;
        if !cwd.join("package.json").is_file() {
            return Err(GeniusError::Validation("This project has no package.json, so npm commands cannot run.".into()));
        }
        if op == Operation::Dev {
            if let Some(existing) = self.dev_server(project_id) {
                self.stop(&existing.id).await?;
            }
        }
        let port = if op == Operation::Dev { Some(free_port()?) } else { None };
        let npm_args = op.npm_args(port, if op == Operation::Dev { dev_config } else { None });
        for a in &npm_args {
            validate_npm_arg(a)?;
        }
        let mut args = npm.prefix.clone();
        args.extend(npm_args.iter().cloned());
        let shown = format!("npm {}", npm_args.join(" "));
        self.spawn_cmd(project_id, task_id, cwd, op, reason, &npm.program, &args, port, &shown, env).await
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    async fn spawn(
        &self,
        project_id: &str,
        task_id: Option<&str>,
        cwd: &Path,
        op: Operation,
        reason: &str,
        program: &Path,
        args: &[String],
        port: Option<u16>,
    ) -> Result<RunInfo> {
        let prog_name = program.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "npm".into());
        let shown = format!("{} {}", prog_name, args.join(" "));
        self.spawn_cmd(project_id, task_id, cwd, op, reason, program, args, port, &shown, &[]).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn spawn_cmd(
        &self,
        project_id: &str,
        task_id: Option<&str>,
        cwd: &Path,
        op: Operation,
        reason: &str,
        program: &Path,
        args: &[String],
        port: Option<u16>,
        shown: &str,
        env: &[(&str, String)],
    ) -> Result<RunInfo> {
        let id = uuid::Uuid::new_v4().to_string();
        let command = shown.to_string();
        let cwd = &crate::workspace::display_path(cwd);
        let info = RunInfo {
            id: id.clone(),
            project_id: project_id.into(),
            op,
            command: command.clone(),
            cwd: cwd.to_string_lossy().into(),
            reason: reason.into(),
            started_at: now(),
            port,
        };
        self.db.insert_run(&RunRow {
            id: id.clone(),
            project_id: project_id.into(),
            op: op.as_str().into(),
            command: command.clone(),
            cwd: info.cwd.clone(),
            reason: reason.into(),
            status: "running".into(),
            exit_code: None,
            output_tail: String::new(),
            started_at: info.started_at.clone(),
            ended_at: None,
        })?;

        let mut ev = self.bus.start(
            Some(project_id),
            task_id,
            op.event_kind(),
            EventStatus::Running,
            format!("{}: {}", op.label(), command),
            json!({ "runId": id, "command": command, "cwd": info.cwd, "reason": reason, "op": op.as_str() }),
        );

        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("NO_COLOR", "1")
            .env("FORCE_COLOR", "0")
            .env("BROWSER", "none")
            .env("npm_config_update_notifier", "false")
            .kill_on_drop(true);
        for k in SCRUBBED_ENV {
            cmd.env_remove(k);
        }
        for (k, v) in env {
            cmd.env(k, v);
        }
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let started = Instant::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("could not start {command}: {e}");
                self.db.finish_run(&id, "spawn_failed", None, &msg)?;
                ev.update(EventStatus::Failed, json!({ "error": { "kind": "process", "message": msg } }));
                return Err(GeniusError::Process(msg));
            }
        };
        let pid = child.id();
        ev.update(EventStatus::Running, json!({ "pid": pid }));
        self.bus.push(channels::PROCESS_STATE, json!({ "run": info, "state": "running" }));

        let tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut readers = Vec::new();
        for (stream_name, reader) in [
            ("stdout", child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)),
            ("stderr", child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>)),
        ] {
            let Some(reader) = reader else { continue };
            let bus = self.bus.clone();
            let tail = tail.clone();
            let run_id = id.clone();
            let line_tx = self.lines.clone();
            readers.push(tokio::spawn(async move {
                let mut lines = BufReader::new(reader).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let line = clean_line(&line);
                    {
                        let mut t = tail.lock().unwrap();
                        t.push_back(line.clone());
                        if t.len() > TAIL_LINES {
                            t.pop_front();
                        }
                    }
                    let _ = line_tx.send((run_id.clone(), line.clone()));
                    bus.push(channels::PROCESS_OUTPUT, json!({ "runId": run_id, "stream": stream_name, "line": line }));
                }
            }));
        }

        let cancel = CancellationToken::new();
        let (tx, rx) = watch::channel(None);
        self.runs.lock().unwrap().insert(id.clone(), RunState { info: info.clone(), pid, cancel: cancel.clone(), done: rx });

        let db = self.db.clone();
        let bus = self.bus.clone();
        let info2 = info.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                s = child.wait() => s.ok(),
                _ = cancel.cancelled() => {
                    if let Some(pid) = pid { kill_tree(pid).await; }
                    let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                    let _ = child.start_kill();
                    None
                }
            };
            for r in readers {
                let _ = tokio::time::timeout(Duration::from_secs(2), r).await;
            }
            let exit_code = status.and_then(|s| s.code());
            let cancelled = cancel.is_cancelled();
            let st = if cancelled {
                "cancelled"
            } else if status.is_some_and(|s| s.success()) {
                "succeeded"
            } else {
                "failed"
            };
            let output_tail = tail.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n");
            let outcome = RunOutcome {
                id: info2.id.clone(),
                op: info2.op,
                status: st.into(),
                exit_code,
                duration_ms: started.elapsed().as_millis() as u64,
                output_tail: output_tail.clone(),
            };
            let _ = db.finish_run(&info2.id, st, exit_code, &output_tail);
            let ev_status = match st {
                "succeeded" => EventStatus::Completed,
                "cancelled" => EventStatus::Cancelled,
                _ => EventStatus::Failed,
            };
            ev.update(ev_status, json!({ "exitCode": exit_code, "durationMs": outcome.duration_ms, "status": st }));
            bus.push(channels::PROCESS_STATE, json!({ "run": info2, "state": st, "outcome": outcome }));
            let _ = tx.send(Some(outcome));
        });

        Ok(info)
    }

    pub async fn wait(&self, run_id: &str) -> Result<RunOutcome> {
        let mut rx = {
            let runs = self.runs.lock().unwrap();
            runs.get(run_id).map(|r| r.done.clone()).ok_or_else(|| GeniusError::NotFound(format!("run {run_id}")))?
        };
        loop {
            if let Some(o) = rx.borrow().clone() {
                return Ok(o);
            }
            if rx.changed().await.is_err() {
                return Err(GeniusError::Internal("process supervisor dropped".into()));
            }
        }
    }

    /// Start and wait. Honours `cancel` by stopping the process.
    pub async fn run_to_end(
        &self,
        project_id: &str,
        task_id: Option<&str>,
        cwd: &Path,
        op: Operation,
        reason: &str,
        cancel: &CancellationToken,
    ) -> Result<RunOutcome> {
        let info = self.start(project_id, task_id, cwd, op, reason).await?;
        tokio::select! {
            o = self.wait(&info.id) => o,
            _ = cancel.cancelled() => {
                self.stop(&info.id).await?;
                self.wait(&info.id).await
            }
        }
    }

    pub async fn stop(&self, run_id: &str) -> Result<()> {
        let token = {
            let runs = self.runs.lock().unwrap();
            runs.get(run_id).map(|r| r.cancel.clone())
        };
        match token {
            Some(t) => {
                t.cancel();
                let _ = tokio::time::timeout(Duration::from_secs(10), self.wait(run_id)).await;
                Ok(())
            }
            None => Err(GeniusError::NotFound(format!("run {run_id}"))),
        }
    }

    pub async fn stop_project(&self, project_id: &str) {
        for r in self.active(Some(project_id)) {
            let _ = self.stop(&r.id).await;
        }
    }

    pub async fn stop_all(&self) {
        for r in self.active(None) {
            let _ = self.stop(&r.id).await;
        }
    }

    /// Synchronous best-effort kill used during application exit.
    pub fn kill_all_now(&self) {
        let pids: Vec<u32> = self.runs.lock().unwrap().values().filter(|r| r.done.borrow().is_none()).filter_map(|r| r.pid).collect();
        for pid in pids {
            kill_tree_sync(pid);
        }
    }

    pub fn pid_of(&self, run_id: &str) -> Option<u32> {
        self.runs.lock().unwrap().get(run_id).and_then(|r| r.pid)
    }

    /// Wait until the dev server accepts TCP connections, or fails/exits.
    pub async fn wait_until_listening(&self, run: &RunInfo, timeout: Duration) -> Result<String> {
        let port = run.port.ok_or_else(|| GeniusError::Internal("dev server has no port".into()))?;
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(outcome) = self.runs.lock().unwrap().get(&run.id).and_then(|r| r.done.borrow().clone()) {
                return Err(GeniusError::Process(format!(
                    "the development server exited before it started listening (status: {}, exit code: {})",
                    outcome.status,
                    outcome.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())
                )));
            }
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                return Ok(format!("http://127.0.0.1:{port}/"));
            }
            if Instant::now() > deadline {
                return Err(GeniusError::Process(format!("the development server did not start listening on port {port} within {}s", timeout.as_secs())));
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

pub fn free_port() -> Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

#[cfg(unix)]
async fn kill_tree(pid: u32) {
    let pgid = pid as i32;
    unsafe {
        libc::kill(-pgid, libc::SIGTERM);
    }
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if unsafe { libc::kill(-pgid, 0) } != 0 {
            return;
        }
    }
    unsafe {
        libc::kill(-pgid, libc::SIGKILL);
    }
}

#[cfg(unix)]
fn kill_tree_sync(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

#[cfg(windows)]
async fn kill_tree(pid: u32) {
    kill_tree_sync(pid);
}

#[cfg(windows)]
fn kill_tree_sync(pid: u32) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Locate an executable on PATH (honouring PATHEXT on Windows). No shell involved.
pub fn find_executable(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .map(|e| e.to_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for ext in &exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if cand.metadata().map(|m| m.permissions().mode() & 0o111 == 0).unwrap_or(true) {
                        continue;
                    }
                }
                return Some(cand);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub hint: String,
}

/// Run a fixed, non-interactive command and capture its output (used for git/gh/tool probing).
pub async fn capture(program: &Path, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Result<(i32, String, String)> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .kill_on_drop(true);
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| GeniusError::Process(format!("{} timed out after {}s", program.display(), timeout.as_secs())))?
        .map_err(|e| GeniusError::Process(format!("could not run {}: {e}", program.display())))?;
    Ok((
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub async fn detect_tools() -> Vec<ToolStatus> {
    let specs: [(&str, &[&str], &str); 4] = [
        ("node", &["--version"], "Install Node.js 20+ from https://nodejs.org — required to install, run and build web projects."),
        ("npm", &["--version"], "npm ships with Node.js. Reinstall Node.js if it is missing."),
        ("git", &["--version"], "Install Git from https://git-scm.com — required for source control features."),
        ("gh", &["--version"], "Install GitHub CLI from https://cli.github.com — used to sign in to GitHub and create repositories."),
    ];
    let mut out = Vec::new();
    for (name, args, hint) in specs {
        let path = find_executable(name);
        let version = match &path {
            Some(p) => capture(p, args, None, Duration::from_secs(10))
                .await
                .ok()
                .filter(|(c, _, _)| *c == 0)
                .map(|(_, o, _)| o.lines().next().unwrap_or("").trim().to_string()),
            None => None,
        };
        out.push(ToolStatus {
            name: name.into(),
            found: version.is_some(),
            path: path.map(|p| p.to_string_lossy().into()),
            version,
            hint: hint.into(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingEmitter;

    #[test]
    fn args_are_fixed() {
        assert_eq!(Operation::Build.npm_args(None, None), vec!["run", "build"]);
        let dev = Operation::Dev.npm_args(Some(5000), None);
        assert!(dev.contains(&"127.0.0.1".to_string()) && dev.ends_with(&["--port".into(), "5000".into()]));
        let dev = Operation::Dev.npm_args(Some(5000), Some("node_modules/.genius/preview.config.mjs"));
        assert!(dev.ends_with(&["--config".into(), "node_modules/.genius/preview.config.mjs".into()]));
        assert_eq!(Operation::Install.npm_args(None, None)[0], "install");
    }

    #[test]
    fn clean_line_strips_ansi_and_secrets() {
        assert_eq!(clean_line("\x1b[32mok\x1b[0m token ghp_ABCDEFGHIJKLMNOPQRSTUVWX"), "ok token [REDACTED]");
    }

    fn manager() -> (ProcessManager, Arc<RecordingEmitter>, Arc<Db>) {
        let db = Arc::new(Db::open_in_memory().unwrap());
        db.insert_project(&crate::db::ProjectRow {
            id: "p".into(), name: "p".into(), description: String::new(), path: "/x".into(), kind: "k".into(), spec: None,
            notes: String::new(), status: json!({}), created_at: now(), updated_at: now(), last_opened_at: None,
        }).unwrap();
        let rec = Arc::new(RecordingEmitter::default());
        let bus = EventBus::new(db.clone(), rec.clone());
        (ProcessManager::new(db.clone(), bus), rec, db)
    }

    /// Uses a fake `npm` script placed first on PATH... instead we exercise `spawn` directly
    /// with a real program to verify streaming, exit codes and cancellation.
    #[cfg(unix)]
    #[tokio::test]
    async fn streams_output_and_reports_exit_code() {
        let (pm, rec, db) = manager();
        let dir = tempfile::tempdir().unwrap();
        let sh = PathBuf::from("/bin/sh");
        let args = vec!["-c".to_string(), "echo hello; echo oops 1>&2; exit 3".to_string()];
        let info = pm.spawn("p", None, dir.path(), Operation::Build, "test", &sh, &args, None).await.unwrap();
        let out = pm.wait(&info.id).await.unwrap();
        assert_eq!(out.status, "failed");
        assert_eq!(out.exit_code, Some(3));
        assert!(out.output_tail.contains("hello") && out.output_tail.contains("oops"));
        let lines: Vec<String> = rec.on(channels::PROCESS_OUTPUT).iter().map(|v| v["line"].as_str().unwrap().to_string()).collect();
        assert!(lines.contains(&"hello".to_string()));
        let runs = db.list_runs("p", 5).unwrap();
        assert_eq!(runs[0].exit_code, Some(3));
        assert_eq!(runs[0].status, "failed");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_process_tree() {
        let (pm, _rec, db) = manager();
        let dir = tempfile::tempdir().unwrap();
        let sh = PathBuf::from("/bin/sh");
        // parent spawns a child that would outlive a naive kill
        let args = vec!["-c".to_string(), "sleep 60 & echo started; wait".to_string()];
        let info = pm.spawn("p", None, dir.path(), Operation::Dev, "test", &sh, &args, None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let pid = pm.pid_of(&info.id).unwrap() as i32;
        assert_eq!(pm.active(Some("p")).len(), 1);
        pm.stop(&info.id).await.unwrap();
        let out = pm.wait(&info.id).await.unwrap();
        assert_eq!(out.status, "cancelled");
        assert!(pm.active(Some("p")).is_empty());
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_ne!(unsafe { libc::kill(-pid, 0) }, 0, "process group is gone");
        assert_eq!(db.list_runs("p", 1).unwrap()[0].status, "cancelled");
    }

    #[tokio::test]
    async fn missing_package_json_is_rejected() {
        let (pm, _rec, _db) = manager();
        let dir = tempfile::tempdir().unwrap();
        if find_executable("npm").is_some() {
            let err = pm.start("p", None, dir.path(), Operation::Build, "x").await.unwrap_err();
            assert!(matches!(err, GeniusError::Validation(_)));
        }
    }

    #[test]
    fn windows_npm_resolution_prefers_node_cli_and_never_the_shell_script() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("nodejs");
        std::fs::create_dir_all(bin.join("node_modules/npm/bin")).unwrap();
        // what a Node.js install on Windows contains
        for f in ["npm", "npm.cmd", "npm.ps1", "node.exe"] {
            std::fs::write(bin.join(f), "").unwrap();
        }
        let dirs = vec![d.path().join("empty"), bin.clone()];
        // without npm-cli.js: the batch file, not the POSIX `npm` script
        let l = resolve_npm(&dirs, true).unwrap();
        assert_eq!(l.kind, "npm.cmd");
        assert!(l.program.ends_with("npm.cmd"));
        // with npm-cli.js next to node.exe: run node directly, no cmd.exe involved
        std::fs::write(bin.join("node_modules/npm/bin/npm-cli.js"), "").unwrap();
        let l = resolve_npm(&dirs, true).unwrap();
        assert_eq!(l.kind, "node npm-cli.js");
        assert!(l.program.ends_with("node.exe"));
        assert!(l.prefix[0].ends_with("npm-cli.js"));
        assert!(resolve_npm(&[d.path().join("empty")], true).is_none());
    }

    #[test]
    fn npm_arguments_are_validated_against_cmd_metacharacters() {
        for op in [Operation::Install, Operation::InstallLegacyPeers, Operation::Build, Operation::Test, Operation::Typecheck] {
            for a in op.npm_args(None, None) {
                validate_npm_arg(&a).unwrap();
            }
        }
        for a in Operation::Dev.npm_args(Some(51234), Some("node_modules/.genius/preview.config.mjs")) {
            validate_npm_arg(&a).unwrap();
        }
        for bad in ["a b", "x&calc", "\"q\"", "%PATH%", "a|b", "^", "!x!", "", "(x)", "a>b"] {
            assert!(validate_npm_arg(bad).is_err(), "{bad} must be rejected");
        }
    }

    #[test]
    fn finds_executables() {
        #[cfg(unix)]
        assert!(find_executable("sh").is_some());
        assert!(find_executable("definitely-not-a-real-tool-xyz").is_none());
    }
}
