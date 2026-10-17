//! Project lifecycle: dependency installation, the live preview, verification, and the
//! bounded "Fix automatically" loop.
//!
//! Every state transition is derived from a real observation — files on disk, an npm exit
//! code, an HTTP answer carrying the run's identity token, a module that compiled or failed.
//! Nothing here is ever reported as ready, installed or fixed on the strength of a model's text.
//!
//! ```text
//! Idle → Checking → (Installing) → Starting → Compiling → Ready
//!                ↘ Failed (config | install | start | identity | compile)      Stopping → Idle
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

use crate::app::Genius;
use crate::changes::{ChangeStore, Changeset};
use crate::context::{self, ContextRequest};
use crate::db::now;
use crate::deps::{self, DepState, DepStatus};
use crate::diagnostics;
use crate::error::{GeniusError, Result};
use crate::events::{channels, EventBus, EventKind, EventStatus};
use crate::preview::{self, CheckFailure, PreviewProblem, PreviewState, PreviewStatus};
use crate::process::{Operation, ProcessManager, RunOutcome};
use crate::prompts;
use crate::settings::Policy;

const START_TIMEOUT: Duration = Duration::from_secs(150);
const COMPILE_BUDGET: Duration = Duration::from_secs(60);
pub const MAX_AUTOFIX_ATTEMPTS: usize = 3;

struct Session {
    state: PreviewState,
    cancel: CancellationToken,
    token: Option<String>,
}

/// Shared preview bookkeeping (cloneable into watcher tasks).
#[derive(Clone)]
pub struct PreviewHub {
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    install_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    bus: EventBus,
    procs: ProcessManager,
    client: reqwest::Client,
}

impl PreviewHub {
    pub fn new(bus: EventBus, procs: ProcessManager) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            locks: Arc::new(Mutex::new(HashMap::new())),
            install_locks: Arc::new(Mutex::new(HashMap::new())),
            bus,
            procs,
            client: preview::http_client(),
        }
    }

    fn lock_for(map: &Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>, pid: &str) -> Arc<tokio::sync::Mutex<()>> {
        map.lock().unwrap().entry(pid.to_string()).or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone()
    }

    pub fn state(&self, pid: &str) -> PreviewState {
        let s = self.sessions.lock().unwrap().get(pid).map(|s| s.state.clone());
        let mut st = s.unwrap_or_else(|| PreviewState { project_id: pid.into(), message: "The preview is not running.".into(), updated_at: now(), ..Default::default() });
        // A run that died without its watcher noticing yet is never reported as live.
        if let Some(rid) = &st.run_id {
            if matches!(st.status, PreviewStatus::Ready | PreviewStatus::Compiling) && !self.procs.is_running(rid) {
                st.status = PreviewStatus::Failed;
                st.message = "The development server is no longer running.".into();
            }
        }
        st
    }

    /// Apply `f` to the session if it still belongs to `generation`, then publish the result.
    fn update(&self, pid: &str, generation: Option<u64>, f: impl FnOnce(&mut PreviewState)) -> Option<PreviewState> {
        let st = {
            let mut map = self.sessions.lock().unwrap();
            let s = map.entry(pid.to_string()).or_insert_with(|| Session { state: PreviewState { project_id: pid.into(), ..Default::default() }, cancel: CancellationToken::new(), token: None });
            if generation.is_some_and(|g| g != s.state.generation) {
                return None;
            }
            f(&mut s.state);
            s.state.updated_at = now();
            s.state.clone()
        };
        self.bus.push(channels::PREVIEW, json!({ "projectId": pid, "state": st, "status": legacy_status(st.status), "url": st.url, "error": st.problem.as_ref().map(|p| p.summary.clone()) }));
        Some(st)
    }

    /// Begin a new generation: cancels whatever the previous one was doing.
    fn begin(&self, pid: &str) -> (u64, CancellationToken) {
        let mut map = self.sessions.lock().unwrap();
        let s = map.entry(pid.to_string()).or_insert_with(|| Session { state: PreviewState { project_id: pid.into(), ..Default::default() }, cancel: CancellationToken::new(), token: None });
        s.cancel.cancel();
        s.cancel = CancellationToken::new();
        s.state.generation += 1;
        s.token = None;
        (s.state.generation, s.cancel.clone())
    }

    fn set_token(&self, pid: &str, generation: u64, token: Option<String>) {
        if let Some(s) = self.sessions.lock().unwrap().get_mut(pid) {
            if s.state.generation == generation {
                s.token = token;
            }
        }
    }

    fn token(&self, pid: &str) -> Option<String> {
        self.sessions.lock().unwrap().get(pid).and_then(|s| s.token.clone())
    }

    /// Re-run the compile check against the running server of the current generation.
    pub async fn recheck(&self, pid: &str, root: &str) -> Option<PreviewState> {
        let st = self.state(pid);
        let (Some(port), Some(run_id)) = (st.port, st.run_id.clone()) else { return None };
        if !self.procs.is_running(&run_id) || st.status.busy() {
            return None;
        }
        let token = self.token(pid);
        let generation = st.generation;
        let mut result = preview::check_server(&self.client, port, token.as_deref(), Some(root), COMPILE_BUDGET).await;
        for _ in 0..20 {
            if !matches!(result, Err(CheckFailure::NotReady(_))) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            result = preview::check_server(&self.client, port, token.as_deref(), Some(root), COMPILE_BUDGET).await;
        }
        let r = result;
        self.update(pid, Some(generation), |s| apply_check(s, r, Some(run_id.clone())))
    }
}

fn legacy_status(s: PreviewStatus) -> &'static str {
    match s {
        PreviewStatus::Ready => "ready",
        PreviewStatus::Failed => "failed",
        PreviewStatus::Idle => "stopped",
        _ => "starting",
    }
}

fn apply_check(s: &mut PreviewState, r: std::result::Result<usize, CheckFailure>, run_id: Option<String>) {
    match r {
        Ok(n) => {
            s.status = PreviewStatus::Ready;
            s.message = format!("Your app is running ({n} modules compiled).");
            s.problem = None;
            s.modules_checked = n;
        }
        Err(CheckFailure::Compile { file, message, details }) => {
            s.status = PreviewStatus::Failed;
            s.message = "The development server is running, but your app has an error.".into();
            s.problem = Some(PreviewProblem {
                stage: "compile".into(),
                summary: match &file {
                    Some(f) => format!("Your app could not be compiled: {message} (in {f})"),
                    None => format!("Your app could not be compiled: {message}"),
                },
                details,
                file,
                run_id,
                fixable: true,
            });
        }
        Err(CheckFailure::WrongServer(m)) => {
            s.status = PreviewStatus::Failed;
            s.message = "Another program answered on the preview port.".into();
            s.problem = Some(PreviewProblem { stage: "identity".into(), summary: m.clone(), details: m, run_id, fixable: false, ..Default::default() });
        }
        Err(CheckFailure::NotReady(m)) => {
            s.status = PreviewStatus::Failed;
            s.message = "The development server stopped answering.".into();
            s.problem = Some(PreviewProblem { stage: "start".into(), summary: "The development server is not answering.".into(), details: m, run_id, fixable: true, ..Default::default() });
        }
    }
}

