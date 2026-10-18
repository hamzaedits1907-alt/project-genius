// Pure helpers for custom agents, teams and missions: constants mirrored from the backend,
// validation, status mapping and the task-graph layout. No React, no IPC.
import type {
  AgentApproval,
  AgentBudget,
  AgentDraft,
  AgentRuntime,
  AgentStatus,
  AgentTemplate,
  CapabilityReport,
  CustomAgent,
  Mission,
  MissionStatus,
  MissionTask,
  MissionTaskStatus,
} from './types';
import type { Mood } from '../components/GeniusAvatar';

// ---------------------------------------------------------------------------
// Tool registry (mirrors TOOLS in crates/genius-core/src/agent.rs). When the capability
// report lists the agent tools, that live list wins (see `toolCatalog`).
// ---------------------------------------------------------------------------
export interface ToolInfo {
  name: string;
  /** Plain-language description for non-programmers. */
  description: string;
  readOnly: boolean;
}

export const TOOL_REGISTRY: ToolInfo[] = [
  { name: 'project_facts', readOnly: true, description: 'Get a verified overview of the project: frameworks, scripts, routes and files.' },
  { name: 'list_files', readOnly: true, description: 'See which files exist in the project.' },
  { name: 'search', readOnly: true, description: 'Search the project text for a word or phrase.' },
  { name: 'read_file', readOnly: true, description: 'Open and read project files.' },
  { name: 'last_run_output', readOnly: true, description: 'Read the real output of the latest build, test or dev-server run.' },
  { name: 'inspect_preview', readOnly: true, description: 'Look at the running app: layout, accessibility and rendering problems.' },
  { name: 'preview_errors', readOnly: true, description: 'Read runtime errors from the live preview.' },
  { name: 'git_status', readOnly: true, description: 'See the current branch and changed files.' },
  { name: 'git_diff', readOnly: true, description: 'See the uncommitted changes of a file.' },
  { name: 'recent_changes', readOnly: true, description: 'See recent changes made by Genius or by you.' },
  { name: 'memory', readOnly: true, description: 'Read project memory (decisions and conventions).' },
  { name: 'search_references', readOnly: true, description: 'Search documents you imported as reference material.' },
  { name: 'read_reference', readOnly: true, description: 'Read an imported reference document.' },
  { name: 'use_skill', readOnly: true, description: 'Load an expert playbook for a kind of work.' },
  { name: 'capabilities', readOnly: true, description: 'Ask Genius what it can currently do.' },
  { name: 'web_search', readOnly: true, description: 'Search the web (needs Network access).' },
  { name: 'fetch_url', readOnly: true, description: 'Read a public web page (needs Network access).' },
  { name: 'write_file', readOnly: false, description: 'Create a file or rewrite it completely. Changes go to review.' },
  { name: 'edit_file', readOnly: false, description: 'Change part of an existing file. Changes go to review.' },
  { name: 'write_files', readOnly: false, description: 'Write several files at once. Changes go to review.' },
  { name: 'delete_file', readOnly: false, description: 'Delete a project file. Changes go to review.' },
  { name: 'add_dependency', readOnly: false, description: 'Add an npm package to the project.' },
  { name: 'run_check', readOnly: false, description: 'Run the real type check, build or tests (executes project code).' },
  { name: 'remember', readOnly: false, description: 'Save a note to project memory.' },
  { name: 'todo', readOnly: false, description: 'Show a live progress checklist while working.' },
];

export const WEB_TOOLS = ['web_search', 'fetch_url'];
/** Tools that change project files. An agent that "only plans" must not have them. */
export const EDIT_TOOLS = ['write_file', 'edit_file', 'write_files', 'delete_file', 'add_dependency'];
export const DEFAULT_TOOLS = TOOL_REGISTRY.filter((t) => t.readOnly && !WEB_TOOLS.includes(t.name)).map((t) => t.name);

/** The live tool list from the capability report when it has one; otherwise the mirrored constant. */
export function toolCatalog(report: CapabilityReport | null | undefined): ToolInfo[] {
  const live = report?.agentTools ?? [];
  if (!live.length) return TOOL_REGISTRY;
  const known = new Map(TOOL_REGISTRY.map((t) => [t.name, t]));
  return live.map((t) => ({ name: t.name, readOnly: t.readOnly, description: known.get(t.name)?.description ?? t.description }));
}

