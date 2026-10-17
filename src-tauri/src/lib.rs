mod commands;

use genius_core::app::Genius;
use genius_core::events::Emitter;
use genius_core::secrets::{KeyringStore, MemoryStore, SecretStore};
use std::sync::Arc;
use tauri::{Emitter as _, Manager, RunEvent};

/// Pushes core events to the main window only (never to other webviews).
struct TauriEmitter(tauri::AppHandle);

impl Emitter for TauriEmitter {
    fn emit(&self, channel: &str, payload: serde_json::Value) {
        if let Err(e) = self.0.emit_to("main", channel, payload) {
            log::warn!("failed to emit {channel}: {e}");
        }
    }
}

/// Use the OS credential store. If it is unavailable (e.g. no Secret Service on a minimal
/// Linux desktop) fall back to memory-only storage and say so in the UI — never plaintext on disk.
struct CheckedSecretStore {
    inner: Box<dyn SecretStore>,
    name: &'static str,
}

impl SecretStore for CheckedSecretStore {
    fn get(&self, n: &str) -> genius_core::Result<Option<String>> {
        self.inner.get(n)
    }
    fn set(&self, n: &str, v: &str) -> genius_core::Result<()> {
        self.inner.set(n, v)
    }
    fn delete(&self, n: &str) -> genius_core::Result<()> {
        self.inner.delete(n)
    }
    fn backend_name(&self) -> &'static str {
        self.name
    }
}

fn secret_store() -> Arc<dyn SecretStore> {
    let keyring = KeyringStore;
    match keyring.get("genius-probe") {
        Ok(_) => Arc::new(CheckedSecretStore { name: keyring.backend_name(), inner: Box::new(keyring) }),
        Err(e) => {
            log::error!("system credential store unavailable: {e}");
            Arc::new(CheckedSecretStore {
                inner: Box::new(MemoryStore::default()),
                name: "memory only — system credential store unavailable; the key will not be kept after closing Genius",
            })
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let emitter = Arc::new(TauriEmitter(app.handle().clone()));
            // Construct inside Tauri's Tokio runtime so Genius captures its handle.
            let genius = tauri::async_runtime::block_on(async { Genius::new(&data_dir, emitter, secret_store()) })
                .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
            app.manage(genius);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::key_status,
            commands::save_api_key,
            commands::remove_api_key,
            commands::test_connection,
            commands::list_models,
            commands::probe_model,
            commands::select_model,
            commands::detect_tools,
            commands::app_info,
            commands::list_projects,
            commands::get_project,
            commands::create_project,
            commands::import_project,
            commands::open_project,
            commands::update_project_meta,
            commands::delete_project,
            commands::update_spec,
            commands::list_messages,
            commands::list_files,
            commands::read_file,
            commands::save_file,
            commands::delete_file,
            commands::search_files,
            commands::start_plan,
            commands::start_generate,
            commands::preview_context,
            commands::start_change,
            commands::start_diagnose,
            commands::cancel_task,
            commands::active_tasks,
            commands::repair_attempts,
            commands::reset_repair_attempts,
            commands::pending_approvals,
            commands::respond_approval,
            commands::list_changesets,
            commands::get_changeset,
            commands::apply_changeset,
            commands::reject_changeset,
            commands::propose_revert,
            commands::propose_restore_to,
            commands::start_operation,
            commands::list_runs,
            commands::active_runs,
            commands::stop_run,
            commands::needs_install,
            commands::start_preview,
            commands::dep_status,
            commands::restart_preview,
            commands::preview_state,
            commands::start_auto_fix,
            commands::start_create_app,
            commands::stop_preview,
            commands::preview_status,
            commands::list_activity,
            commands::export_project,
            commands::readiness,
            commands::git_view,
            commands::git_init,
            commands::git_diff,
            commands::git_stage,
            commands::staged_findings,
            commands::git_commit,
            commands::suggest_commit_message,
            commands::git_log,
            commands::git_commit_detail,
            commands::git_branches,
            commands::git_branch,
            commands::git_set_identity,
            commands::git_set_remote,
            commands::git_fetch,
            commands::git_pull,
            commands::git_push,
            commands::git_merge_abort,
            commands::git_resolve,
            commands::propose_conflict_resolution,
            commands::ensure_gitignore,
            commands::github_status,
            commands::github_login_web,
            commands::github_login_token,
            commands::github_logout,
            commands::github_repos,
            commands::github_orgs,
            commands::github_clone,
            commands::publish_plan,
            commands::publish,
            commands::supabase_generate_types,
            commands::supabase_apply_migration,
            commands::supabase_migrations,
            commands::supabase_propose_migration,
            commands::supabase_configure,
            commands::supabase_status,
            commands::supabase_link,
            commands::supabase_projects,
            commands::supabase_disconnect,
            commands::supabase_connect,
            commands::integration_statuses,
            commands::probe_vision,
            commands::import_zip_project,
            commands::supported_import_types,
            commands::delete_attachment,
            commands::set_attachment_scope,
            commands::attachment_preview,
            commands::list_attachments,
            commands::import_attachment_data,
            commands::import_attachment,
            commands::delete_memory,
            commands::update_memory,
            commands::add_memory,
            commands::list_memories,
            commands::capability_report,
            commands::agent_steps,
            commands::start_agent,
            commands::conversation_messages,
            commands::delete_conversation,
            commands::rename_conversation,
            commands::create_conversation,
            commands::list_conversations,
            commands::recent_ui_errors,
            commands::record_ui_error,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Genius");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            // Never leave dev servers or builds running after Genius closes.
            if let Some(g) = handle.try_state::<Arc<Genius>>() {
                g.procs.kill_all_now();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// Every command registered in the handler must be listed in the ACL manifest (build.rs)
    /// and granted in the main-window capability — otherwise it would be silently denied.
    #[test]
    fn acl_lists_match_handler() {
        let lib = include_str!("lib.rs");
        let build = include_str!("../build.rs");
        let cap = include_str!("../capabilities/main.json");
        let handler: Vec<&str> = lib
            .lines()
            .filter_map(|l| l.trim().strip_prefix("commands::"))
            .map(|l| l.trim_end_matches(','))
            .collect();
        assert!(handler.len() > 50);
        for c in handler {
            assert!(build.contains(&format!("\"{c}\"")), "{c} missing from build.rs manifest");
            assert!(cap.contains(&format!("\"allow-{}\"", c.replace('_', "-"))), "{c} not granted in capabilities/main.json");
        }
    }
}
