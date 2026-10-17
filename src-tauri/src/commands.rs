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
use genius_core::pages::DeployStatus;
use genius_core::process::{Operation, RunInfo, ToolStatus};
use genius_core::runtime_errors::PreviewRuntimeError;
use genius_core::secretscan::SecretFinding;
use genius_core::settings::Settings;
use genius_core::spec::ProjectSpec;
use genius_core::vcs::{GitView, PublishPlan, PublishRequest};
use genius_core::workspace::{FileEntry, SearchHit};
use genius_core::Result;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{AppHandle, Emitter as _, State};
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

/// Runtime errors posted by the preview page; returns the merged current list.
#[tauri::command]
pub fn report_preview_errors(g: G, project_id: String, errors: Vec<PreviewRuntimeError>) -> Result<Vec<PreviewRuntimeError>> {
    g.report_preview_errors(&project_id, errors)?;
    Ok(g.preview_errors(&project_id))
}

#[tauri::command]
pub fn preview_errors(g: G, project_id: String) -> Vec<PreviewRuntimeError> {
    g.preview_errors(&project_id)
}

#[tauri::command]
pub fn clear_preview_errors(g: G, project_id: String) {
    g.clear_preview_errors(&project_id)
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

#[tauri::command]
pub fn deploy_pages(g: G, project_id: String) -> Result<TaskInfo> {
    g.start_deploy_pages(&project_id)
}

#[tauri::command]
pub fn deploy_status(g: G, project_id: String) -> Result<Option<DeployStatus>> {
    g.deploy_status(&project_id)
}

// ---------------------------------------------------------------- diagnostics

#[tauri::command]
pub fn record_ui_error(g: G, report: genius_core::uilog::UiErrorReport) -> Result<String> {
    log::error!("UI error in {}: {}", report.view, genius_core::redact::redact(&report.message));
    genius_core::uilog::record(g.data_dir(), &report, &genius_core::db::now()).map(|p| p.display().to_string())
}

#[tauri::command]
pub fn recent_ui_errors(g: G, limit: Option<usize>) -> Vec<Value> {
    genius_core::uilog::recent(g.data_dir(), limit.unwrap_or(20).min(200))
}

// ---------------------------------------------------------------- conversations & agent

#[tauri::command]
pub fn list_conversations(g: G, project_id: String) -> Result<Vec<genius_core::db::ConversationRow>> {
    g.db.list_conversations(&project_id)
}

#[tauri::command]
pub fn create_conversation(g: G, project_id: String, title: Option<String>) -> Result<genius_core::db::ConversationRow> {
    g.db.get_project(&project_id)?;
    g.db.create_conversation(&project_id, title.as_deref().filter(|t| !t.trim().is_empty()).unwrap_or("New conversation"))
}

#[tauri::command]
pub fn rename_conversation(g: G, conversation_id: String, title: String) -> Result<()> {
    g.db.rename_conversation(&conversation_id, title.trim())
}

#[tauri::command]
pub fn delete_conversation(g: G, conversation_id: String) -> Result<()> {
    g.db.delete_conversation(&conversation_id)
}

#[tauri::command]
pub fn conversation_messages(g: G, conversation_id: String) -> Result<Vec<MessageRow>> {
    g.db.list_conversation_messages(&conversation_id, 500)
}

#[tauri::command]
pub fn start_agent(g: G, project_id: String, request: genius_core::agent::AgentRequest) -> Result<TaskInfo> {
    g.start_agent(&project_id, request)
}

#[tauri::command]
pub fn agent_steps(g: G, task_id: String) -> Result<Vec<genius_core::db::AgentStepRow>> {
    g.agent_steps(&task_id)
}

#[tauri::command]
pub async fn capability_report(g: G<'_>, project_id: Option<String>) -> Result<genius_core::capabilities::CapabilityReport> {
    g.capability_report(project_id.as_deref()).await
}

// ---------------------------------------------------------------- project memory

#[tauri::command]
pub fn list_memories(g: G, project_id: String) -> Result<Vec<genius_core::db::MemoryRow>> {
    g.db.list_memories(&project_id)
}

#[tauri::command]
pub fn add_memory(g: G, project_id: String, kind: String, content: String) -> Result<genius_core::db::MemoryRow> {
    if content.trim().is_empty() || content.len() > 2000 {
        return Err(genius_core::GeniusError::Validation("memory must be 1-2000 characters".into()));
    }
    g.db.get_project(&project_id)?;
    g.db.add_memory(&project_id, &kind, content.trim(), "user", "added by you")
}

#[tauri::command]
pub fn update_memory(g: G, memory_id: String, content: String) -> Result<()> {
    g.db.update_memory(&memory_id, content.trim(), "user", "edited by you")
}

#[tauri::command]
pub fn delete_memory(g: G, project_id: String, memory_id: String) -> Result<()> {
    g.db.delete_memory(&project_id, &memory_id)
}

// ---------------------------------------------------------------- imported files

#[tauri::command]
pub fn import_attachment(g: G, project_id: String, path: String, scope: String, conversation_id: Option<String>) -> Result<genius_core::attachments::Attachment> {
    g.import_attachment(&project_id, &path, &scope, conversation_id.as_deref())
}

#[tauri::command]
pub fn import_attachment_data(g: G, project_id: String, name: String, data_base64: String, scope: String, conversation_id: Option<String>) -> Result<genius_core::attachments::Attachment> {
    use base64::Engine;
    if data_base64.len() as u64 > genius_core::attachments::MAX_IMPORT_BYTES * 4 / 3 + 16 {
        return Err(genius_core::GeniusError::Validation("file exceeds the import limit".into()));
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(data_base64.as_bytes()).map_err(|e| genius_core::GeniusError::Validation(format!("invalid file data: {e}")))?;
    g.import_attachment_bytes(&project_id, &name, &bytes, &scope, conversation_id.as_deref())
}

#[tauri::command]
pub fn list_attachments(g: G, project_id: String) -> Result<Vec<genius_core::attachments::Attachment>> {
    g.list_attachments(&project_id)
}

#[tauri::command]
pub fn attachment_preview(g: G, project_id: String, attachment_id: String) -> Result<Value> {
    g.attachment_preview(&project_id, &attachment_id)
}

#[tauri::command]
pub fn set_attachment_scope(g: G, project_id: String, attachment_id: String, scope: String) -> Result<genius_core::attachments::Attachment> {
    g.set_attachment_scope(&project_id, &attachment_id, &scope)
}

#[tauri::command]
pub fn delete_attachment(g: G, project_id: String, attachment_id: String) -> Result<()> {
    g.delete_attachment(&project_id, &attachment_id)
}

#[tauri::command]
pub fn supported_import_types() -> Vec<String> {
    genius_core::attachments::supported_extensions()
}

#[tauri::command]
pub fn import_zip_project(g: G, path: String) -> Result<ProjectRow> {
    g.import_zip_project(&path)
}

#[tauri::command]
pub async fn probe_vision(g: G<'_>, model: String) -> Result<Value> {
    g.probe_vision(&model).await
}

// ---------------------------------------------------------------- integrations

#[tauri::command]
pub fn integration_statuses(g: G) -> Vec<Value> {
    g.integration_statuses()
}

#[tauri::command]
pub async fn supabase_connect(g: G<'_>, token: String) -> Result<Value> {
    g.supabase_connect(&token).await
}

#[tauri::command]
pub fn supabase_disconnect(g: G) -> Result<Value> {
    g.supabase_disconnect()
}

#[tauri::command]
pub async fn supabase_projects(g: G<'_>) -> Result<Vec<genius_core::supabase::SupabaseProject>> {
    g.supabase_projects().await
}

#[tauri::command]
pub async fn supabase_link(g: G<'_>, project_id: String, reference: String) -> Result<Value> {
    g.supabase_link(&project_id, &reference).await
}

#[tauri::command]
pub async fn supabase_status(g: G<'_>, project_id: String) -> Result<Value> {
    g.supabase_status(&project_id).await
}

#[tauri::command]
pub async fn supabase_configure(g: G<'_>, project_id: String) -> Result<Value> {
    g.supabase_configure(&project_id).await
}

#[tauri::command]
pub fn supabase_propose_migration(g: G, project_id: String, name: String, sql: String) -> Result<Value> {
    g.supabase_propose_migration(&project_id, &name, &sql)
}

#[tauri::command]
pub fn supabase_migrations(g: G, project_id: String) -> Result<Vec<Value>> {
    g.supabase_migrations(&project_id)
}

#[tauri::command]
pub async fn supabase_apply_migration(g: G<'_>, project_id: String, path: String) -> Result<Value> {
    g.supabase_apply_migration(&project_id, &path, &CancellationToken::new()).await
}

#[tauri::command]
pub async fn supabase_generate_types(g: G<'_>, project_id: String) -> Result<Value> {
    g.supabase_generate_types(&project_id).await
}

// ---------------------------------------------------------------- updates

const UPDATE_PROGRESS_EVENT: &str = "genius://update-progress";

fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    current_version(&app)
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<genius_core::updater::UpdateInfo> {
    genius_core::updater::check(&current_version(&app)).await
}

/// Re-check, download (reporting progress on `genius://update-progress`), verify and launch
/// the installer. Quits or restarts the app afterwards when the platform requires it.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<String> {
    use genius_core::updater::{self, PostInstall};
    let info = updater::check(&current_version(&app)).await?;
    if !info.available {
        return Err(genius_core::GeniusError::Validation(
            info.detail.unwrap_or_else(|| "Genius is already up to date.".into()),
        ));
    }
    let asset = info.asset.ok_or_else(|| {
        genius_core::GeniusError::Validation(
            info.detail.unwrap_or_else(|| "This release has no installer for your platform.".into()),
        )
    })?;
    let emitter = app.clone();
    let outcome = updater::download_and_install(&asset, move |p| {
        if let Err(e) = emitter.emit_to("main", UPDATE_PROGRESS_EVENT, &p) {
            log::warn!("failed to emit update progress: {e}");
        }
    })
    .await?;
    match outcome.action {
        PostInstall::Exit => {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                app.exit(0);
            });
        }
        PostInstall::Restart => {
            // The AppImage file was replaced: launch the new file (the running binary lives in
            // the old image's mount), then quit. Fall back to Tauri's restart if that fails.
            let app = app.clone();
            let target = std::path::PathBuf::from(&outcome.path);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(800));
                let mut cmd = std::process::Command::new(&target);
                // Drop the old image's runtime variables so they can't point into its mount.
                let appdir = std::env::var("APPDIR").unwrap_or_default();
                for var in ["APPDIR", "APPIMAGE", "ARGV0", "OWD"] {
                    cmd.env_remove(var);
                }
                for var in ["LD_LIBRARY_PATH", "PATH", "XDG_DATA_DIRS", "GSETTINGS_SCHEMA_DIR", "GDK_PIXBUF_MODULE_FILE", "GIO_MODULE_DIR", "GTK_PATH", "PYTHONHOME"] {
                    if let Ok(v) = std::env::var(var) {
                        if !appdir.is_empty() && v.contains(&appdir) {
                            let kept: Vec<&str> = v.split(':').filter(|p| !p.starts_with(&appdir)).collect();
                            if kept.is_empty() { cmd.env_remove(var); } else { cmd.env(var, kept.join(":")); }
                        }
                    }
                }
                match cmd.spawn() {
                    Ok(_) => app.exit(0),
                    Err(e) => {
                        log::warn!("could not relaunch {}: {e}", target.display());
                        app.restart();
                    }
                }
            });
        }
        PostInstall::Nothing => {}
    }
    Ok(outcome.message)
}
