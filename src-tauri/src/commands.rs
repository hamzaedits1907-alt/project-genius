//! IPC commands. Each is a thin wrapper over `genius_core::app::Genius`; all validation,
//! security checks and event reporting live in the core so the same behaviour is covered
//! by the core's tests. Errors serialize as `{ kind, message }` with secrets redacted.

use genius_core::app::{Genius, KeyStatus, ModelList, ReadinessCheck, TaskInfo};
use genius_core::approvals::ApprovalRequest;
use genius_core::changes::{Changeset, FileChange};
use genius_core::context::ContextPlan;
use genius_core::db::{MessageRow, ProjectRow, RunRow};
use genius_core::events::ActivityEvent;
use genius_core::export::ExportResult;
use genius_core::git::{Branch, Commit, RepoStatus};
use genius_core::github::{AuthStatus, RepoInfo};
use genius_core::nvidia::ProbeResult;
use genius_core::process::{Operation, RunInfo, ToolStatus};
use genius_core::secretscan::SecretFinding;
use genius_core::settings::Settings;
use genius_core::spec::ProjectSpec;
use genius_core::vcs::{GitView, PublishPlan, PublishRequest};
use genius_core::workspace::{FileEntry, SearchHit};
use genius_core::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::State;
use tokio_util::sync::CancellationToken;

type G<'a> = State<'a, Arc<Genius>>;

// ---------------------------------------------------------------- settings & AI provider

#[tauri::command]
pub fn get_settings(g: G) -> Result<Settings> {
    g.settings()
}

#[tauri::command]
pub fn save_settings(g: G, settings: Settings) -> Result<Settings> {
    g.save_settings(&settings)
}

#[tauri::command]
pub fn key_status(g: G) -> KeyStatus {
    g.key_status()
}

#[tauri::command]
pub fn save_api_key(g: G, key: String) -> Result<KeyStatus> {
    g.save_api_key(&key)
}

#[tauri::command]
pub fn remove_api_key(g: G) -> Result<KeyStatus> {
    g.remove_api_key()
}

#[tauri::command]
pub async fn test_connection(g: G<'_>) -> Result<Value> {
    g.test_connection().await
}

#[tauri::command]
pub async fn list_models(g: G<'_>, refresh: bool) -> Result<ModelList> {
    g.list_models(refresh).await
}

#[tauri::command]
pub async fn probe_model(g: G<'_>, model: String) -> Result<ProbeResult> {
    g.probe_model(&model).await
}

#[tauri::command]
pub fn select_model(g: G, model: String) -> Result<Settings> {
    g.select_model(&model)
}

#[tauri::command]
pub async fn detect_tools() -> Vec<ToolStatus> {
    genius_core::process::detect_tools().await
}

#[tauri::command]
pub fn app_info(g: G) -> Value {
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "dataDir": g.data_dir(),
        "workspaceDir": g.workspace_dir().ok(),
        "secretBackend": g.secrets.backend_name(),
        "platform": std::env::consts::OS,
    })
}

// ---------------------------------------------------------------- projects

#[tauri::command]
pub fn list_projects(g: G) -> Result<Vec<ProjectRow>> {
    g.list_projects()
}

#[tauri::command]
pub fn get_project(g: G, project_id: String) -> Result<ProjectRow> {
    g.project(&project_id)
}

#[tauri::command]
pub fn create_project(g: G, name: String, description: String) -> Result<ProjectRow> {
    g.create_project(&name, &description)
}

#[tauri::command]
pub fn import_project(g: G, path: String) -> Result<ProjectRow> {
    g.import_project(&path, None)
}

#[tauri::command]
pub fn open_project(g: G, project_id: String) -> Result<ProjectRow> {
    g.open_project(&project_id)
}

#[tauri::command]
pub fn update_project_meta(g: G, project_id: String, name: String, description: String, notes: String) -> Result<ProjectRow> {
    g.update_project_meta(&project_id, &name, &description, &notes)
}