/// A real failure, with the evidence the repair step needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    /// "config" | "install" | "preview" | "build" | "runtime"
    pub kind: String,
    pub summary: String,
    pub command: String,
    pub exit_code: Option<i32>,
    pub output: String,
    pub run_id: Option<String>,
    pub file: Option<String>,
    /// false when no code change can fix it (offline, npm missing, disk full, …).
    pub fixable: bool,
}

/// Plain-language reading of an `npm install` failure: (summary, fixable by editing files).
pub fn classify_install(output: &str) -> (String, bool) {
    let o = output.to_lowercase();
    if o.contains("eresolve") {
        return ("Two packages require incompatible versions of a shared dependency.".into(), true);
    }
    if o.contains("etarget") || o.contains("no matching version found") {
        return ("A package version listed in package.json does not exist on npm.".into(), true);
    }
    if o.contains("e404") || o.contains("is not in this registry") || o.contains("404 not found") {
        return ("A package listed in package.json does not exist on npm.".into(), true);
    }
    if o.contains("enotfound")
        || o.contains("etimedout")
        || o.contains("eai_again")
        || o.contains("econnreset")
        || o.contains("econnrefused")
        || o.contains("network request")
        || o.contains("self signed certificate")
        || o.contains("unable to get local issuer")
    {
        return ("npm could not reach the package registry. Check your internet connection (or company proxy) and try again.".into(), false);
    }
    if o.contains("eperm") || o.contains("ebusy") || o.contains("eacces") {
        return ("A file inside node_modules is locked by another program (an editor, antivirus, or a running server). Close it and try again.".into(), false);
    }
    if o.contains("enospc") {
        return ("The disk is full, so npm could not write the packages.".into(), false);
    }
    if o.contains("could not read package.json") || (o.contains("enoent") && o.contains("package.json")) {
        return ("npm could not find package.json in the project folder.".into(), false);
    }
    if o.contains("ejsonparse") || o.contains("failed to parse json") {
        return ("package.json is not valid JSON.".into(), true);
    }
    ("Installing the dependencies failed.".into(), true)
}