// ---------------------------------------------------------------------------
// Defaults and limits
// ---------------------------------------------------------------------------
export const BUDGET_DEFAULTS: AgentBudget = { maxSteps: 30, maxMinutes: 20, maxTokens: 200_000, maxModelCalls: 40 };
/** Hard maxima enforced by the form (the backend clamps again). */
export const BUDGET_MAX: AgentBudget = { maxSteps: 100, maxMinutes: 120, maxTokens: 2_000_000, maxModelCalls: 200 };
export const RETRY_DEFAULT = 1;
export const RETRY_MAX = 5;

export const BUDGET_LABELS: Record<keyof AgentBudget, { label: string; unit: string; hint: string }> = {
  maxSteps: { label: 'Steps', unit: 'steps', hint: 'How many tool actions the agent may take per task.' },
  maxMinutes: { label: 'Time', unit: 'minutes', hint: 'Wall-clock limit per task.' },
  maxTokens: { label: 'Tokens', unit: 'tokens', hint: 'Total AI tokens per task.' },
  maxModelCalls: { label: 'Model calls', unit: 'calls', hint: 'Number of AI requests per task.' },
};

export const AVATAR_PRESETS: { key: string; emoji: string; label: string }[] = [
  { key: 'spark', emoji: '✨', label: 'Spark' },
  { key: 'owl', emoji: '🦉', label: 'Owl' },
  { key: 'robot', emoji: '🤖', label: 'Robot' },
  { key: 'leaf', emoji: '🍃', label: 'Leaf' },
];
export const EMOJI_CHOICES = ['🎨', '🔍', '🛠️', '🧪', '🛡️', '🧭', '📝', '📊', '🚀', '🧠', '🦊', '🐙'];

export function avatarEmoji(avatar: string): string {
  return AVATAR_PRESETS.find((p) => p.key === avatar)?.emoji ?? avatar;
}

export const INSTRUCTION_TEMPLATE = `You are a careful {role}.

## What you do
- Describe the work this agent is responsible for.

## How you work
1. Read the relevant files before changing anything.
2. Make small, focused changes and explain them briefly.
3. Check your work (build or tests) when the tools allow it.

## Rules
- Never guess: if something is unclear, say so and ask.
- Stay inside the files and folders you were given.
- Never touch secrets (.env files, keys, tokens).

## When you are done
Summarise what you changed, what you verified, and what is left.`;

export function blankAgent(now = new Date().toISOString()): AgentDraft {
  return {
    name: '',
    avatar: 'spark',
    role: '',
    description: '',
    instructions: '',
    specialization: [],
    knowledgeSources: [],
    modelPolicy: 'auto',
    model: null,
    tools: [...DEFAULT_TOOLS],
    scope: { projects: 'current', paths: [] },
    network: false,
    memoryPolicy: 'project',
    budget: { ...BUDGET_DEFAULTS },
    retryLimit: RETRY_DEFAULT,
    approval: 'ask',
    acceptanceCriteria: '',
    outputRequirements: '',
    builtin: false,
    createdAt: now,
    updatedAt: now,
  };
}

export function fromTemplate(t: AgentTemplate, now = new Date().toISOString()): AgentDraft {
  return { ...blankAgent(now), ...structuredClone(t.agent), builtin: false, createdAt: now, updatedAt: now };
}