#[tauri::command]
pub async fn delete_project(g: G<'_>, project_id: String, delete_files: bool) -> Result<Value> {
    g.delete_project(&project_id, delete_files).await
}

#[tauri::command]
pub fn update_spec(g: G, project_id: String, spec: ProjectSpec) -> Result<ProjectRow> {
    g.update_spec(&project_id, &spec)
}

#[tauri::command]
pub fn list_messages(g: G, project_id: String) -> Result<Vec<MessageRow>> {
    g.db.list_messages(&project_id, 200)
}

// ---------------------------------------------------------------- files

#[tauri::command]
pub fn list_files(g: G, project_id: String) -> Result<Vec<FileEntry>> {
    g.list_files(&project_id)
}

#[tauri::command]
pub fn read_file(g: G, project_id: String, path: String) -> Result<String> {
    g.read_file(&project_id, &path)
}

#[tauri::command]
pub fn save_file(g: G, project_id: String, path: String, content: String) -> Result<Option<Changeset>> {
    g.save_file(&project_id, &path, &content)
}

#[tauri::command]
pub fn delete_file(g: G, project_id: String, path: String) -> Result<Changeset> {
    g.delete_file(&project_id, &path)
}

#[tauri::command]
pub fn search_files(g: G, project_id: String, query: String) -> Result<Vec<SearchHit>> {
    g.search_files(&project_id, &query)
}

// ---------------------------------------------------------------- AI tasks

#[tauri::command]
pub fn start_plan(g: G, project_id: String, description: String, constraints: String) -> Result<TaskInfo> {
    g.start_plan(&project_id, &description, &constraints)
}

#[tauri::command]
pub fn start_generate(g: G, project_id: String) -> Result<TaskInfo> {
    g.start_generate(&project_id)
}

#[tauri::command]
pub fn preview_context(g: G, project_id: String, request: String, pinned: Vec<String>, excluded: Vec<String>) -> Result<ContextPlan> {
    g.preview_context(&project_id, &request, &pinned, &excluded)
}

#[tauri::command]
pub fn start_change(g: G, project_id: String, request: String, pinned: Vec<String>, excluded: Vec<String>) -> Result<TaskInfo> {
    g.start_change(&project_id, &request, pinned, excluded)
}

#[tauri::command]
pub fn start_diagnose(g: G, project_id: String, run_id: Option<String>) -> Result<TaskInfo> {
    g.start_diagnose(&project_id, run_id)
}

#[tauri::command]
pub fn cancel_task(g: G, task_id: String) -> Result<()> {
    g.cancel_task(&task_id)
}

#[tauri::command]
pub fn active_tasks(g: G, project_id: Option<String>) -> Vec<TaskInfo> {
    g.active_tasks(project_id.as_deref())
}

#[tauri::command]
pub fn repair_attempts(g: G, project_id: String) -> Vec<String> {
    g.repair_attempts(&project_id)
}

#[tauri::command]
pub fn reset_repair_attempts(g: G, project_id: String) {
    g.reset_repair_attempts(&project_id)
}

// ---------------------------------------------------------------- approvals

#[tauri::command]
pub fn pending_approvals(g: G, project_id: Option<String>) -> Vec<ApprovalRequest> {
    g.approvals.pending(project_id.as_deref())
}

#[tauri::command]
pub fn respond_approval(g: G, approval_id: String, approved: bool) -> Result<()> {
    g.approvals.respond(&approval_id, approved)
}

// ---------------------------------------------------------------- changesets / history

#[tauri::command]
pub fn list_changesets(g: G, project_id: String) -> Result<Vec<Changeset>> {
    g.list_changesets(&project_id)
}

#[tauri::command]
pub fn get_changeset(g: G, changeset_id: String) -> Result<Value> {
    let (cs, files): (Changeset, Vec<FileChange>) = g.changeset(&changeset_id)?;
    Ok(json!({ "changeset": cs, "files": files }))
}