fn port_unavailable(output: &str) -> bool {
    let o = output.to_lowercase();
    o.contains("already in use") || o.contains("eaddrinuse") || (o.contains("eacces") && o.contains("listen"))
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

fn build_summary(output: &str) -> String {
    let diags = diagnostics::parse(output);
    match diags.first() {
        Some(d) => match (&d.file, d.line) {
            (Some(f), Some(l)) => format!("{} error in {f} line {l}: {}", if d.source == "typescript" { "TypeScript" } else { "Build" }, d.message),
            (Some(f), None) => format!("Build error in {f}: {}", d.message),
            _ => format!("Build error: {}", d.message),
        },
        None => "The production build failed.".into(),
    }
}

impl Genius {
    // ================================================================= dependencies

    /// Packages that may be installed without asking: the template's own packages plus every
    /// package that was part of an install the user approved before.
    fn trusted_packages(&self, project_id: &str) -> Vec<String> {
        let mut t: Vec<String> = crate::scaffold::template_packages();
        if let Ok(p) = self.db.get_project(project_id) {
            if let Some(a) = p.status.get("approvedPackages").and_then(|v| v.as_array()) {
                t.extend(a.iter().filter_map(|x| x.as_str().map(String::from)));
            }
        }
        t
    }

    /// Dependency state computed from disk right now.
    pub fn dep_status(&self, project_id: &str) -> Result<DepStatus> {
        let ws = self.workspace(project_id)?;
        let trusted = self.trusted_packages(project_id);
        let refs: Vec<&str> = trusted.iter().map(String::as_str).collect();
        Ok(deps::status(ws.root(), &refs))
    }

    pub fn needs_install(&self, project_id: &str) -> Result<bool> {
        Ok(self.dep_status(project_id)?.needs_install())
    }

    /// Install dependencies if — and only if — the files say it is needed. Serialized per
    /// project, so a preview start and a verification never run two installs at once. Packages
    /// that were never approved require approval (per the dependency-install policy).
    /// Returns the install outcome, or `None` when nothing had to be installed.
    pub async fn ensure_installed(&self, project_id: &str, task_id: Option<&str>, cancel: &CancellationToken) -> Result<Option<RunOutcome>> {
        let lock = PreviewHub::lock_for(&self.hub.install_locks, project_id);
        let _guard = lock.lock().await;
        let st = self.dep_status(project_id)?;
        match st.state {
            DepState::Ready | DepState::NoPackageJson => return Ok(None),
            DepState::InvalidPackageJson => return Err(GeniusError::Validation(st.reason)),
            DepState::NeedsInstall => {}
        }
        let ws = self.workspace(project_id)?;
        if !st.new_packages.is_empty() {
            let policy = self.settings()?.approvals.dependency_install;
            self.approvals
                .request(
                    &self.bus,
                    Some(project_id),
                    task_id,
                    "dependency_install",
                    policy,
                    &format!("Install {} new package(s): {}", st.new_packages.len(), st.new_packages.iter().take(8).cloned().collect::<Vec<_>>().join(", ")),
                    json!({ "command": "npm install --no-audit --no-fund", "cwd": ws.root(), "newPackages": st.new_packages, "allPackages": st.packages, "note": "npm downloads these packages from the npm registry and runs their install scripts on your computer." }),
                    cancel,
                )
                .await?;
        }
        // On Windows a running dev server keeps files in node_modules open (esbuild), which makes
        // npm fail with EPERM/EBUSY. Stop it first; the caller restarts the preview afterwards.
        if let Some(dev) = self.procs.dev_server(project_id) {
            self.hub.update(project_id, None, |s| {
                s.status = PreviewStatus::Installing;
                s.message = "Installing new dependencies; the preview restarts afterwards…".into();
            });
            self.bus.record(Some(project_id), task_id, EventKind::Install, EventStatus::Completed, "Stopped the preview server while dependencies are installed", json!({ "runId": dev.id }));
            self.procs.stop(&dev.id).await?;
        }
        let mut out = self.run_npm(project_id, task_id, Operation::Install, &st.reason, cancel).await?;
        if !out.ok() && out.output_tail.contains("ERESOLVE") {
            self.bus.record(
                Some(project_id),
                task_id,
                EventKind::Install,
                EventStatus::Running,
                "Peer-dependency conflict: retrying the install in compatibility mode (--legacy-peer-deps)",
                json!({ "runId": out.id }),
            );
            out = self.run_npm(project_id, task_id, Operation::InstallLegacyPeers, "Retry after a peer-dependency conflict", cancel).await?;
        }
        if out.ok() {
            self.after_install(project_id)?;
        }
        Ok(Some(out))
    }

    /// Record a successful install: fingerprint on disk + packages approved for this project.
    pub(crate) fn after_install(&self, project_id: &str) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let st = deps::record_install(ws.root())?;
        let mut approved = self.trusted_packages(project_id);
        approved.extend(st.packages);
        approved.sort();
        approved.dedup();
        self.db.patch_project_status(project_id, &json!({ "approvedPackages": approved }))?;
        Ok(())
    }

    /// Run an allowlisted npm operation to completion and record it in the project status.
    pub(crate) async fn run_npm(&self, project_id: &str, task_id: Option<&str>, op: Operation, reason: &str, cancel: &CancellationToken) -> Result<RunOutcome> {
        let ws = self.workspace(project_id)?;
        let out = self.procs.run_to_end(project_id, task_id, ws.root(), op, reason, cancel).await?;
        let key = match op {
            Operation::Install | Operation::InstallLegacyPeers => "lastInstall",
            Operation::Build => "lastBuild",
            Operation::Test => "lastTest",
            Operation::Typecheck => "lastTypecheck",
            Operation::Dev => "lastDev",
        };
        self.db.patch_project_status(project_id, &json!({ key: { "status": out.status, "exitCode": out.exit_code, "at": now(), "runId": out.id, "durationMs": out.duration_ms } }))?;
        if out.status == "cancelled" {
            return Err(GeniusError::Cancelled);
        }
        Ok(out)
    }

    // ================================================================= preview

    pub fn preview_state(&self, project_id: &str) -> PreviewState {
        self.hub.state(project_id)
    }

    /// URL of a preview that is verified ready (compat for older callers).
    pub fn preview_url(&self, project_id: &str) -> Option<String> {
        let s = self.hub.state(project_id);
        (s.status == PreviewStatus::Ready).then_some(s.url).flatten()
    }

    /// Start the preview in the background and return immediately. If a preview of this
    /// project is already starting or running, its current state is returned instead of
    /// starting a second server.
    pub fn start_preview(self: &Arc<Self>, project_id: &str) -> Result<PreviewState> {
        self.workspace(project_id)?;
        let cur = self.hub.state(project_id);
        let alive = cur.run_id.as_deref().is_some_and(|r| self.procs.is_running(r));
        if cur.status.busy() || (cur.status == PreviewStatus::Ready && alive) {
            return Ok(cur);
        }
        self.spawn_preview(project_id)
    }

    /// Stop any running server of this project and start a fresh one.
    pub fn restart_preview(self: &Arc<Self>, project_id: &str) -> Result<PreviewState> {
        self.workspace(project_id)?;
        self.spawn_preview(project_id)
    }

    fn spawn_preview(self: &Arc<Self>, project_id: &str) -> Result<PreviewState> {
        let me = self.clone();
        let pid = project_id.to_string();
        let st = self.hub.update(project_id, None, |s| {
            s.status = PreviewStatus::Checking;
            s.message = "Checking your project…".into();
            s.problem = None;
        });
        self.rt.spawn(async move {
            if let Err(e) = me.ensure_preview(&pid, None).await {
                log::warn!("preview of {pid} ended: {e}");
            }
        });
        Ok(st.unwrap_or_else(|| self.hub.state(project_id)))
    }

    /// The full preview lifecycle; resolves when the preview is Ready or Failed (or was stopped).
    pub async fn ensure_preview(&self, project_id: &str, task_id: Option<&str>) -> Result<PreviewState> {
        let lock = PreviewHub::lock_for(&self.hub.locks, project_id);
        let _guard = lock.lock().await;
        let (generation, cancel) = self.hub.begin(project_id);
        let g = Some(generation);
        let hub = self.hub.clone();
        let fail = |stage: &str, message: &str, summary: String, details: String, run_id: Option<String>, fixable: bool| {
            hub.update(project_id, g, |s| {
                s.status = PreviewStatus::Failed;
                s.message = message.into();
                s.problem = Some(PreviewProblem { stage: stage.into(), summary, details, run_id, fixable, file: None });
            })
            .unwrap_or_else(|| hub.state(project_id))
        };
        // Stop whatever server this project had (restart semantics, never two servers).
        if let Some(dev) = self.procs.dev_server(project_id) {
            self.hub.update(project_id, g, |s| {
                s.status = PreviewStatus::Stopping;
                s.message = "Stopping the previous server…".into();
            });
            self.procs.stop(&dev.id).await?;
        }
        self.hub.update(project_id, g, |s| {
            s.status = PreviewStatus::Checking;
            s.message = "Checking your project…".into();
            s.problem = None;
            s.url = None;
            s.port = None;
            s.run_id = None;
            s.modules_checked = 0;
        });

        // 1. Project folder and configuration.
        let ws = match self.workspace(project_id) {
            Ok(w) => w,
            Err(e) => return Ok(fail("config", "The project folder could not be opened.", e.to_string(), e.to_string(), None, false)),
        };
        let root = ws.root().to_string_lossy().to_string();
        let dep = self.dep_status(project_id)?;
        match dep.state {
            DepState::NoPackageJson => {
                return Ok(fail("config", "This project has no package.json yet.", "There is nothing to run yet: ask Genius to create the app first.".into(), dep.reason, None, false));
            }
            DepState::InvalidPackageJson => return Ok(fail("config", "package.json has an error.", dep.reason.clone(), dep.reason, None, true)),
            _ => {}
        }
        let has_dev = ws.read_text_opt("package.json")?.and_then(|t| deps::parse_strict(&t).ok()).is_some_and(|v| v.pointer("/scripts/dev").and_then(|d| d.as_str()).is_some());
        if !has_dev {
            return Ok(fail(
                "config",
                "package.json has no \"dev\" script.",
                "The project cannot be started because package.json has no \"dev\" script.".into(),
                "scripts.dev is missing".into(),
                None,
                true,
            ));
        }

        // 2. Dependencies, installed automatically when the files say so.
        if dep.needs_install() {
            self.hub.update(project_id, g, |s| {
                s.status = PreviewStatus::Installing;
                s.message = format!("Installing dependencies… ({})", dep.reason);
            });
            match self.ensure_installed(project_id, task_id, &cancel).await {
                Ok(Some(out)) if !out.ok() => {
                    let (summary, fixable) = classify_install(&out.output_tail);
                    return Ok(fail("install", "Installing the dependencies failed.", summary, tail(&out.output_tail, 6000), Some(out.id), fixable));
                }
                Ok(_) => {}
                Err(GeniusError::Cancelled) if cancel.is_cancelled() => return Ok(self.hub.state(project_id)),
                Err(GeniusError::Cancelled) => {
                    return Ok(fail("install", "Installation was not approved.", "The dependency installation was declined, so the preview cannot start.".into(), String::new(), None, false))
                }
                Err(e) => return Ok(fail("install", "Installing the dependencies failed.", e.to_string(), e.to_string(), None, !matches!(e, GeniusError::MissingTool(_)))),
            }
        }
        if cancel.is_cancelled() {
            return Ok(self.hub.state(project_id));
        }

        // 3. Script trust (a modified `dev` script needs approval), inspector wrapper.
        if let Err(e) = self.gate_script(project_id, task_id, Operation::Dev, &cancel).await {
            return Ok(fail("config", "The preview was not started.", e.to_string(), e.to_string(), None, false));
        }
        let inspector = crate::inspector::prepare(ws.root()).unwrap_or_else(|e| {
            log::warn!("preview inspector unavailable: {e}");
            None
        });

        // 4. Start the server; retry on a port collision.
        for attempt in 0..3 {
            let token = inspector.as_ref().map(|_| preview::new_token());
            self.hub.set_token(project_id, generation, token.clone());
            self.hub.update(project_id, g, |s| {
                s.status = PreviewStatus::Starting;
                s.message = "Starting the development server…".into();
                s.inspector = inspector.is_some();
            });
            let env: Vec<(&str, String)> = token.iter().map(|t| (preview::TOKEN_ENV, t.clone())).collect();
            let run = match self.procs.start_env(project_id, task_id, ws.root(), Operation::Dev, "Live preview", inspector.as_deref(), &env).await {
                Ok(r) => r,
                Err(e) => return Ok(fail("start", "The development server could not be started.", e.to_string(), e.to_string(), None, !matches!(e, GeniusError::MissingTool(_)))),
            };
            let port = run.port.unwrap_or_default();
            let url = format!("http://127.0.0.1:{port}/");
            self.hub.update(project_id, g, |s| {
                s.run_id = Some(run.id.clone());
                s.port = Some(port);
            });
            let deadline = Instant::now() + START_TIMEOUT;
            let mut compiling = false;
            let outcome: std::result::Result<std::result::Result<usize, CheckFailure>, Option<RunOutcome>> = loop {
                if cancel.is_cancelled() {
                    let _ = self.procs.stop(&run.id).await;
                    return Ok(self.hub.state(project_id));
                }
                if let Some(o) = self.procs.outcome(&run.id) {
                    break Err(Some(o));
                }
                if Instant::now() > deadline {
                    break Err(None);
                }
                if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                    if !compiling {
                        compiling = true;
                        self.hub.update(project_id, g, |s| {
                            s.status = PreviewStatus::Compiling;
                            s.message = "Compiling your app…".into();
                            s.url = Some(url.clone());
                        });
                    }
                    match preview::check_server(&self.hub.client, port, token.as_deref(), Some(&root), COMPILE_BUDGET).await {
                        Err(CheckFailure::NotReady(_)) => {}
                        other => break Ok(other),
                    }
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            };
            match outcome {
                Err(Some(o)) => {
                    // EADDRINUSE, or EACCES on Windows when the port falls in a range reserved by
                    // Hyper-V/WSL: pick another port.
                    let collided = port_unavailable(&o.output_tail);
                    if collided && attempt < 2 {
                        continue;
                    }
                    let summary = if collided { "The preview port was taken by another program.".to_string() } else { "The development server stopped while starting.".to_string() };
                    return Ok(fail("start", "The development server stopped while starting.", summary, tail(&o.output_tail, 6000), Some(run.id), !collided));
                }
                Err(None) => {
                    let _ = self.procs.stop(&run.id).await;
                    let out = self.db.list_runs(project_id, 5)?.into_iter().find(|r| r.id == run.id).map(|r| r.output_tail).unwrap_or_default();
                    return Ok(fail(
                        "start",
                        "The development server did not become ready.",
                        format!("The development server did not answer within {} seconds.", START_TIMEOUT.as_secs()),
                        tail(&out, 6000),
                        Some(run.id),
                        true,
                    ));
                }
                Ok(Err(CheckFailure::WrongServer(m))) => {
                    let _ = self.procs.stop(&run.id).await;
                    if attempt < 2 {
                        continue;
                    }
                    return Ok(fail("identity", "Another program answered on the preview port.", m.clone(), m, Some(run.id), false));
                }
                Ok(result) => {
                    let ok = result.is_ok();
                    let st = self.hub.update(project_id, g, |s| {
                        s.url = Some(url.clone());
                        apply_check(s, result, Some(run.id.clone()));
                    });
                    if ok {
                        self.bus.record(
                            Some(project_id),
                            task_id,
                            EventKind::Preview,
                            EventStatus::Completed,
                            format!("Preview is live at {url}"),
                            json!({ "url": url, "runId": run.id, "verified": "HTTP answer with this run's identity and a compiled module graph" }),
                        );
                    } else if let Some(p) = st.as_ref().and_then(|s| s.problem.clone()) {
                        self.bus.record(Some(project_id), task_id, EventKind::Preview, EventStatus::Failed, p.summary.clone(), json!({ "url": url, "runId": run.id, "details": p.details }));
                    }
                    self.watch_preview(project_id, generation, &run.id, &root);
                    return Ok(st.unwrap_or_else(|| self.hub.state(project_id)));
                }
            }
        }
        Ok(self.hub.state(project_id))
    }

    /// Follow a running server: re-verify after file changes or reported errors, and report an
    /// unexpected exit. Events of an older generation are ignored.
    fn watch_preview(&self, project_id: &str, generation: u64, run_id: &str, root: &str) {
        let hub = self.hub.clone();
        let procs = self.procs.clone();
        let mut lines = procs.subscribe_lines();
        let (pid, rid, root) = (project_id.to_string(), run_id.to_string(), root.to_string());
        self.rt.spawn(async move {
            let mut done = Box::pin({
                let procs = procs.clone();
                let rid = rid.clone();
                async move { procs.wait(&rid).await }
            });
            let mut due: Option<Instant> = None;
            loop {
                let sleep = async {
                    match due {
                        Some(t) => tokio::time::sleep_until(t.into()).await,
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::select! {
                    o = &mut done => {
                        let o = o.ok();
                        hub.update(&pid, Some(generation), |s| {
                            // Stopped on purpose (stop, restart, install): not a crash.
                            if s.status.busy() || s.status == PreviewStatus::Idle { return; }
                            let st = o.as_ref().map(|o| o.status.clone()).unwrap_or_default();
                            if st == "cancelled" {
                                s.status = PreviewStatus::Idle;
                                s.message = "The preview was stopped.".into();
                                s.url = None;
                                return;
                            }
                            s.status = PreviewStatus::Failed;
                            s.message = "The development server stopped unexpectedly.".into();
                            s.url = None;
                            s.problem = Some(PreviewProblem {
                                stage: "start".into(),
                                summary: format!("The development server exited (exit code {}).", o.as_ref().and_then(|o| o.exit_code).map(|c| c.to_string()).unwrap_or_else(|| "none".into())),
                                details: o.map(|o| tail(&o.output_tail, 6000)).unwrap_or_default(),
                                run_id: Some(rid.clone()),
                                fixable: true,
                                file: None,
                            });
                        });
                        break;
                    }
                    l = lines.recv() => {
                        match l {
                            Ok((r, line)) if r == rid && (preview::is_error_line(&line) || preview::is_update_line(&line)) => {
                                due = Some(Instant::now() + Duration::from_millis(600));
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            _ => {}
                        }
                    }
                    _ = sleep => {
                        due = None;
                        if hub.state(&pid).generation != generation { break; }
                        hub.recheck(&pid, &root).await;
                    }
                }
            }
        });
    }

    /// Debounced background re-verification of a running preview after Genius wrote files.
    pub(crate) fn schedule_preview_recheck(&self, project_id: &str) {
        let st = self.hub.state(project_id);
        if !matches!(st.status, PreviewStatus::Ready | PreviewStatus::Failed) || !st.run_id.as_deref().is_some_and(|r| self.procs.is_running(r)) {
            return;
        }
        let Ok(ws) = self.workspace(project_id) else { return };
        let root = ws.root().to_string_lossy().to_string();
        let hub = self.hub.clone();
        let pid = project_id.to_string();
        let generation = st.generation;
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if hub.state(&pid).generation == generation {
                hub.recheck(&pid, &root).await;
            }
        });
    }

    pub async fn stop_preview(&self, project_id: &str) -> Result<()> {
        let (generation, _) = self.hub.begin(project_id);
        self.hub.update(project_id, Some(generation), |s| {
            s.status = PreviewStatus::Stopping;
            s.message = "Stopping the preview…".into();
        });
        if let Some(r) = self.procs.dev_server(project_id) {
            self.procs.stop(&r.id).await?;
        }
        self.hub.update(project_id, Some(generation), |s| {
            s.status = PreviewStatus::Idle;
            s.message = "The preview is not running.".into();
            s.url = None;
            s.port = None;
            s.run_id = None;
            s.problem = None;
        });
        Ok(())
    }

    /// After files changed: re-verify a running preview (Vite reloads it by itself), or restart
    /// it when the installed dependencies changed underneath it.
    pub async fn refresh_preview_after_change(&self, project_id: &str, task_id: Option<&str>, deps_changed: bool) -> Option<PreviewState> {
        let st = self.hub.state(project_id);
        // No run id: the user has not started a preview; do not start one behind their back.
        let run_id = st.run_id.clone()?;
        if deps_changed || !self.procs.is_running(&run_id) {
            return self.ensure_preview(project_id, task_id).await.ok();
        }
        let root = self.workspace(project_id).ok()?.root().to_string_lossy().to_string();
        // give Vite's watcher a moment to pick up the new files
        tokio::time::sleep(Duration::from_millis(400)).await;
        self.hub.recheck(project_id, &root).await
    }

    // ================================================================= verification

    /// Install (if needed) → production build → preview, each judged on its own. A failing
    /// production build does not stop the development preview, and neither hides the other.
    pub async fn verify_pipeline(&self, project_id: &str, task_id: &str, cancel: &CancellationToken, start_preview: bool) -> Result<Value> {
        let mut report = json!({});
        let dep = self.dep_status(project_id)?;
        if dep.state == DepState::InvalidPackageJson {
            report["ok"] = json!(false);
            report["failedStep"] = json!("config");
            report["error"] = json!(dep.reason);
            return Ok(report);
        }
        let installed = match self.ensure_installed(project_id, Some(task_id), cancel).await {
            Ok(Some(out)) => {
                report["install"] = json!(out.status);
                if !out.ok() {
                    report["ok"] = json!(false);
                    report["failedStep"] = json!("install");
                    report["installSummary"] = json!(classify_install(&out.output_tail).0);
                    return Ok(report);
                }
                true
            }
            Ok(None) => false,
            Err(GeniusError::Cancelled) if cancel.is_cancelled() => return Err(GeniusError::Cancelled),
            Err(GeniusError::Cancelled) => {
                report["ok"] = json!(false);
                report["failedStep"] = json!("install");
                report["installSummary"] = json!("Installing the new packages was not approved.");
                return Ok(report);
            }
            Err(e) => return Err(e),
        };
        let build = self.run_operation(project_id, Some(task_id), Operation::Build, "Verify that the project compiles", false, cancel).await?;
        report["build"] = json!({ "status": build.status, "exitCode": build.exit_code, "runId": build.id, "summary": if build.ok() { "The production build succeeded.".to_string() } else { build_summary(&build.output_tail) } });

        // The development preview is judged independently of the production build.
        let preview_state = if start_preview { Some(self.ensure_preview(project_id, Some(task_id)).await?) } else { self.refresh_preview_after_change(project_id, Some(task_id), installed).await };
        if let Some(p) = &preview_state {
            report["preview"] = json!({ "status": p.status, "url": p.url, "problem": p.problem });
            if p.status == PreviewStatus::Ready {
                report["previewUrl"] = json!(p.url);
            }
        }

        if !build.ok() {
            report["ok"] = json!(false);
            report["failedStep"] = json!("build");
            let attempts = self.repair_attempts(project_id).len() as u32;
            let max = self.settings()?.max_repair_attempts;
            if attempts < max {
                match Box::pin(self.diagnose(project_id, task_id, Some(&build.id), cancel)).await {
                    Ok(v) => report["doctor"] = v,
                    Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                    Err(e) => report["doctor"] = json!({ "error": { "kind": e.kind(), "message": e.to_string() } }),
                }
            } else {
                report["doctor"] = json!({ "stopped": format!("Stopped after {max} automatic repair attempts. Review the diagnostics, edit the code, or ask for a specific change.") });
                self.bus.record(
                    Some(project_id),
                    Some(task_id),
                    EventKind::Diagnosis,
                    EventStatus::Failed,
                    format!("Build Doctor stopped after {max} attempts"),
                    json!({ "attempts": self.repair_attempts(project_id) }),
                );
            }
            return Ok(report);
        }
        self.reset_repair_attempts(project_id);
        let preview_ok = preview_state.as_ref().is_none_or(|p| p.status == PreviewStatus::Ready);
        report["ok"] = json!(preview_ok);
        if !preview_ok {
            report["failedStep"] = json!("preview");
        }
        Ok(report)
    }

    // ================================================================= Fix automatically

    /// Run the real checks in order and return the first failure (None = everything passed).
    /// `with_preview` includes the development preview (installs and starts it if necessary).
    pub async fn find_failure(&self, project_id: &str, task_id: &str, with_preview: bool, cancel: &CancellationToken) -> Result<Option<Failure>> {
        let dep = self.dep_status(project_id)?;
        match dep.state {
            DepState::NoPackageJson => {
                return Ok(Some(Failure {
                    kind: "config".into(),
                    summary: "This project has no package.json.".into(),
                    command: "read package.json".into(),
                    exit_code: None,
                    output: dep.reason,
                    run_id: None,
                    file: Some("package.json".into()),
                    fixable: false,
                }));
            }
            DepState::InvalidPackageJson => {
                return Ok(Some(Failure {
                    kind: "config".into(),
                    summary: dep.reason.clone(),
                    command: "read package.json".into(),
                    exit_code: None,
                    output: dep.reason,
                    run_id: None,
                    file: Some("package.json".into()),
                    fixable: true,
                }));
            }
            _ => {}
        }
        match self.ensure_installed(project_id, Some(task_id), cancel).await {
            Ok(Some(out)) if !out.ok() => {
                let (summary, fixable) = classify_install(&out.output_tail);
                return Ok(Some(Failure {
                    kind: "install".into(),
                    summary,
                    command: "npm install".into(),
                    exit_code: out.exit_code,
                    output: tail(&out.output_tail, 12_000),
                    run_id: Some(out.id),
                    file: Some("package.json".into()),
                    fixable,
                }));
            }
            Ok(_) => {}
            Err(GeniusError::Cancelled) if !cancel.is_cancelled() => {
                return Ok(Some(Failure {
                    kind: "install".into(),
                    summary: "Installing the new packages was not approved.".into(),
                    command: "npm install".into(),
                    exit_code: None,
                    output: String::new(),
                    run_id: None,
                    file: None,
                    fixable: false,
                }));
            }
            Err(e) => return Err(e),
        }
        if with_preview {
            let st = self.ensure_preview(project_id, Some(task_id)).await?;
            if st.status != PreviewStatus::Ready {
                let p = st.problem.clone().unwrap_or_default();
                return Ok(Some(Failure {
                    kind: "preview".into(),
                    summary: if p.summary.is_empty() { st.message.clone() } else { p.summary },
                    command: "npm run dev (Genius compile check)".into(),
                    exit_code: None,
                    output: p.details,
                    run_id: p.run_id,
                    file: p.file,
                    fixable: p.fixable,
                }));
            }
        }
        let build = self.run_operation(project_id, Some(task_id), Operation::Build, "Check that the project builds", false, cancel).await?;
        if !build.ok() {
            return Ok(Some(Failure {
                kind: "build".into(),
                summary: build_summary(&build.output_tail),
                command: "npm run build".into(),
                exit_code: build.exit_code,
                output: build.output_tail.clone(),
                run_id: Some(build.id),
                file: None,
                fixable: true,
            }));
        }
        Ok(None)
    }

    /// Start "Fix automatically". `runtime_error` carries an error reported by the running page.
    pub fn start_auto_fix(self: &Arc<Self>, project_id: &str, runtime_error: Option<String>) -> Result<crate::app::TaskInfo> {
        if self.settings()?.model.is_none() {
            return Err(crate::nvidia::ProviderError::NoModel.into());
        }
        self.client()?;
        let me = self.clone();
        let pid = project_id.to_string();
        self.spawn_task(project_id, "repair", "Fix automatically", move |tid, cancel| async move { me.auto_fix(&pid, &tid, runtime_error.as_deref(), &cancel).await })
    }

    /// Bounded repair loop: check → (AI) minimal fix → validate → apply (revertible) → re-check.
    /// Succeeds only when the real checks pass afterwards.
    pub async fn auto_fix(&self, project_id: &str, task_id: &str, runtime_error: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
        self.reset_repair_attempts(project_id);
        let with_preview = true;
        let mut attempts: Vec<Value> = Vec::new();
        let mut applied: Vec<String> = Vec::new();
        let mut runtime = runtime_error.map(|r| r.to_string());
        let conv = self.db.current_conversation(project_id)?;
        loop {
            if cancel.is_cancelled() {
                return Err(GeniusError::Cancelled);
            }
            let failure = match self.find_failure(project_id, task_id, with_preview, cancel).await? {
                Some(f) => Some(f),
                None => runtime.take().map(|r| Failure {
                    kind: "runtime".into(),
                    summary: "The page reported an error while running.".into(),
                    command: "browser (preview page)".into(),
                    exit_code: None,
                    output: r,
                    run_id: None,
                    file: None,
                    fixable: true,
                }),
            };
            let Some(failure) = failure else {
                let st = self.preview_state(project_id);
                let mut text = if applied.is_empty() {
                    "I checked the project and found nothing to fix: dependencies are installed, the app compiles in the preview and the production build succeeds.".to_string()
                } else {
                    format!("Fixed. I applied {} change(s) and verified the result: dependencies are installed, the app compiles in the preview and the production build succeeds.", applied.len())
                };
                if runtime_error.is_some() && !applied.is_empty() {
                    text.push_str(" The original error happened while the page was running; Genius cannot re-run your page's interactions, so reload the preview to confirm it is gone.");
                }
                self.db.add_message_in(
                    &conv.id,
                    project_id,
                    Some(task_id),
                    "assistant",
                    "answer",
                    &text,
                    &json!({ "autoFix": { "fixed": true, "attempts": attempts, "changesets": applied, "previewUrl": st.url } }),
                )?;
                return Ok(json!({ "fixed": true, "attempts": attempts, "changesets": applied, "preview": st }));
            };
            let give_up = |reason: String, attempts: &Vec<Value>| -> Result<Value> {
                let text = format!(
                    "I could not fix this automatically.\n\nProblem: {}\n{}\n\nWhat you can do: {}",
                    failure.summary,
                    reason,
                    if failure.fixable {
                        "describe what the app should do in the chat and I will make a targeted change, or open Advanced → Build output for the full log."
                    } else {
                        "this needs a change on your computer, not in the code (see above). Then press Fix automatically again."
                    }
                );
                self.db.add_message_in(&conv.id, project_id, Some(task_id), "assistant", "error", &text, &json!({ "autoFix": { "fixed": false, "failure": failure, "attempts": attempts } }))?;
                Err(GeniusError::Process(format!("Not fixed: {} {reason}", failure.summary)))
            };
            if !failure.fixable {
                return give_up("This problem cannot be solved by changing the project's files.".into(), &attempts);
            }
            if attempts.len() >= MAX_AUTOFIX_ATTEMPTS {
                return give_up(format!("I tried {} fixes; the checks still fail.", attempts.len()), &attempts);
            }
            let proposal = match self.propose_repair(project_id, task_id, &failure, cancel).await {
                Ok(p) => p,
                Err(GeniusError::Cancelled) => return Err(GeniusError::Cancelled),
                Err(e) => return give_up(format!("The repair step failed: {e}"), &attempts),
            };
            let Some((cs, root_cause)) = proposal else {
                return give_up("The AI did not propose a valid change for this error.".into(), &attempts);
            };
            let cs = self.apply_changeset_inner(project_id, Some(task_id), &cs.id)?;
            applied.push(cs.id.clone());
            attempts
                .push(json!({ "failure": failure.summary, "kind": failure.kind, "rootCause": root_cause, "changesetId": cs.id, "files": cs.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>() }));
        }
    }

    /// Ask the model for the smallest fix of a real failure; the proposal is validated (and
    /// re-requested once with the validation errors) before it is stored. Returns the proposed
    /// changeset and the stated root cause, or None when no valid change was produced.
    pub async fn propose_repair(&self, project_id: &str, task_id: &str, failure: &Failure, cancel: &CancellationToken) -> Result<Option<(Changeset, String)>> {
        let ws = self.workspace(project_id)?;
        let diags = diagnostics::parse(&failure.output);
        let mut referenced: Vec<String> = diagnostics::referenced_files(&failure.output, &diags).into_iter().filter(|f| ws.exists(f)).collect();
        if let Some(f) = &failure.file {
            if ws.exists(f) && !referenced.contains(f) {
                referenced.insert(0, f.clone());
            }
        }
        let about_packages = matches!(failure.kind.as_str(), "install" | "config") || failure.output.contains("Failed to resolve import") || failure.output.contains("Cannot find module");
        if about_packages && !referenced.iter().any(|f| f == "package.json") {
            referenced.push("package.json".into());
        }
        let mut ev = self.bus.start(
            Some(project_id),
            Some(task_id),
            EventKind::Diagnosis,
            EventStatus::Running,
            format!("Diagnosing: {}", failure.summary),
            json!({ "failure": failure.kind, "command": failure.command, "diagnostics": diags, "files": referenced }),
        );
        let p = self.db.get_project(project_id)?;
        let s = self.settings()?;
        let query: String = diags.iter().map(|d| d.message.clone()).chain(std::iter::once(failure.summary.clone())).collect::<Vec<_>>().join(" ");
        let plan = context::plan(&ws, &ContextRequest { query: &query, pinned: &[], excluded: &[], must: &referenced, budget_chars: s.context_budget_chars / 2 })?;
        let mut diag_text = if diags.is_empty() {
            format!("(no structured diagnostics recognised)\nSUMMARY: {}", failure.summary)
        } else {
            diags
                .iter()
                .map(|d| format!("{}:{}:{} {} {}", d.file.as_deref().unwrap_or("?"), d.line.unwrap_or(0), d.column.unwrap_or(0), d.code.as_deref().unwrap_or(""), d.message))
                .collect::<Vec<_>>()
                .join("\n")
        };
        diag_text.push_str(&missing_package_hints(ws.root(), &failure.output));
        let previous = self.repair_attempts(project_id);
        let mut messages =
            prompts::repair_messages(&failure.command, failure.exit_code, &diag_text, &diagnostics::focus_log(&failure.output, 12_000), &context::render(&ws, &plan), &previous, &p.notes);
        for round in 0..2 {
            let (r, events) = match self.ai_call(project_id, task_id, if round == 0 { "Diagnosing and preparing a fix" } else { "Correcting the proposed fix" }, &messages, true, cancel).await {
                Ok(x) => x,
                Err(e) => {
                    ev.update(if matches!(e, GeniusError::Cancelled) { EventStatus::Cancelled } else { EventStatus::Failed }, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                    return Err(e);
                }
            };
            let parsed = crate::blocks::collect(&events);
            let root_cause = parsed
                .explanation
                .lines()
                .find(|l| l.to_lowercase().starts_with("root cause"))
                .map(|l| l.trim().to_string())
                .unwrap_or_else(|| parsed.explanation.lines().next().unwrap_or("").to_string());
            let safe = self.safe_changes(&ws, &parsed.files, &parsed.deletes)?;
            if safe.changes.is_empty() {
                ev.update(EventStatus::Failed, json!({ "rootCause": root_cause, "note": "No applicable fix was proposed.", "rejectedPaths": safe.rejected }));
                return Ok(None);
            }
            if !safe.check.ok() {
                if round == 0 {
                    messages.push(crate::nvidia::ChatMessage::assistant(r.content));
                    messages.push(prompts::validation_feedback(&safe.check.blockers));
                    continue;
                }
                ev.update(EventStatus::Failed, json!({ "rootCause": root_cause, "blocked": safe.check.blockers }));
                return Ok(None);
            }
            self.repair_history
                .lock()
                .unwrap()
                .entry(project_id.to_string())
                .or_default()
                .push(format!("{root_cause} (files: {})", safe.changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>().join(", ")));
            let cs = ChangeStore::new(&self.db).create(
                project_id,
                Some(task_id),
                "repair",
                &format!("Fix: {}", failure.summary.chars().take(70).collect::<String>()),
                &parsed.explanation,
                &safe.changes,
            )?;
            ev.update(EventStatus::Completed, json!({ "rootCause": root_cause, "changesetId": cs.id, "warnings": safe.check.warnings }));
            return Ok(Some((cs, root_cause)));
        }
        Ok(None)
    }

    /// Diagnose a real failed run and propose a targeted repair (subject to the review policy).
    pub async fn diagnose(&self, project_id: &str, task_id: &str, run_id: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
        let runs = self.db.list_runs(project_id, 50)?;
        let run = match run_id {
            Some(id) => runs.into_iter().find(|r| r.id == id),
            None => runs.into_iter().find(|r| r.status == "failed" && r.op != "dev"),
        }
        .ok_or_else(|| GeniusError::NotFound("a failed build or test run to diagnose".into()))?;
        if run.status != "failed" {
            return Err(GeniusError::Validation(format!("That run did not fail (status: {}).", run.status)));
        }
        let failure = Failure {
            kind: if run.op == "install" { "install".into() } else { "build".into() },
            summary: if run.op == "install" { classify_install(&run.output_tail).0 } else { build_summary(&run.output_tail) },
            command: run.command.clone(),
            exit_code: run.exit_code,
            output: run.output_tail.clone(),
            run_id: Some(run.id.clone()),
            file: None,
            fixable: true,
        };
        let Some((cs, root_cause)) = self.propose_repair(project_id, task_id, &failure, cancel).await? else {
            return Ok(json!({ "changesetId": null, "note": "No valid fix was proposed." }));
        };
        self.db.add_message(project_id, Some(task_id), "assistant", &format!("Build Doctor — {}\n\n{}", run.command, cs.summary))?;
        let policy = self.settings()?.approvals.apply_ai_edits;
        Box::pin(self.finish_proposal(project_id, task_id, &cs, policy, cancel, json!({ "rootCause": root_cause }))).await
    }

    pub(crate) async fn gate_script(&self, project_id: &str, task_id: Option<&str>, op: Operation, cancel: &CancellationToken) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let pkg: Value = ws.read_text_opt("package.json")?.and_then(|t| deps::parse_strict(&t).ok()).ok_or_else(|| GeniusError::Validation("package.json is missing or invalid".into()))?;
        let Some(script) = op.script() else { return Ok(()) };
        let Some(actual) = pkg.pointer(&format!("/scripts/{script}")).and_then(|s| s.as_str()) else {
            return Err(GeniusError::Validation(format!("package.json has no \"{script}\" script")));
        };
        let trusted = crate::scaffold::trusted_scripts().iter().any(|(k, v)| *k == script && *v == actual);
        if !trusted {
            self.approvals
                .request(
                    &self.bus,
                    Some(project_id),
                    task_id,
                    "untrusted_script",
                    Policy::Ask,
                    &format!("Run modified npm script \"{script}\""),
                    json!({ "script": script, "command": actual, "note": "This script differs from the Genius template. It runs as a program on your computer." }),
                    cancel,
                )
                .await?;
        }
        Ok(())
    }
}

impl Genius {
    /// "Create a modern personal dashboard with notes…": create the project and run
    /// plan → generate → install → checks → preview as one cancellable task. With
    /// `review_plan`, stop after the plan so the user can edit it first.
    pub fn start_create_app(self: &Arc<Self>, name: Option<&str>, description: &str, review_plan: bool) -> Result<(crate::db::ProjectRow, crate::app::TaskInfo)> {
        let description = description.trim();
        if description.len() < 10 {
            return Err(GeniusError::Validation("Describe the app in at least a sentence.".into()));
        }
        self.client()?;
        if self.settings()?.model.is_none() {
            return Err(crate::nvidia::ProviderError::NoModel.into());
        }
        let name = name.map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| name_from_description(description));
        let p = self.create_project(&name, description)?;
        self.db.create_conversation(&p.id, "Project conversation")?;
        let me = self.clone();
        let (pid, desc) = (p.id.clone(), description.to_string());
        let task = self.spawn_task(&p.id, "generate", if review_plan { "Plan the application" } else { "Create the application" }, move |tid, cancel| async move {
            let spec = me.plan(&pid, &tid, &desc, "", &cancel).await?;
            if review_plan {
                return Ok(json!({ "spec": spec, "stoppedAfterPlan": true }));
            }
            let out = me.generate(&pid, &tid, &cancel).await?;
            let v = &out["verify"];
            let text = creation_report(v);
            me.db.add_message(&pid, Some(&tid), "assistant", &text)?;
            Ok(out)
        })?;
        Ok((self.db.get_project(&p.id)?, task))
    }
}

