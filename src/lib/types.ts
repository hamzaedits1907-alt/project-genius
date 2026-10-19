// Types mirroring the Rust core (serde camelCase). Keep in sync with crates/genius-core.

export type Policy = 'ask' | 'auto';

export interface Settings {
  baseUrl: string;
  model: string | null;
  /** "auto": Genius picks the best available model per job; "manual": always `model`. */
  modelMode: ModelMode;
  effort: Effort;
  /** Let reasoning models think (reasoning streams on `genius://ai-thinking`). */
  thinking: boolean;
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
  provider: ProviderId;
  limits: TaskLimits;
}

export type ProviderId = 'nvidia' | 'openai' | 'anthropic' | 'gemini' | 'custom';

/** Limits on Genius's own work per task. 0 means "no limit" where noted. They do not control the provider's bill. */
export interface TaskLimits {
  /** 0 = no limit */
  maxModelCalls: number;
  /** 0 = no limit */
  maxToolCalls: number;
  /** 0 = no limit */
  maxMinutes: number;
  /** measured + estimated tokens; 0 = no limit */
  maxTokens: number;
  maxConcurrentTasks: number;
  maxProviderRetries: number;
}

export interface ProviderCapabilities {
  provider: ProviderId;
  label: string;
  wire: string;
  streaming: boolean;
  usageReporting: boolean;
  modelListing: boolean;
  imageInput: string;
  autoRouting: boolean;
  toolProtocol: string;
  /** "live": run against the real API; "contract-tests": adapter tested against a stand-in only. */
  verification: 'live' | 'contract-tests';
  notes: string[];
}

export interface DiagStep {
  id: string;
  title: string;
  status: 'ok' | 'warn' | 'fail' | 'skipped' | 'info';
  detail: string;
  hint: string | null;
  ms: number | null;
}

export interface DiagReport {
  provider: ProviderId;
  label: string;
  integrationVerified: 'live' | 'contract-tests';
  host: string;
  model: string | null;
  ok: boolean;
  verdict: string;
  steps: DiagStep[];
  checkedAt: string;
}

export interface BrainOverview {
  projectType: string[];
  languages: [string, number][];
  entryPoints: string[];
  commands: { purpose: string; command: string; source: string }[];
  modules: { path: string; files: number; lines: number; languages: string[] }[];
  keyFiles: { path: string; importedBy: number; reason: string }[];
  externalDeps: [string, number][];
  files: number;
  lines: number;
  symbols: number;
  unresolvedImports: number;
  tests: number;
  indexedAt: string;
  truncated: boolean;
  provenance: string;
}

export interface BrainAnswer {
  indexedAt: string;
  provenance: string;
  hits: { path: string; kind: string; name: string; line: number; score: number }[];
  excerpts: { path: string; line: number; text: string }[];
  impact: { changed: string[]; unknown: string[]; affected: { path: string; depth: number; via: string }[]; tests: string[]; truncated: boolean; note: string } | null;
  notes: { kind: string; origin: string; updated: string; text: string; stale: string[] }[];
  pastChanges: { title: string; status: string; when: string; files: string[] }[];
  gitHistory: { path: string; short: string; date: string; subject: string }[];
}

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
  /** Characters ÷ 4 guess for requests whose provider reported no usage; never merged into the counts above. */
  estimatedTokens?: number;
  /** measured (all requests reported usage) | estimated (none did) | mixed | none */
  source?: 'measured' | 'estimated' | 'mixed' | 'none';
  toolCalls?: number;
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

export type ModelMode = 'auto' | 'manual';
export type Effort = 'fast' | 'balanced' | 'max';
export type ModelRole = 'agent' | 'code' | 'plan' | 'subagent' | 'quick' | 'vision';

/** Fallback chain (best first) auto mode uses for each job. */
export type ResolvedModels = Record<ModelRole, string[]>;

export interface UsageSummary {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  calls: number;
  /** Total tokens per model. */
  byModel: Record<string, number>;
}

