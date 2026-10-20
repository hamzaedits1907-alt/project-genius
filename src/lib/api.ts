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

/**
 * Called after a provider key was saved or removed on this device, so the account key vault (when unlocked) can
 * keep its encrypted copy in step. The key is only handed to the vault code, which encrypts it before it leaves.
 */
export const keyHooks: { saved?: (provider: T.ProviderId, key: string) => void; removed?: (provider: T.ProviderId) => void } = {};

export const api = {
  // settings & AI
  getSettings: () => call<T.Settings>('get_settings'),
  saveSettings: (settings: T.Settings) => call<T.Settings>('save_settings', { settings }),
  keyStatus: () => call<T.KeyStatus>('key_status'),
  saveApiKey: async (key: string, remember?: boolean) => {
    const status = await call<T.KeyStatus>('save_api_key', { key, remember: remember ?? false });
    if (keyHooks.saved) void call<T.Settings>('get_settings').then((s) => keyHooks.saved?.(s.provider, key.trim())).catch(() => undefined);
    return status;
  },
  removeApiKey: async () => {
    const provider = keyHooks.removed ? await call<T.Settings>('get_settings').then((s) => s.provider).catch(() => null) : null;
    const status = await call<T.KeyStatus>('remove_api_key');
    if (provider) keyHooks.removed?.(provider);
    return status;
  },
  testConnection: () => call<Record<string, unknown>>('test_connection'),
  diagnoseProvider: (deep: boolean) => call<T.DiagReport>('diagnose_provider', { deep }),
  providerCapabilities: () => call<T.ProviderCapabilities[]>('provider_capabilities'),
  brainOverview: (projectId: string) => call<T.BrainOverview>('brain_overview', { projectId }),
  brainAsk: (projectId: string, query: string, impact: string[]) => call<T.BrainAnswer>('brain_ask', { projectId, query, impact }),
  listModels: (refresh: boolean) => call<T.ModelList>('list_models', { refresh }),
  probeModel: (model: string) => call<T.ProbeResult>('probe_model', { model }),
  selectModel: (model: string) => call<T.Settings>('select_model', { model }),
  /** Models auto mode uses per job (may fetch the model list once). */
  resolvedModels: () => call<T.ResolvedModels>('resolved_models'),
  usageSummary: (projectId: string) => call<T.UsageSummary>('usage_summary', { projectId }),
  detectTools: () => call<T.ToolStatus[]>('detect_tools'),
  /** Rewrites a rough idea (3–4000 characters) into a detailed, buildable brief in the same language. */
  enhancePrompt: (text: string) => call<string>('enhance_prompt', { text }),
  appInfo: () => call<T.AppInfo>('app_info'),

  // updates (GitHub Releases)
  appVersion: () => call<string>('app_version'),
  checkForUpdate: () => call<T.UpdateInfo>('check_for_update'),
  /** Downloads and launches the installer; progress arrives on `genius://update-progress`. */
  installUpdate: () => call<string>('install_update'),

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
  /** Forward runtime errors posted by the preview page; returns the merged current list. */
  reportPreviewErrors: (projectId: string, errors: T.PreviewRuntimeError[]) => call<T.PreviewRuntimeError[]>('report_preview_errors', { projectId, errors }),
  previewErrors: (projectId: string) => call<T.PreviewRuntimeError[]>('preview_errors', { projectId }),
  clearPreviewErrors: (projectId: string) => call<void>('clear_preview_errors', { projectId }),
  reportPreviewInspection: (requestId: string, report: T.InspectionReport | null, error: string | null) => call<void>('report_preview_inspection', { requestId, report, error }),

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

  // conversations & agent
  listConversations: (projectId: string) => call<T.Conversation[]>('list_conversations', { projectId }),
  createConversation: (projectId: string, title?: string) => call<T.Conversation>('create_conversation', { projectId, title: title ?? null }),
  renameConversation: (conversationId: string, title: string) => call<void>('rename_conversation', { conversationId, title }),
  deleteConversation: (conversationId: string) => call<void>('delete_conversation', { conversationId }),
  conversationMessages: (conversationId: string) => call<T.MessageRow[]>('conversation_messages', { conversationId }),
  startAgent: (projectId: string, request: T.AgentRequest) => call<T.TaskInfo>('start_agent', { projectId, request }),
  agentSteps: (taskId: string) => call<T.AgentStep[]>('agent_steps', { taskId }),
  listCommands: () => call<T.SlashCommand[]>('list_commands'),
  /** Queue a message for the running agent; it reads it after its current step (`genius://agent-steered`). */
  steerAgent: (taskId: string, message: string) => call<void>('steer_agent', { taskId, message }),
  /** Remove `messageId` and everything after it; optionally revert the file changes made since. */
  rewindConversation: (conversationId: string, messageId: string, revertFiles: boolean) =>
    call<T.RewindResult>('rewind_conversation', { conversationId, messageId, revertFiles }),
  compactConversation: (conversationId: string) => call<void>('compact_conversation', { conversationId }),
  capabilityReport: (projectId?: string) => call<T.CapabilityReport>('capability_report', { projectId: projectId ?? null }),

  // memory
  listMemories: (projectId: string) => call<T.Memory[]>('list_memories', { projectId }),
  addMemory: (projectId: string, kind: string, content: string) => call<T.Memory>('add_memory', { projectId, kind, content }),
  updateMemory: (memoryId: string, content: string) => call<void>('update_memory', { memoryId, content }),
  deleteMemory: (projectId: string, memoryId: string) => call<void>('delete_memory', { projectId, memoryId }),

  // imported files
  importAttachment: (projectId: string, path: string, scope: 'message' | 'reference', conversationId?: string) =>
    call<T.Attachment>('import_attachment', { projectId, path, scope, conversationId: conversationId ?? null }),
  importAttachmentData: (projectId: string, name: string, dataBase64: string, scope: 'message' | 'reference', conversationId?: string) =>
    call<T.Attachment>('import_attachment_data', { projectId, name, dataBase64, scope, conversationId: conversationId ?? null }),
  listAttachments: (projectId: string) => call<T.Attachment[]>('list_attachments', { projectId }),
  attachmentPreview: (projectId: string, attachmentId: string) => call<{ attachment: T.Attachment; preview: string; imageUrl?: string | null }>('attachment_preview', { projectId, attachmentId }),
  setAttachmentScope: (projectId: string, attachmentId: string, scope: 'message' | 'reference') => call<T.Attachment>('set_attachment_scope', { projectId, attachmentId, scope }),
  deleteAttachment: (projectId: string, attachmentId: string) => call<void>('delete_attachment', { projectId, attachmentId }),
  supportedImportTypes: () => call<string[]>('supported_import_types'),
  importZipProject: (path: string) => call<T.Project>('import_zip_project', { path }),
  probeVision: (model: string) => call<{ ok: boolean; answer: string; error: string | null; at: string }>('probe_vision', { model }),

  // Genius Chat / Research (conversations live in a hidden general workspace)
  chatWorkspace: () => call<T.Project>('chat_workspace'),
  listChatConversations: (mode: T.ChatMode | null, query?: string) => call<T.ChatConversation[]>('list_chat_conversations', { mode, query: query ?? null }),
  createChatConversation: (mode: T.ChatMode, title?: string) => call<T.ChatConversation>('create_chat_conversation', { mode, title: title ?? null }),
  startChat: (conversationId: string, request: T.ChatRequest) => call<T.TaskInfo>('start_chat', { conversationId, request }),
  researchSources: (conversationId: string) => call<T.ResearchSource[]>('research_sources', { conversationId }),
  projectFromConversation: (conversationId: string, name?: string, sourceMessageId?: string) =>
    call<T.Project>('project_from_conversation', { conversationId, name: name ?? null, sourceMessageId: sourceMessageId ?? null }),
  startResearchSpec: (conversationId: string) => call<T.TaskInfo>('start_research_spec', { conversationId }),

  // integrations
  integrationStatuses: () => call<{ id: string; name: string; status: string; meta: Record<string, unknown> | null; spec: Record<string, unknown> }[]>('integration_statuses'),
  supabaseConnect: (token: string) => call<{ connected: boolean; projects: T.SupabaseProject[] }>('supabase_connect', { token }),
  supabaseDisconnect: () => call<{ connected: boolean; note: string }>('supabase_disconnect'),
  supabaseProjects: () => call<T.SupabaseProject[]>('supabase_projects'),
  supabaseLink: (projectId: string, reference: string) => call<Record<string, unknown>>('supabase_link', { projectId, reference }),
  supabaseStatus: (projectId: string) => call<Record<string, unknown>>('supabase_status', { projectId }),
  supabaseConfigure: (projectId: string) => call<{ envWritten: string; variables: string[]; changesetId: string | null; gitignoreAdded: string[] }>('supabase_configure', { projectId }),
  supabaseProposeMigration: (projectId: string, name: string, sql: string) => call<{ changesetId: string; path: string }>('supabase_propose_migration', { projectId, name, sql }),
  supabaseMigrations: (projectId: string) => call<{ path: string; size: number; appliedRemote: string | null }[]>('supabase_migrations', { projectId }),
  supabaseApplyMigration: (projectId: string, path: string) => call<Record<string, unknown>>('supabase_apply_migration', { projectId, path }),
  supabaseGenerateTypes: (projectId: string) => call<{ changesetId: string | null; note?: string }>('supabase_generate_types', { projectId }),

  publish: (projectId: string, request: T.PublishRequest) => call<{ url: string; repo: string; branch: string; verified: boolean }>('publish', { projectId, request }),
  /** Build and deploy the website to GitHub Pages (a task; progress via deployStatus). */
  deployPages: (projectId: string) => call<T.TaskInfo>('deploy_pages', { projectId }),
  deployStatus: (projectId: string) => call<T.DeployStatus | null>('deploy_status', { projectId }),

  // custom agents, teams and missions
  listAgents: () => call<T.CustomAgent[]>('list_agents'),
  getAgent: (agentId: string) => call<T.CustomAgent>('get_agent', { agentId }),
  saveAgent: (agent: T.CustomAgent | T.AgentDraft) => call<T.CustomAgent>('save_agent', { agent }),
  deleteAgent: (agentId: string) => call<void>('delete_agent', { agentId }),
  agentTemplates: () => call<T.AgentTemplate[]>('agent_templates'),
  agentRuntimes: () => call<T.AgentRuntime[]>('agent_runtimes'),
  agentLogs: (agentId: string, limit?: number) => call<T.AgentTaskLog[]>('agent_logs', { agentId, limit: limit ?? null }),
  runAgentTask: (agentId: string, projectId: string, task: string) => call<T.Mission>('run_agent_task', { agentId, projectId, task }),
  listTeams: () => call<T.Team[]>('list_teams'),
  saveTeam: (team: T.Team | Omit<T.Team, 'id'>) => call<T.Team>('save_team', { team }),
  deleteTeam: (teamId: string) => call<void>('delete_team', { teamId }),
  createMission: (projectId: string, goal: string, teamId: string | null) => call<T.Mission>('create_mission', { projectId, goal, teamId }),
  listMissions: (projectId: string | null) => call<T.Mission[]>('list_missions', { projectId }),
  getMission: (missionId: string) => call<T.Mission>('get_mission', { missionId }),
  startMission: (missionId: string) => call<T.Mission>('start_mission', { missionId }),
  pauseMission: (missionId: string) => call<T.Mission>('pause_mission', { missionId }),
  resumeMission: (missionId: string) => call<T.Mission>('resume_mission', { missionId }),
  cancelMission: (missionId: string) => call<T.Mission>('cancel_mission', { missionId }),
  cancelMissionTask: (taskId: string) => call<T.Mission>('cancel_mission_task', { taskId }),
  reprioritizeTask: (taskId: string, priority: number) => call<T.Mission>('reprioritize_task', { taskId, priority }),
  retryMissionTask: (taskId: string) => call<T.Mission>('retry_mission_task', { taskId }),
  approveMissionTask: (taskId: string, approved: boolean) => call<T.Mission>('approve_mission_task', { taskId, approved }),
};
