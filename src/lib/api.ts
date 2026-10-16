// Typed wrappers for every backend command. The frontend never handles secrets beyond
// passing a newly-entered key/token straight to the backend once.
import { invoke } from '@tauri-apps/api/core';
import type * as T from './types';

export function errorMessage(e: unknown): string {
  if (e && typeof e === 'object' && 'message' in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function errorKind(e: unknown): string {
  if (e && typeof e === 'object' && 'kind' in e) return String((e as { kind: unknown }).kind);
  return 'unknown';
}

const call = <R>(cmd: string, args?: Record<string, unknown>) => invoke<R>(cmd, args);

export const api = {
  // settings & AI
  getSettings: () => call<T.Settings>('get_settings'),
  saveSettings: (settings: T.Settings) => call<T.Settings>('save_settings', { settings }),
  keyStatus: () => call<T.KeyStatus>('key_status'),
  saveApiKey: (key: string) => call<T.KeyStatus>('save_api_key', { key }),
  removeApiKey: () => call<T.KeyStatus>('remove_api_key'),
  testConnection: () => call<Record<string, unknown>>('test_connection'),
  listModels: (refresh: boolean) => call<T.ModelList>('list_models', { refresh }),
  probeModel: (model: string) => call<T.ProbeResult>('probe_model', { model }),
  selectModel: (model: string) => call<T.Settings>('select_model', { model }),
  detectTools: () => call<T.ToolStatus[]>('detect_tools'),
  appInfo: () => call<T.AppInfo>('app_info'),

  // projects
  listProjects: () => call<T.Project[]>('list_projects'),
  getProject: (projectId: string) => call<T.Project>('get_project', { projectId }),
  createProject: (name: string, description: string) => call<T.Project>('create_project', { name, description }),
  importProject: (path: string) => call<T.Project>('import_project', { path }),
  openProject: (projectId: string) => call<T.Project>('open_project', { projectId }),
  updateProjectMeta: (projectId: string, name: string, description: string, notes: string) =>
    call<T.Project>('update_project_meta', { projectId, name, description, notes }),
  deleteProject: (projectId: string, deleteFiles: boolean) => call<{ filesDeleted: boolean }>('delete_project', { projectId, deleteFiles }),
  updateSpec: (projectId: string, spec: T.ProjectSpec) => call<T.Project>('update_spec', { projectId, spec }),
  listMessages: (projectId: string) => call<T.MessageRow[]>('list_messages', { projectId }),

  // files
  listFiles: (projectId: string) => call<T.FileEntry[]>('list_files', { projectId }),
  readFile: (projectId: string, path: string) => call<string>('read_file', { projectId, path }),
  saveFile: (projectId: string, path: string, content: string) => call<T.Changeset | null>('save_file', { projectId, path, content }),
  deleteFile: (projectId: string, path: string) => call<T.Changeset>('delete_file', { projectId, path }),
  searchFiles: (projectId: string, query: string) => call<T.SearchHit[]>('search_files', { projectId, query }),

  // AI tasks
  startPlan: (projectId: string, description: string, constraints: string) => call<T.TaskInfo>('start_plan', { projectId, description, constraints }),
  startGenerate: (projectId: string) => call<T.TaskInfo>('start_generate', { projectId }),
  previewContext: (projectId: string, request: string, pinned: string[], excluded: string[]) =>
    call<T.ContextPlan>('preview_context', { projectId, request, pinned, excluded }),
  startChange: (projectId: string, request: string, pinned: string[], excluded: string[]) =>
    call<T.TaskInfo>('start_change', { projectId, request, pinned, excluded }),
  startDiagnose: (projectId: string, runId: string | null) => call<T.TaskInfo>('start_diagnose', { projectId, runId }),
  cancelTask: (taskId: string) => call<void>('cancel_task', { taskId }),
  activeTasks: (projectId?: string) => call<T.TaskInfo[]>('active_tasks', { projectId: projectId ?? null }),
  repairAttempts: (projectId: string) => call<string[]>('repair_attempts', { projectId }),
  resetRepairAttempts: (projectId: string) => call<void>('reset_repair_attempts', { projectId }),

  // approvals
  pendingApprovals: (projectId?: string) => call<T.ApprovalRequest[]>('pending_approvals', { projectId: projectId ?? null }),
  respondApproval: (approvalId: string, approved: boolean) => call<void>('respond_approval', { approvalId, approved }),

  // changesets
  listChangesets: (projectId: string) => call<T.Changeset[]>('list_changesets', { projectId }),
  getChangeset: (changesetId: string) => call<{ changeset: T.Changeset; files: T.FileChange[] }>('get_changeset', { changesetId }),
  applyChangeset: (changesetId: string) => call<T.Changeset>('apply_changeset', { changesetId }),
  rejectChangeset: (changesetId: string) => call<T.Changeset>('reject_changeset', { changesetId }),
  proposeRevert: (changesetId: string, force: boolean) => call<T.Changeset>('propose_revert', { changesetId, force }),
  proposeRestoreTo: (changesetId: string) => call<T.Changeset>('propose_restore_to', { changesetId }),

  // execution & preview
  startOperation: (projectId: string, op: T.Operation) => call<T.TaskInfo>('start_operation', { projectId, op }),
  listRuns: (projectId: string) => call<T.RunRow[]>('list_runs', { projectId }),
  activeRuns: (projectId?: string) => call<T.RunInfo[]>('active_runs', { projectId: projectId ?? null }),
  stopRun: (runId: string) => call<void>('stop_run', { runId }),
  needsInstall: (projectId: string) => call<boolean>('needs_install', { projectId }),
  startPreview: (projectId: string) => call<string>('start_preview', { projectId }),
  stopPreview: (projectId: string) => call<void>('stop_preview', { projectId }),
  previewStatus: (projectId: string) => call<string | null>('preview_status', { projectId }),

  // activity, export
  listActivity: (projectId: string | null, limit = 1500) => call<T.ActivityEvent[]>('list_activity', { projectId, limit }),
  exportProject: (projectId: string, destination: string, zip: boolean) => call<T.ExportResult>('export_project', { projectId, destination, zip }),
  readiness: (projectId: string) => call<T.ReadinessCheck[]>('readiness', { projectId }),

  // git
  gitView: (projectId: string) => call<T.GitView>('git_view', { projectId }),
  gitInit: (projectId: string) => call<{ gitignoreAdded: string[] }>('git_init', { projectId }),
  gitDiff: (projectId: string, path: string, staged: boolean, untracked: boolean) => call<string>('git_diff', { projectId, path, staged, untracked }),
  gitStage: (projectId: string, paths: string[], stage: boolean) => call<T.RepoStatus>('git_stage', { projectId, paths, stage }),
  stagedFindings: (projectId: string) => call<T.SecretFinding[]>('staged_findings', { projectId }),
  gitCommit: (projectId: string, message: string, allowFindings: boolean) => call<T.Commit>('git_commit', { projectId, message, allowFindings }),
  suggestCommitMessage: (projectId: string) => call<string>('suggest_commit_message', { projectId }),
  gitLog: (projectId: string, limit = 100) => call<T.Commit[]>('git_log', { projectId, limit }),
  gitCommitDetail: (projectId: string, sha: string) => call<{ files: [string, string][]; diff: string }>('git_commit_detail', { projectId, sha }),
  gitBranches: (projectId: string) => call<T.Branch[]>('git_branches', { projectId }),
  gitBranch: (projectId: string, name: string, create: boolean) => call<void>('git_branch', { projectId, name, create }),
  gitSetIdentity: (projectId: string, name: string, email: string) => call<void>('git_set_identity', { projectId, name, email }),
  gitSetRemote: (projectId: string, name: string, url: string) => call<void>('git_set_remote', { projectId, name, url }),
  gitFetch: (projectId: string) => call<T.RepoStatus>('git_fetch', { projectId }),
  gitPull: (projectId: string, allowMerge: boolean) => call<T.RepoStatus>('git_pull', { projectId, allowMerge }),
  gitPush: (projectId: string) => call<{ verified: boolean; head: string }>('git_push', { projectId }),
  gitMergeAbort: (projectId: string) => call<void>('git_merge_abort', { projectId }),
  gitResolve: (projectId: string, path: string, side: 'ours' | 'theirs') => call<void>('git_resolve', { projectId, path, side }),
  proposeConflictResolution: (projectId: string, path: string) => call<T.Changeset>('propose_conflict_resolution', { projectId, path }),
  ensureGitignore: (projectId: string) => call<string[]>('ensure_gitignore', { projectId }),

  // GitHub
  githubStatus: () => call<T.AuthStatus>('github_status'),
  githubLoginWeb: () => call<void>('github_login_web'),
  githubLoginToken: (token: string) => call<T.AuthStatus>('github_login_token', { token }),
  githubLogout: () => call<T.AuthStatus>('github_logout'),
  githubRepos: (owner?: string) => call<T.RepoInfo[]>('github_repos', { owner: owner ?? null }),
  githubOrgs: () => call<string[]>('github_orgs'),
  githubClone: (nameWithOwner: string) => call<T.Project>('github_clone', { nameWithOwner }),
  publishPlan: (projectId: string) => call<T.PublishPlan>('publish_plan', { projectId }),
  // diagnostics
  recordUiError: (report: { view: string; message: string; stack: string; componentStack: string }) => call<string>('record_ui_error', { report }),
  recentUiErrors: (limit?: number) => call<Record<string, string>[]>('recent_ui_errors', { limit: limit ?? null }),

  publish: (projectId: string, request: T.PublishRequest) => call<{ url: string; repo: string; branch: string; verified: boolean }>('publish', { projectId, request }),
};