fn name_from_description(d: &str) -> String {
    let skip = ["a", "an", "the", "create", "build", "make", "me", "please", "modern", "simple", "with", "for", "app", "application", "website", "that", "i", "want", "to", "my"];
    let words: Vec<String> = d
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !skip.contains(&w.to_lowercase().as_str()))
        .take(3)
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase()).unwrap_or_default()
        })
        .collect();
    if words.is_empty() {
        "My App".into()
    } else {
        words.join(" ").chars().take(40).collect()
    }
}

/// Honest summary of a generation's verification for the conversation.
fn creation_report(v: &Value) -> String {
    let mut lines = Vec::new();
    match v.get("install").and_then(|x| x.as_str()) {
        Some("succeeded") => lines.push("✓ Dependencies installed.".to_string()),
        Some(_) => lines.push(format!("✗ Installing dependencies failed: {}", v.get("installSummary").and_then(|x| x.as_str()).unwrap_or("see the install output"))),
        None => {}
    }
    if let Some(b) = v.get("build") {
        let ok = b.get("status").and_then(|x| x.as_str()) == Some("succeeded");
        lines.push(if ok { "✓ Production build succeeded.".into() } else { format!("✗ Production build failed: {}", b.get("summary").and_then(|x| x.as_str()).unwrap_or("")) });
    }
    match v.pointer("/preview/status").and_then(|x| x.as_str()) {
        Some("ready") => lines.push(format!("✓ Preview is running at {} and the app compiled.", v.pointer("/preview/url").and_then(|x| x.as_str()).unwrap_or(""))),
        Some(_) => lines.push(format!("✗ Preview: {}", v.pointer("/preview/problem/summary").and_then(|x| x.as_str()).unwrap_or("did not start"))),
        None => lines.push("Preview was not started.".into()),
    }
    let ok = v.get("ok").and_then(|x| x.as_bool()) == Some(true);
    format!(
        "{}\n\n{}",
        if ok { "Your app is ready. Tell me what to change next." } else { "The code was written, but something still needs attention. Press “Fix automatically” to let me repair it." },
        lines.join("\n")
    )
}

