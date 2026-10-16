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

const gh: T.AuthStatus = { ghInstalled: false, ghVersion: null, connected: false, login: null, scopes: [], detail: 'gh not installed' };

const status: T.RepoStatus = { isRepo: false, branch: null, head: null, upstream: null, ahead: 0, behind: 0, files: [], merging: false, remotes: [] };

export type Handler = (cmd: string, args: Record<string, unknown>) => unknown;

export const defaults: Record<string, Handler> = {
  get_settings: () => settings,
  save_settings: (_c, a) => a.settings,
  key_status: () => ({ stored: false, hint: null, backend: 'memory', error: null }),
  app_info: () => ({ version: '0.1.0', dataDir: '/tmp/d', workspaceDir: '/tmp/ws', secretBackend: 'memory', platform: 'linux' }),
  list_models: () => ({ models: [], fetchedAt: null, source: 'cache', stale: true, baseUrl: settings.baseUrl, probes: {} }),
  detect_tools: () => [],
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
  needs_install: () => false,
  repair_attempts: () => [],
  readiness: () => [],
  git_view: () => ({ available: true, error: null, status, geniusPaths: [], identity: null }),
  git_branches: () => [],
  git_log: () => [],
  github_status: () => gh,
  github_repos: () => [],
  github_orgs: () => [],
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