#[tauri::command]
pub fn apply_changeset(g: G, changeset_id: String) -> Result<Changeset> {
    g.apply_changeset(&changeset_id)
}

#[tauri::command]
pub fn reject_changeset(g: G, changeset_id: String) -> Result<Changeset> {
    g.reject_changeset(&changeset_id)
}

#[tauri::command]
pub fn propose_revert(g: G, changeset_id: String, force: bool) -> Result<Changeset> {
    g.propose_revert(&changeset_id, force)
}

#[tauri::command]
pub fn propose_restore_to(g: G, changeset_id: String) -> Result<Changeset> {
    g.propose_restore_to(&changeset_id)
}

// ---------------------------------------------------------------- execution & preview

#[tauri::command]
pub fn start_operation(g: G, project_id: String, op: Operation) -> Result<TaskInfo> {
    g.start_operation(&project_id, op)
}

#[tauri::command]
pub fn list_runs(g: G, project_id: String) -> Result<Vec<RunRow>> {
    g.db.list_runs(&project_id, 100)
}

#[tauri::command]
pub fn active_runs(g: G, project_id: Option<String>) -> Vec<RunInfo> {
    g.procs.active(project_id.as_deref())
}

#[tauri::command]
pub async fn stop_run(g: G<'_>, run_id: String) -> Result<()> {
    g.procs.stop(&run_id).await
}

#[tauri::command]
pub fn needs_install(g: G, project_id: String) -> Result<bool> {
    g.needs_install(&project_id)
}

#[tauri::command]
pub async fn start_preview(g: G<'_>, project_id: String) -> Result<String> {
    g.start_preview(&project_id, None).await
}

#[tauri::command]
pub async fn stop_preview(g: G<'_>, project_id: String) -> Result<()> {
    g.stop_preview(&project_id).await
}

#[tauri::command]
pub fn preview_status(g: G, project_id: String) -> Option<String> {
    g.preview_url(&project_id)
}

// ---------------------------------------------------------------- activity, export, readiness

#[tauri::command]
pub fn list_activity(g: G, project_id: Option<String>, limit: Option<usize>) -> Result<Vec<ActivityEvent>> {
    g.bus.list(project_id.as_deref(), limit.unwrap_or(1000).min(3000))
}

#[tauri::command]
pub fn export_project(g: G, project_id: String, destination: String, zip: bool) -> Result<ExportResult> {
    g.export_project(&project_id, &destination, zip)
}

#[tauri::command]
pub async fn readiness(g: G<'_>, project_id: String) -> Result<Vec<ReadinessCheck>> {
    g.readiness(&project_id).await
}

// ---------------------------------------------------------------- git

#[tauri::command]
pub async fn git_view(g: G<'_>, project_id: String) -> Result<GitView> {
    g.git_view(&project_id).await
}

#[tauri::command]
pub async fn git_init(g: G<'_>, project_id: String) -> Result<Value> {
    g.git_init(&project_id).await
}

#[tauri::command]
pub async fn git_diff(g: G<'_>, project_id: String, path: String, staged: bool, untracked: bool) -> Result<String> {
    g.git_diff(&project_id, &path, staged, untracked).await
}

#[tauri::command]
pub async fn git_stage(g: G<'_>, project_id: String, paths: Vec<String>, stage: bool) -> Result<RepoStatus> {
    g.git_stage(&project_id, paths, stage).await
}

#[tauri::command]
pub async fn staged_findings(g: G<'_>, project_id: String) -> Result<Vec<SecretFinding>> {
    g.staged_findings(&project_id).await
}

#[tauri::command]
pub async fn git_commit(g: G<'_>, project_id: String, message: String, allow_findings: bool) -> Result<Commit> {
    g.git_commit(&project_id, &message, allow_findings).await
}