/** A copy of an existing agent to customise (never builtin, no id). */
export function duplicateAgent(a: CustomAgent, now = new Date().toISOString()): AgentDraft {
  const rest: AgentDraft = structuredClone(a);
  delete rest.id;
  return { ...rest, name: `${a.name} (copy)`, builtin: false, createdAt: now, updatedAt: now };
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------
export type AgentErrors = Partial<Record<string, string>>;

/** A project-relative path prefix: no absolute paths, no `..`, no drive letters or URLs. */
export function isSafeRelPath(p: string): boolean {
  const s = p.trim();
  if (!s || s.length > 300) return false;
  if (s.startsWith('/') || s.startsWith('\\') || s.startsWith('~') || /^[a-zA-Z]:/.test(s) || /^[a-z][a-z0-9+.-]*:\/\//i.test(s)) return false;
  if (s.includes('\0')) return false;
  return !s.split(/[\\/]/).some((seg) => seg === '..');
}

export function isHttpsUrl(u: string): boolean {
  try {
    const url = new URL(u.trim());
    return url.protocol === 'https:' && !!url.hostname;
  } catch {
    return false;
  }
}

export function validateAgent(a: AgentDraft): AgentErrors {
  const e: AgentErrors = {};
  if (!a.name.trim()) e.name = 'Give the agent a name.';
  else if (a.name.trim().length > 60) e.name = 'Keep the name under 60 characters.';
  if (!a.avatar.trim()) e.avatar = 'Pick an avatar.';
  if (!a.role.trim()) e.role = 'Describe the role in a few words, e.g. "UI designer".';
  else if (a.role.length > 80) e.role = 'Keep the role under 80 characters.';
  if (a.description.length > 500) e.description = 'Keep the description under 500 characters.';
  if (a.instructions.trim().length < 20) e.instructions = 'Write instructions of at least 20 characters so the agent knows how to work.';
  else if (a.instructions.length > 8000) e.instructions = 'Instructions are limited to 8000 characters.';
  if (a.specialization.length > 12) e.specialization = 'Use at most 12 specializations.';
  for (const s of a.knowledgeSources) {
    if (!(isHttpsUrl(s) || isSafeRelPath(s))) {
      e.knowledgeSources = `"${s}" is not a project-relative path or an https:// link.`;
      break;
    }
  }
  if (a.modelPolicy === 'manual' && !a.model) e.model = 'Choose a model, or switch the policy to Auto.';
  if (a.tools.length === 0) e.tools = 'Select at least one tool.';
  else if (a.approval === 'plan_only' && a.tools.some((t) => EDIT_TOOLS.includes(t))) e.tools = 'A plan-only agent never edits files: remove the editing tools or change the approval mode.';
  else if (!a.network && a.tools.some((t) => WEB_TOOLS.includes(t))) e.tools = 'Web tools need Network access: turn it on or remove web_search / fetch_url.';
  for (const p of a.scope.paths) {
    if (!isSafeRelPath(p)) {
      e.scope = `"${p}" must be a folder inside the project (no absolute paths or "..").`;
      break;
    }
  }
  for (const k of Object.keys(BUDGET_MAX) as (keyof AgentBudget)[]) {
    const v = a.budget[k];
    if (!Number.isInteger(v) || v < 1) e[`budget.${k}`] = `${BUDGET_LABELS[k].label} must be a whole number of at least 1.`;
    else if (v > BUDGET_MAX[k]) e[`budget.${k}`] = `${BUDGET_LABELS[k].label} is limited to ${BUDGET_MAX[k].toLocaleString()} ${BUDGET_LABELS[k].unit}.`;
  }
  if (!Number.isInteger(a.retryLimit) || a.retryLimit < 0 || a.retryLimit > RETRY_MAX) e.retryLimit = `Retries must be between 0 and ${RETRY_MAX}.`;
  if (a.acceptanceCriteria.length > 2000) e.acceptanceCriteria = 'Limited to 2000 characters.';
  if (a.outputRequirements.length > 2000) e.outputRequirements = 'Limited to 2000 characters.';
  return e;
}

/** The payload for `save_agent`: trimmed, de-duplicated, manual model only when the policy is manual. */
export function normalizeAgent(a: AgentDraft): AgentDraft {
  const uniq = (xs: string[]) => Array.from(new Set(xs.map((x) => x.trim()).filter(Boolean)));
  return {
    ...a,
    name: a.name.trim(),
    avatar: a.avatar.trim(),
    role: a.role.trim(),
    description: a.description.trim(),
    instructions: a.instructions.trim(),
    specialization: uniq(a.specialization),
    knowledgeSources: uniq(a.knowledgeSources),
    model: a.modelPolicy === 'manual' ? a.model : null,
    tools: uniq(a.tools),
    scope: { ...a.scope, paths: uniq(a.scope.paths) },
    acceptanceCriteria: a.acceptanceCriteria.trim(),
    outputRequirements: a.outputRequirements.trim(),
    updatedAt: new Date().toISOString(),
  };
}

// ---------------------------------------------------------------------------
// Status mapping
// ---------------------------------------------------------------------------
export const AGENT_STATUS_LABEL: Record<AgentStatus, string> = {
  idle: 'Idle',
  planning: 'Planning',
  working: 'Working',
  blocked: 'Blocked',
  awaiting_approval: 'Awaiting approval',
  reviewing: 'Reviewing',
  completed: 'Completed',
  failed: 'Failed',
  paused: 'Paused',
};

/** Only these statuses animate: everything else is still. */
export function isAnimatedStatus(s: AgentStatus): boolean {
  return s === 'working' || s === 'planning' || s === 'reviewing';
}

export function moodForStatus(s: AgentStatus): Mood {
  switch (s) {
    case 'working':
      return 'working';
    case 'planning':
    case 'reviewing':
      return 'thinking';
    case 'blocked':
    case 'awaiting_approval':
      return 'listening';
    case 'completed':
      return 'happy';
    case 'failed':
      return 'sad';
    default:
      return 'idle';
  }
}

export type Tone = 'ok' | 'bad' | 'warn' | 'run' | 'accent' | 'idle';

export function agentStatusTone(s: AgentStatus): Tone {
  switch (s) {
    case 'working':
    case 'planning':
    case 'reviewing':
      return 'run';
    case 'completed':
      return 'ok';
    case 'failed':
      return 'bad';
    case 'blocked':
    case 'awaiting_approval':
      return 'warn';
    default:
      return 'idle';
  }
}

export const idleRuntime = (agentId: string): AgentRuntime => ({ agentId, status: 'idle', currentTaskId: null, lastActivityAt: null, usage: { tokens: 0, calls: 0, tasksDone: 0, tasksFailed: 0 } });

export const TASK_STATUS_LABEL: Record<MissionTaskStatus, string> = {
  pending: 'Waiting',
  ready: 'Ready',
  running: 'Running',
  blocked: 'Blocked',
  awaiting_approval: 'Needs approval',
  reviewing: 'Reviewing',
  done: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

export function taskTone(s: MissionTaskStatus): Tone {
  switch (s) {
    case 'done':
      return 'ok';
    case 'failed':
      return 'bad';
    case 'blocked':
    case 'awaiting_approval':
      return 'warn';
    case 'running':
    case 'reviewing':
      return 'run';
    case 'ready':
      return 'accent';
    default:
      return 'idle';
  }
}

export const MISSION_STATUS_LABEL: Record<MissionStatus, string> = {
  planning: 'Planning',
  ready: 'Ready to start',
  running: 'Running',
  paused: 'Paused',
  integrating: 'Integrating',
  verifying: 'Verifying',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

export function missionTone(s: MissionStatus): Tone {
  switch (s) {
    case 'completed':
      return 'ok';
    case 'failed':
      return 'bad';
    case 'paused':
      return 'warn';
    case 'running':
    case 'planning':
    case 'integrating':
    case 'verifying':
      return 'run';
    case 'ready':
      return 'accent';
    default:
      return 'idle';
  }
}

export const isTerminalMission = (s: MissionStatus) => s === 'completed' || s === 'failed' || s === 'cancelled';
export const canStartMission = (m: Mission) => m.status === 'ready';
export const canPauseMission = (m: Mission) => m.status === 'running';
export const canResumeMission = (m: Mission) => m.status === 'paused';
export const canCancelMission = (m: Mission) => !isTerminalMission(m.status);

export const canCancelTask = (t: MissionTask) => ['pending', 'ready', 'running', 'blocked', 'awaiting_approval', 'reviewing'].includes(t.status);
export const canRetryTask = (t: MissionTask) => t.status === 'failed' || t.status === 'cancelled' || t.status === 'blocked';
export const canReprioritizeTask = (t: MissionTask) => ['pending', 'ready', 'blocked'].includes(t.status);

export function missionTotals(m: Mission) {
  const by: Partial<Record<MissionTaskStatus, number>> = {};
  let tokens = 0;
  let calls = 0;
  for (const t of m.tasks) {
    by[t.status] = (by[t.status] ?? 0) + 1;
    tokens += t.usage.tokens;
    calls += t.usage.calls;
  }
  const done = by.done ?? 0;
  return { tokens, calls, total: m.tasks.length, done, blocked: by.blocked ?? 0, awaiting: by.awaiting_approval ?? 0, failed: by.failed ?? 0, running: (by.running ?? 0) + (by.reviewing ?? 0), by };
}

/** Tasks (across missions) currently waiting for the user's decision. */
export function pendingApprovalTasks(missions: Mission[]): MissionTask[] {
  return missions.filter((m) => !isTerminalMission(m.status)).flatMap((m) => m.tasks.filter((t) => t.status === 'awaiting_approval'));
}

export function tokenText(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k`;
  return String(n);
}

// ---------------------------------------------------------------------------
// Task graph layout (layered DAG)
// ---------------------------------------------------------------------------
export const NODE_W = 184;
export const NODE_H = 58;
const GAP_X = 64;
const GAP_Y = 18;
const PAD = 16;

export interface GraphNode {
  task: MissionTask;
  x: number;
  y: number;
  layer: number;
}
export interface GraphEdge {
  from: string;
  to: string;
  path: string;
}
export interface TaskGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  width: number;
  height: number;
}

/** Layer = longest dependency chain above the task. Unknown dependencies are ignored; cycles cannot recurse forever. */
export function layoutGraph(tasks: MissionTask[]): TaskGraph {
  const byId = new Map(tasks.map((t) => [t.id, t]));
  const layer = new Map<string, number>();
  const visiting = new Set<string>();
  const depth = (t: MissionTask): number => {
    const known = layer.get(t.id);
    if (known !== undefined) return known;
    if (visiting.has(t.id)) return 0; // cycle: break it
    visiting.add(t.id);
    let d = 0;
    for (const dep of t.dependsOn) {
      const p = byId.get(dep);
      if (p && p.id !== t.id) d = Math.max(d, depth(p) + 1);
    }
    visiting.delete(t.id);
    layer.set(t.id, d);
    return d;
  };
  tasks.forEach(depth);
  const layers: MissionTask[][] = [];
  for (const t of tasks) (layers[layer.get(t.id) ?? 0] ??= []).push(t);
  const nodes: GraphNode[] = [];
  const pos = new Map<string, GraphNode>();
  let maxRows = 0;
  layers.forEach((col, li) => {
    const sorted = [...(col ?? [])].sort((a, b) => b.priority - a.priority || a.title.localeCompare(b.title));
    maxRows = Math.max(maxRows, sorted.length);
    sorted.forEach((task, row) => {
      const n: GraphNode = { task, layer: li, x: PAD + li * (NODE_W + GAP_X), y: PAD + row * (NODE_H + GAP_Y) };
      nodes.push(n);
      pos.set(task.id, n);
    });
  });
  const edges: GraphEdge[] = [];
  for (const t of tasks) {
    for (const dep of t.dependsOn) {
      const a = pos.get(dep);
      const b = pos.get(t.id);
      if (!a || !b || a === b) continue;
      const x1 = a.x + NODE_W;
      const y1 = a.y + NODE_H / 2;
      const x2 = b.x;
      const y2 = b.y + NODE_H / 2;
      const mx = (x1 + x2) / 2;
      edges.push({ from: dep, to: t.id, path: `M${x1},${y1} C${mx},${y1} ${mx},${y2} ${x2},${y2}` });
    }
  }
  return {
    nodes,
    edges,
    width: PAD * 2 + layers.length * NODE_W + Math.max(0, layers.length - 1) * GAP_X,
    height: PAD * 2 + maxRows * NODE_H + Math.max(0, maxRows - 1) * GAP_Y,
  };
}

export const BOARD_COLUMNS: { key: string; label: string; statuses: MissionTaskStatus[] }[] = [
  { key: 'waiting', label: 'Waiting', statuses: ['pending', 'ready'] },
  { key: 'running', label: 'In progress', statuses: ['running', 'reviewing'] },
  { key: 'attention', label: 'Needs you', statuses: ['blocked', 'awaiting_approval'] },
  { key: 'done', label: 'Done', statuses: ['done'] },
  { key: 'closed', label: 'Failed / cancelled', statuses: ['failed', 'cancelled'] },
];

export const APPROVAL_LABEL: Record<AgentApproval, string> = {
  ask: 'Ask me before changes are applied',
  auto: 'Apply changes automatically',
  plan_only: 'Plan only (never edits files)',
};

export function memoryPolicyLabel(p: string): string {
  return p === 'none' ? 'No memory' : p === 'project' ? 'Project memory' : 'Its own memory';
}