/// Deterministic hints for the repair prompt: packages the error mentions that are imported
/// but not declared, or declared but not installed.
fn missing_package_hints(root: &Path, output: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r#"(?:Failed to resolve import|Cannot find module|Could not resolve)\s+["']([^"']+)["']"#).unwrap());
    let decl = std::fs::read_to_string(root.join("package.json")).ok().and_then(|t| deps::parse_strict(&t).ok()).map(|v| deps::declared(&v)).unwrap_or_default();
    let mut out = String::new();
    for c in RE.captures_iter(output) {
        let Some(pkg) = crate::proposal::package_of(&c[1]) else { continue };
        if !decl.contains_key(&pkg) {
            out.push_str(&format!("\nGENIUS CHECK: the package '{pkg}' is imported but NOT listed in package.json. Either add it to \"dependencies\" (output the complete package.json, keeping every existing entry and script) or stop importing it."));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_failures_are_classified() {
        assert!(classify_install("npm ERR! code ERESOLVE\nnpm ERR! ERESOLVE unable to resolve dependency tree").1);
        let (s, fixable) = classify_install("npm error code ENOTFOUND\nnpm error network request to https://registry.npmjs.org/react failed");
        assert!(!fixable && s.contains("internet"));
        let (s, fixable) = classify_install("npm error code ETARGET\nnpm error notarget No matching version found for uuid@^99.0.0.");
        assert!(fixable && s.contains("version"));
        assert!(!classify_install("npm error code EPERM\nnpm error syscall unlink").1);
        assert!(classify_install("npm error 404 Not Found - GET https://registry.npmjs.org/not-a-pkg").0.contains("does not exist"));
    }

    #[test]
    fn port_problems_are_recognised() {
        assert!(port_unavailable("Error: Port 5173 is already in use"));
        assert!(port_unavailable("Error: listen EACCES: permission denied 127.0.0.1:50123"));
        assert!(!port_unavailable("error TS2304: Cannot find name 'x'"));
    }

    #[test]
    fn names_are_derived_from_descriptions() {
        assert_eq!(name_from_description("Create a modern personal dashboard with notes, a weather widget, and quick links."), "Personal Dashboard Notes");
        assert_eq!(name_from_description("a an the"), "My App");
    }

    #[test]
    fn creation_report_is_honest() {
        let r = creation_report(
            &json!({ "ok": false, "install": "succeeded", "build": { "status": "failed", "summary": "TypeScript error in src/App.tsx line 3: x" }, "preview": { "status": "ready", "url": "http://127.0.0.1:5555/" } }),
        );
        assert!(r.contains("✗ Production build failed") && r.contains("✓ Preview is running") && r.contains("Fix automatically"));
    }

    #[test]
    fn hints_name_undeclared_packages() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("package.json"), r#"{"dependencies":{"react":"^19.0.0"}}"#).unwrap();
        let h = missing_package_hints(d.path(), "Failed to resolve import \"uuid\" from \"src/App.tsx\". Does the file exist?\nFailed to resolve import \"react/jsx-runtime\"");
        assert!(h.contains("'uuid' is imported but NOT listed"));
        assert!(!h.contains("'react'"));
    }
}
