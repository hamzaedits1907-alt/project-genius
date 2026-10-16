// Types mirroring the Rust core (serde camelCase). Keep in sync with crates/genius-core.

export type Policy = 'ask' | 'auto';

export interface Settings {
  baseUrl: string;
  model: string | null;
  temperature: number;
  maxTokens: number;
  contextBudgetChars: number;
  workspaceDir: string | null;
  approvals: { applyAiEdits: Policy; dependencyInstall: Policy; gitCommit: Policy; gitPush: Policy };
  verifyAfterApply: boolean;
  maxRepairAttempts: number;
  theme: 'dark' | 'light' | 'system';
  onboardingDone: boolean;
}

export interface GeniusError {
  kind: string;
  message: string;
}

export interface KeyStatus {
  stored: boolean;
  hint: string | null;
  backend: string;
  error: string | null;
}

export interface ModelInfo {
  id: string;
  owner: string | null;
  created: number | null;
  inferredCategory: string;
}

export interface ProbeResult {
  model: string;
  chat: boolean;
  streaming: boolean;
  latencyMs: number;
  sample: string;
}

export interface ModelList {
  models: ModelInfo[];
  fetchedAt: string | null;
  source: 'live' | 'cache';
  stale: boolean;
  baseUrl: string;
  probes: Record<string, ProbeResult>;
}

export interface ToolStatus {
  name: string;
  found: boolean;
  path: string | null;
  version: string | null;
  hint: string;
}

export interface ProjectSpec {
  name: string;
  purpose: string;
  features: { name: string; description: string }[];
  pages: { name: string; description: string; components: string[] }[];
  components: { name: string; description: string }[];
  dataModels: { name: string; fields: string[]; relations: string[] }[];
  architecture: string;
  dependencies: { name: string; reason: string }[];
  milestones: string[];
  testStrategy: string;
  risks: string[];
  assumptions: string[];
  files: { path: string; purpose: string }[];
}

export interface RunStatusSummary {
  status: string;
  exitCode: number | null;
  at: string;
  runId: string;
  durationMs: number;
}

export interface ProjectStatus {
  stage?: 'created' | 'imported' | 'planned' | 'generated';
  lastBuild?: RunStatusSummary;
  lastTest?: RunStatusSummary;
  lastInstall?: RunStatusSummary;
  lastTypecheck?: RunStatusSummary;
  publish?: { repo: string; step: string; url?: string; verified?: boolean; at: string; visibility: string };
}

export interface Project {
  id: string;
  name: string;
  description: string;
  path: string;
  kind: string;
  spec: ProjectSpec | null;
  notes: string;
  status: ProjectStatus;
  createdAt: string;
  updatedAt: string;
  lastOpenedAt: string | null;
}

export interface MessageRow {
  id: string;
  projectId: string;
  taskId: string | null;
  role: 'user' | 'assistant';
  content: string;
  createdAt: string;
}

export interface FileEntry {
  path: string;
  size: number;
  isText: boolean;
  sensitive: boolean;
}

export interface SearchHit {
  path: string;
  line: number;
  text: string;
}

export interface TaskInfo {
  id: string;
  projectId: string;
  kind: string;
  title: string;
  startedAt: string;
}

export type EventKind =
  | 'task' | 'ai' | 'plan' | 'file_read' | 'file_write' | 'changeset' | 'command' | 'install'
  | 'build' | 'test' | 'preview' | 'git' | 'github' | 'approval' | 'export' | 'diagnosis';

export type EventStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' | 'awaiting_approval';

export interface ActivityEvent {
  id: string;
  projectId: string | null;
  taskId: string | null;
  kind: EventKind;
  status: EventStatus;
  title: string;
  detail: Record<string, unknown>;
  ts: string;
  updatedTs: string;
}

export interface ApprovalRequest {
  id: string;
  projectId: string | null;
  taskId: string | null;
  category: string;
  title: string;
  detail: Record<string, unknown>;
  eventId: string;
}

export interface FileChangeStat {
  path: string;
  op: 'create' | 'modify' | 'delete';
  added: number;
  removed: number;
}

export interface Changeset {
  id: string;
  projectId: string;
  taskId: string | null;
  kind: 'template' | 'generate' | 'change' | 'repair' | 'manual' | 'restore';
  title: string;
  summary: string;
  status: 'proposed' | 'applied' | 'rejected' | 'reverted';
  createdAt: string;
  appliedAt: string | null;
  files: FileChangeStat[];
}