#[tauri::command]
pub async fn suggest_commit_message(g: G<'_>, project_id: String) -> Result<String> {
    g.suggest_commit_message(&project_id).await
}

#[tauri::command]
pub async fn git_log(g: G<'_>, project_id: String, limit: usize) -> Result<Vec<Commit>> {
    g.git_log(&project_id, limit).await
}

#[tauri::command]
pub async fn git_commit_detail(g: G<'_>, project_id: String, sha: String) -> Result<Value> {
    g.git_commit_detail(&project_id, &sha).await
}

#[tauri::command]
pub async fn git_branches(g: G<'_>, project_id: String) -> Result<Vec<Branch>> {
    g.git_branches(&project_id).await
}

#[tauri::command]
pub async fn git_branch(g: G<'_>, project_id: String, name: String, create: bool) -> Result<()> {
    g.git_branch(&project_id, &name, create).await
}

#[tauri::command]
pub async fn git_set_identity(g: G<'_>, project_id: String, name: String, email: String) -> Result<()> {
    g.git_set_identity(&project_id, &name, &email).await
}

#[tauri::command]
pub async fn git_set_remote(g: G<'_>, project_id: String, name: String, url: String) -> Result<()> {
    g.git_set_remote(&project_id, &name, &url).await
}

#[tauri::command]
pub async fn git_fetch(g: G<'_>, project_id: String) -> Result<RepoStatus> {
    g.git_fetch(&project_id).await
}

#[tauri::command]
pub async fn git_pull(g: G<'_>, project_id: String, allow_merge: bool) -> Result<RepoStatus> {
    g.git_pull(&project_id, allow_merge).await
}

#[tauri::command]
pub async fn git_push(g: G<'_>, project_id: String) -> Result<Value> {
    g.git_push(&project_id, &CancellationToken::new()).await
}

#[tauri::command]
pub async fn git_merge_abort(g: G<'_>, project_id: String) -> Result<()> {
    g.git_merge_abort(&project_id).await
}

#[tauri::command]
pub async fn git_resolve(g: G<'_>, project_id: String, path: String, side: String) -> Result<()> {
    g.git_resolve(&project_id, &path, &side).await
}

#[tauri::command]
pub async fn propose_conflict_resolution(g: G<'_>, project_id: String, path: String) -> Result<Changeset> {
    g.propose_conflict_resolution(&project_id, &path).await
}

#[tauri::command]
pub fn ensure_gitignore(g: G, project_id: String) -> Result<Vec<String>> {
    g.ensure_gitignore(&project_id)
}

// ---------------------------------------------------------------- GitHub

#[tauri::command]
pub async fn github_status(g: G<'_>) -> Result<AuthStatus> {
    Ok(g.github_status().await)
}

#[tauri::command]
pub fn github_login_web(g: G) -> Result<()> {
    g.github_login_web()
}

#[tauri::command]
pub async fn github_login_token(g: G<'_>, token: String) -> Result<AuthStatus> {
    g.github_login_token(&token).await
}

#[tauri::command]
pub async fn github_logout(g: G<'_>) -> Result<AuthStatus> {
    g.github_logout().await
}

#[tauri::command]
pub async fn github_repos(g: G<'_>, owner: Option<String>) -> Result<Vec<RepoInfo>> {
    g.github_repos(owner).await
}

#[tauri::command]
pub async fn github_orgs(g: G<'_>) -> Result<Vec<String>> {
    g.github_orgs().await
}

#[tauri::command]
pub async fn github_clone(g: G<'_>, name_with_owner: String) -> Result<ProjectRow> {
    g.github_clone(&name_with_owner).await
}

#[tauri::command]
pub async fn publish_plan(g: G<'_>, project_id: String) -> Result<PublishPlan> {
    g.publish_plan(&project_id).await
}

#[tauri::command]
pub async fn publish(g: G<'_>, project_id: String, request: PublishRequest) -> Result<Value> {
    g.publish(&project_id, &request, &CancellationToken::new()).await
}
