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
  uiMode: 'simple' | 'advanced';
  autoCheckUpdates: boolean;
  internetAccess: boolean;
  assistantName: string;
  /** 'auto' lets Genius pick the model per task; 'manual' always uses `model`. */
  modelMode: 'auto' | 'manual';
  effort: Effort;
  /** Ask reasoning models to stream their thinking. */
  thinking: boolean;
}

/** How hard Genius works on a request (speed vs depth). */
export type Effort = 'fast' | 'balanced' | 'max';

/** A chat slash command. `prompt` commands are expanded by the backend; `action` ones run in the UI. */
export interface SlashCommand {
  name: string;
  description: string;
  args?: string;
  kind: 'prompt' | 'action';
}

export interface RewindResult {
  removedMessages: number;
  revertedChangesets: number;
}

/** Model reasoning streamed while the agent works (never persisted). */
export interface AiThinking {
  taskId: string;
  projectId: string;
  text: string;
}

/** The running agent picked up a message the user queued while it worked. */
export interface AgentSteered {
  taskId: string;
  projectId: string;
  message: string;
}

/** Token usage recorded on an assistant message's report. */
export interface UsageReport {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  calls: number;
  models: string[];
}

export interface UpdateAsset {
  name: string;
  size: number;
  url: string;
  sha256Url: string | null;
}

export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string | null;
  available: boolean;
  notes: string;
  publishedAt: string | null;
  releaseUrl: string | null;
  asset: UpdateAsset | null;
  /** Human-readable status, e.g. "No releases have been published yet". */
  detail: string | null;
}

/** Payload of the `genius://update-progress` event. */
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
  stage: 'downloading' | 'verifying' | 'installing' | 'done';
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
  deploy?: DeployStatus;
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
  conversationId?: string | null;
  /** chat | answer | plan | proposal | error */
  kind?: string;
  meta?: Record<string, unknown>;
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

/** A runtime problem seen in the live preview (reported by the injected preview script). */
export interface PreviewRuntimeError {
  kind: 'error' | 'rejection' | 'console' | 'resource' | 'blank' | 'vite';
  message: string;
  /** File/URL, or "console.error" / "console.warn" for console calls. */
  source?: string | null;
  line?: number | null;
  column?: number | null;
  stack?: string | null;
  page: string;
  at: string;
  count: number;
}

/** GitHub Pages deployment of a project (persisted in project status as `deploy`). */
export interface DeployStatus {
  step: 'building' | 'uploading' | 'enabling' | 'deployed' | 'failed';
  repo: string | null;
  /** Live site of the last successful deploy. */
  url: string | null;
  head: string | null;
  at: string;
  deployedAt: string | null;
  error: string | null;
  running: boolean;
}

// ---- conversations & agent
export interface Conversation {
  id: string;
  projectId: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  messageCount: number;
}

export type AgentMode = 'build' | 'plan';

export interface AgentRequest {
  conversationId: string;
  message: string;
  mode: AgentMode;
  attachmentIds: string[];
  pinned: string[];
  effort?: Effort | null;
  /** Apply, type-check/build and fix on its own before answering. */
  autopilot?: boolean | null;
}

export interface AgentStep {
  id: string;
  projectId: string;
  taskId: string;
  conversationId: string | null;
  agent: 'main' | 'architect' | 'reviewer' | 'ui' | 'designer' | 'tester' | 'explorer' | 'debugger';
  seq: number;
  /** Model turn that issued this step (several tool calls can share a turn). */
  turn?: number | null;
  tool: string;
  args: Record<string, unknown>;
  ok: boolean;
  summary: string;
  result: Record<string, unknown>;
  startedAt: string;
  endedAt: string;
}

export interface AgentReport {
  steps: number;
  filesRead: string[];
  searches: number;
  checks: { check: string; ok: boolean; status: string; exitCode: number | null; summary: string }[];
  subagents: { role: string; ok: boolean }[];
  failedSteps: string[];
  changeset: { id: string; files: FileChangeStat[]; status: string } | null;
  verification: Record<string, unknown> | null;
  notVerified: string[];
  warnings: string[];
  error: string | null;
  usage?: UsageReport | null;
  model?: string | null;
  durationMs?: number | null;
  autopilot?: { applied: string[]; checks: { check: string; ok: boolean }[] } | null;
  /** Changeset ids recorded as the checkpoint for this turn. */
  checkpoint?: string[] | null;
}

export interface Capability {
  id: string;
  label: string;
  status: 'available' | 'unavailable' | 'unverified';
  evidence: string;
  remedy: string;
}

export interface CapabilityReport {
  generatedAt: string;
  provider: { name: string; baseUrl: string; model: string | null; maxTokens: number; keyStored: boolean };
  capabilities: Capability[];
  agentTools: { name: string; description: string; readOnly: boolean }[];
  integrations: { id: string; name?: string; status: string; detail?: string; login?: string | null; meta?: Record<string, unknown> }[];
  permissions: Record<string, string>;
  project: Record<string, unknown> | null;
  limitations: string[];
}

export interface Memory {
  id: string;
  projectId: string;
  kind: string;
  content: string;
  source: 'user' | 'repo' | 'agent';
  evidence: string;
  createdAt: string;
  updatedAt: string;
}

export interface Attachment {
  id: string;
  projectId: string;
  conversationId: string | null;
  name: string;
  kind: string;
  mime: string;
  size: number;
  scope: 'message' | 'reference';
  status: 'ready' | 'failed';
  error: string | null;
  chars: number;
  meta: Record<string, unknown>;
  createdAt: string;
}

export interface SupabaseProject {
  reference: string;
  name: string;
  region: string;
  status: string;
  organizationId: string;
}
