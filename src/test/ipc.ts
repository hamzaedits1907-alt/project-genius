// Test-only Tauri IPC mock: answers every backend command with realistic fixtures so the
// real React tree can be mounted, navigated and unmounted in jsdom.
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import type * as T from '../lib/types';

export const settings: T.Settings = {
  baseUrl: 'https://integrate.api.nvidia.com/v1',
  model: 'meta/llama-3.3-70b-instruct',
  temperature: 0.2,
  maxTokens: 4096,
  contextBudgetChars: 60000,
  workspaceDir: '/tmp/ws',
  approvals: { applyAiEdits: 'ask', dependencyInstall: 'ask', gitCommit: 'ask', gitPush: 'ask' },
  verifyAfterApply: true,
  maxRepairAttempts: 2,
  theme: 'dark',
  onboardingDone: true,
  uiMode: 'advanced',
  autoCheckUpdates: true,
  internetAccess: true,
  assistantName: 'Genius',
  modelMode: 'manual',
  effort: 'max',
  thinking: true,
};

export const project: T.Project = {
  id: 'p1',
  name: 'Dashboard',
  description: 'A dashboard',
  path: '/tmp/ws/dashboard',
  kind: 'vite-react-ts',
  spec: null,
  notes: '',
  status: { stage: 'generated' },
  createdAt: '2026-10-01T00:00:00Z',
  updatedAt: '2026-10-01T00:00:00Z',
  lastOpenedAt: null,
};

export const conversation: T.Conversation = { id: 'c1', projectId: 'p1', title: 'Project conversation', createdAt: '2026-10-01T00:00:00Z', updatedAt: '2026-10-01T00:00:00Z', messageCount: 2 };

export const messages: T.MessageRow[] = [
  { id: 'u1', projectId: 'p1', taskId: 't0', role: 'user', content: 'Add a sidebar', createdAt: '2026-10-01T00:00:00Z', conversationId: 'c1', kind: 'chat', meta: {} },
  {
    id: 'a1', projectId: 'p1', taskId: 't0', role: 'assistant', content: 'Added a Sidebar component.', createdAt: '2026-10-01T00:00:01Z', conversationId: 'c1', kind: 'proposal',
    meta: {
      changesetId: 'cs1',
      report: { steps: 2, filesRead: ['src/App.tsx'], searches: 1, checks: [], subagents: [], failedSteps: [], changeset: null, verification: null, notVerified: ['No build or test has run on these changes yet.'], warnings: ['The explanation mentions testing, but no successful build or test ran in this task.'], error: null },
    },
  },
];

const gh: T.AuthStatus = { ghInstalled: false, ghVersion: null, connected: false, login: null, scopes: [], detail: 'gh not installed' };

const status: T.RepoStatus = { isRepo: false, branch: null, head: null, upstream: null, ahead: 0, behind: 0, files: [], merging: false, remotes: [] };

export type Handler = (cmd: string, args: Record<string, unknown>) => unknown;