export interface FileChange {
  path: string;
  op: 'create' | 'modify' | 'delete';
  before: string | null;
  after: string | null;
}

export interface ContextFile {
  path: string;
  chars: number;
  reason: string;
  included: boolean;
}

export interface ContextPlan {
  files: ContextFile[];
  tree: string[];
  budgetChars: number;
  usedChars: number;
  excludedSensitive: string[];
}

export type Operation = 'install' | 'dev' | 'build' | 'test' | 'typecheck';

export interface RunInfo {
  id: string;
  projectId: string;
  op: Operation;
  command: string;
  cwd: string;
  reason: string;
  startedAt: string;
  port: number | null;
}

export interface RunRow {
  id: string;
  projectId: string;
  op: Operation;
  command: string;
  cwd: string;
  reason: string;
  status: string;
  exitCode: number | null;
  outputTail: string;
  startedAt: string;
  endedAt: string | null;
}

export interface RunOutcome {
  id: string;
  op: Operation;
  status: 'succeeded' | 'failed' | 'cancelled' | 'spawn_failed';
  exitCode: number | null;
  durationMs: number;
  outputTail: string;
}

export interface ExportResult {
  destination: string;
  files: number;
  bytes: number;
  skipped: string[];
}

export interface ReadinessCheck {
  id: string;
  label: string;
  status: 'pass' | 'fail' | 'warn' | 'unknown';
  detail: string;
}

export interface GitFileStatus {
  path: string;
  origPath: string | null;
  index: string;
  worktree: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
  conflicted: boolean;
}

export interface RepoStatus {
  isRepo: boolean;
  branch: string | null;
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  files: GitFileStatus[];
  merging: boolean;
  remotes: { name: string; url: string }[];
}

export interface GitView {
  available: boolean;
  error: string | null;
  status: RepoStatus;
  geniusPaths: string[];
  identity: [string, string] | null;
}

export interface Commit {
  sha: string;
  short: string;
  author: string;
  email: string;
  date: string;
  subject: string;
}

export interface Branch {
  name: string;
  current: boolean;
  upstream: string | null;
}

export interface SecretFinding {
  path: string;
  line: number | null;
  kind: string;
}

export interface AuthStatus {
  ghInstalled: boolean;
  ghVersion: string | null;
  connected: boolean;
  login: string | null;
  scopes: string[];
  detail: string;
}

export interface RepoInfo {
  nameWithOwner: string;
  name: string;
  owner: string;
  visibility: string;
  url: string;
  description: string | null;
  updatedAt: string | null;
  defaultBranch: string | null;
  isEmpty: boolean | null;
}

export interface PublishPlan {
  gitAvailable: boolean;
  gh: AuthStatus;
  isRepo: boolean;
  branch: string;
  remoteUrl: string | null;
  suggestedName: string;
  files: string[];
  excludedByGitignore: string[];
  findings: SecretFinding[];
  gitignoreMissing: string[];
  uncommitted: number;
  previous: ProjectStatus['publish'] | null;
}

export interface PublishRequest {
  owner: string;
  name: string;
  visibility: 'private' | 'public' | 'internal';
  description: string;
  commitMessage: string;
  acknowledgeFindings: boolean;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  workspaceDir: string | null;
  secretBackend: string;
  platform: string;
}

// ---- streamed payloads
export interface AiDelta {
  taskId: string;
  projectId: string;
  text: string;
}

export type FileStreamEvent =
  | { type: 'text'; line: string; taskId: string; projectId: string }
  | { type: 'fileStart'; path: string; taskId: string; projectId: string }
  | { type: 'fileDelta'; path: string; text: string; taskId: string; projectId: string }
  | { type: 'fileEnd'; path: string; content: string; complete: boolean; taskId: string; projectId: string }
  | { type: 'delete'; path: string; taskId: string; projectId: string };

export interface ProcessOutput {
  runId: string;
  stream: 'stdout' | 'stderr';
  line: string;
}

export interface ProcessState {
  run: RunInfo;
  state: 'running' | 'succeeded' | 'failed' | 'cancelled';
  outcome?: RunOutcome;
}

export interface PreviewEvent {
  projectId: string;
  status: 'starting' | 'ready' | 'stopped' | 'failed';
  url?: string;
  error?: string;
}