/** Payload of `genius://ai-thinking` (live model reasoning). */
export interface AiThinking {
  taskId: string;
  projectId: string;
  text: string;
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

/** One problem found by the page inspector (see `inspect_page.js`). */
export interface InspectionIssue {
  kind: string;
  message: string;
  selector?: string | null;
}

/** What the page inspector measured at one viewport width (mirrors `inspection.rs`). */
export interface PageReport {
  url: string;
  title: string;
  lang: string | null;
  viewport: { width: number; height: number };
  scrollWidth: number;
  scrollHeight: number;
  rendered: { root: string; visibleElements: number; textLength: number; blank: boolean };
  headings: { level: number; text: string }[];
  landmarks: string[];
  text: string;
  layout: InspectionIssue[];
  a11y: InspectionIssue[];
  console: { errors: number; warnings: number };
  truncated: boolean;
}

export interface ViewportReport extends PageReport {
  label: 'current' | 'mobile' | 'tablet' | 'desktop';
  requestedWidth: number | null;
}

export interface InspectionReport {
  url: string;
  title: string;
  lang: string | null;
  console: { errors: number; warnings: number };
  viewports: ViewportReport[];
  notes: string[];
}

/** Payload of `genius://inspect-request`. */
export interface InspectRequest {
  projectId: string;
  requestId: string;
  url?: string;
  widths: number[];
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
  /** What the evidence supports: failed | verified (changes + a passing real check) | completed_unverified | answered. */
  lifecycle?: 'failed' | 'blocked' | 'verified' | 'completed_unverified' | 'answered';
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
  /** Set when one of Genius's own limits stopped the task. */
  budget?: { stoppedBy: { limit: string; used: number; max: number } | null; profileStop?: string | null } | null;
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

// ---------------------------------------------------------------------------
// Custom agents, teams and missions (contract: agents-contract.md; backend workstream B1)
// ---------------------------------------------------------------------------
export type AgentModelPolicy = 'auto' | 'max' | 'manual';
export type AgentMemoryPolicy = 'none' | 'project' | 'agent';
export type AgentApproval = 'ask' | 'auto' | 'plan_only';

export interface AgentBudget {
  maxSteps: number;
  maxMinutes: number;
  maxTokens: number;
  maxModelCalls: number;
}

export interface CustomAgent {
  id: string;
  name: string;
  /** Emoji or preset key ("spark", "owl", "robot", "leaf"). */
  avatar: string;
  role: string;
  description: string;
  instructions: string;
  specialization: string[];
  /** Project-relative paths or https URLs. */
  knowledgeSources: string[];
  modelPolicy: AgentModelPolicy;
  model: string | null;
  tools: string[];
  scope: { projects: 'current' | 'any'; paths: string[] };
  network: boolean;
  memoryPolicy: AgentMemoryPolicy;
  budget: AgentBudget;
  retryLimit: number;
  approval: AgentApproval;
  acceptanceCriteria: string;
  outputRequirements: string;
  builtin: boolean;
  createdAt: string;
  updatedAt: string;
}

/** A custom agent that has not been saved yet (no id). */
export type AgentDraft = Omit<CustomAgent, 'id'> & { id?: string };

export type AgentStatus = 'idle' | 'planning' | 'working' | 'blocked' | 'awaiting_approval' | 'reviewing' | 'completed' | 'failed' | 'paused';

export interface AgentRuntime {
  agentId: string;
  status: AgentStatus;
  currentTaskId: string | null;
  lastActivityAt: string | null;
  usage: { tokens: number; calls: number; tasksDone: number; tasksFailed: number };
}

export interface AgentTemplate {
  key: string;
  name: string;
  avatar: string;
  role: string;
  description: string;
  agent: Omit<CustomAgent, 'id' | 'createdAt' | 'updatedAt' | 'builtin'>;
}

export interface Team {
  id: string;
  name: string;
  description: string;
  coordinatorAgentId: string;
  agentIds: string[];
  createdAt: string;
}

export type MissionTaskStatus = 'pending' | 'ready' | 'running' | 'blocked' | 'awaiting_approval' | 'reviewing' | 'done' | 'failed' | 'cancelled';
export type VerifyState = 'unknown' | 'passed' | 'failed';

export interface MissionTask {
  id: string;
  missionId: string;
  title: string;
  description: string;
  assigneeAgentId: string | null;
  dependsOn: string[];
  priority: number;
  status: MissionTaskStatus;
  result: string | null;
  blocker: string | null;
  changesetId: string | null;
  /** Isolated copy path or branch. */
  workspace: string | null;
  verification: { build: VerifyState; tests: VerifyState; notes: string };
  startedAt: string | null;
  endedAt: string | null;
  usage: { tokens: number; calls: number };
}

export type MissionStatus = 'planning' | 'ready' | 'running' | 'paused' | 'integrating' | 'verifying' | 'completed' | 'failed' | 'cancelled';

export interface Mission {
  id: string;
  projectId: string;
  title: string;
  goal: string;
  teamId: string | null;
  status: MissionStatus;
  tasks: MissionTask[];
  summary: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface AgentTaskLog {
  id: string;
  agentId: string;
  missionId: string | null;
  taskId: string | null;
  projectId: string;
  kind: 'message' | 'tool' | 'status' | 'result' | 'error';
  text: string;
  data: unknown;
  at: string;
}

// ---- Genius Chat / Research
export type ChatMode = 'chat' | 'research';

export interface ChatConversation extends Conversation {
  mode: ChatMode;
  snippet: string;
  /** After "Start a project": `{ projectId, projectName, conversationId }`. */
  origin: { projectId?: string; projectName?: string; conversationId?: string } | null;
}

export interface ChatRequest {
  message: string;
  attachmentIds: string[];
  effort?: Effort;
}

export interface ResearchSource {
  id: string;
  conversationId: string;
  taskId: string | null;
  n: number;
  url: string;
  requestedUrl: string;
  title: string;
  status: number;
  fetchedAt: string;
  contentHash: string;
  snippet: string;
  usedFor: string;
  official: boolean;
  flags: string[];
}

export interface CitationValidation {
  ok: boolean;
  facts: number;
  assumptions: number;
  uncertain: number;
  uncited: string[];
  unknownCitations: number[];
  unfetchedUrls: string[];
  cited: number[];
  repaired: boolean;
}