export const defaults: Record<string, Handler> = {
  get_settings: () => settings,
  save_settings: (_c, a) => a.settings,
  key_status: () => ({ stored: false, hint: null, backend: 'memory', error: null }),
  app_info: () => ({ version: '0.1.0', dataDir: '/tmp/d', workspaceDir: '/tmp/ws', secretBackend: 'memory', platform: 'linux' }),
  app_version: () => '0.2.0',
  check_for_update: (): T.UpdateInfo => ({ currentVersion: '0.2.0', latestVersion: '0.2.0', available: false, notes: '', publishedAt: null, releaseUrl: null, asset: null, detail: "You're on the latest version (0.2.0)." }),
  install_update: () => 'Genius is already up to date.',
  list_models: () => ({ models: [], fetchedAt: null, source: 'cache', stale: true, baseUrl: settings.baseUrl, probes: {} }),
  detect_tools: () => [],
  enhance_prompt: (_c, a) => `A polished web app for: ${String(a.text).trim()}.\nPages/sections: home, details\nKey features: search, favourites\nData: saved in localStorage (sample data labelled as sample)`,
  list_projects: () => [project],
  get_project: () => project,
  open_project: () => project,
  list_messages: () => [],
  list_files: () => [{ path: 'src/App.tsx', size: 10, isText: true, sensitive: false }],
  read_file: () => 'export default function App() { return null; }\n',
  list_changesets: () => [],
  list_runs: () => [],
  active_runs: () => [],
  active_tasks: () => [],
  pending_approvals: () => [],
  list_activity: () => [],
  preview_status: () => null,
  preview_errors: () => [],
  report_preview_errors: (_c, a) => a.errors,
  clear_preview_errors: () => null,
  deploy_status: () => null,
  deploy_pages: () => ({ id: 't-deploy', projectId: 'p1', kind: 'deploy', title: 'Deploy website to GitHub Pages', startedAt: '2026-10-01T00:00:00Z' }),
  needs_install: () => false,
  repair_attempts: () => [],
  readiness: () => [],
  git_view: () => ({ available: true, error: null, status, geniusPaths: [], identity: null }),
  git_branches: () => [],
  git_log: () => [],
  github_status: () => gh,
  github_repos: () => [],
  github_orgs: () => [],
  list_conversations: () => [conversation],
  create_conversation: (_c, a) => ({ ...conversation, id: 'c-new', title: (a.title as string) ?? 'New conversation', messageCount: 0 }),
  conversation_messages: () => messages,
  start_agent: (_c, a) => ({ id: 't1', projectId: 'p1', kind: 'agent', title: `Genius: ${(a.request as { message: string }).message}`, startedAt: '2026-10-01T00:00:00Z' }),
  agent_steps: () => [],
  list_commands: (): T.SlashCommand[] => [
    { name: 'review', description: 'Review code for bugs and risks', args: '<file>', kind: 'prompt' },
    { name: 'explain', description: 'Explain how something works', args: '<topic>', kind: 'prompt' },
    { name: 'clear', description: 'Start a new conversation', kind: 'action' },
    { name: 'plan', description: 'Switch to Plan-only mode', kind: 'action' },
    { name: 'build', description: 'Switch to Build mode', kind: 'action' },
    { name: 'effort', description: 'Set how hard Genius works', args: '<fast|balanced|max>', kind: 'action' },
    { name: 'autopilot', description: 'Turn Autopilot on or off', args: '<on|off>', kind: 'action' },
    { name: 'undo', description: 'Undo the last turn and its file changes', kind: 'action' },
    { name: 'compact', description: 'Summarize the conversation to free up context', kind: 'action' },
    { name: 'model', description: 'Choose the AI model', kind: 'action' },
    { name: 'help', description: 'Show all commands', kind: 'action' },
  ],
  steer_agent: () => null,
  rewind_conversation: (): T.RewindResult => ({ removedMessages: 2, revertedChangesets: 1 }),
  compact_conversation: () => null,
  cancel_task: () => null,
  list_memories: () => [{ id: 'm1', projectId: 'p1', kind: 'decision', content: 'Use CSS modules', source: 'agent', evidence: 'task x', createdAt: '2026-10-01T00:00:00Z', updatedAt: '2026-10-01T00:00:00Z' }],
  list_attachments: () => [],
  supported_import_types: () => ['md', 'pdf', 'png'],
  integration_statuses: () => [],
  supabase_status: () => ({ connected: false, linked: null, remote: null, envConfigured: false, clientLibrary: false, envGitignored: true, serviceRoleMentions: [], manualSteps: [] }),
  supabase_migrations: () => [],
  get_changeset: () => ({ changeset: { id: 'cs1', projectId: 'p1', taskId: 't0', kind: 'change', title: 'Add a sidebar', summary: '', status: 'proposed', createdAt: '2026-10-01T00:00:00Z', appliedAt: null, files: [{ path: 'src/Sidebar.tsx', op: 'create', added: 3, removed: 0 }] }, files: [] }),
  capability_report: () => ({ generatedAt: '2026-10-01T00:00:00Z', provider: { name: 'NVIDIA', baseUrl: '', model: 'm', maxTokens: 1, keyStored: true }, capabilities: [{ id: 'vision', label: 'Image input (screenshots)', status: 'unverified', evidence: 'image input has not been tested with this model', remedy: 'Test image input' }], agentTools: [], integrations: [], permissions: {}, project: null, limitations: [] }),
  preview_context: () => ({ files: [], tree: [], budgetChars: 1000, usedChars: 0, excludedSensitive: [] }),
};

export const calls: { cmd: string; args: Record<string, unknown> }[] = [];

export function installIpc(overrides: Record<string, Handler> = {}) {
  calls.length = 0;
  const table = { ...defaults, ...overrides };
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args: a });
      const h = table[cmd];
      if (h) return h(cmd, a);
      if (cmd.startsWith('plugin:')) return undefined;
      throw { kind: 'test', message: `unmocked command ${cmd}` };
    },
    { shouldMockEvents: true },
  );
}

export { clearMocks };
