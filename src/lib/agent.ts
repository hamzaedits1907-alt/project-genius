// Pure helpers for presenting agent activity. Labels describe recorded operations only.
import type { AgentReport, AgentStep } from './types';

const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);

/** "explorer" → "Explorer". */
export function roleName(role: string): string {
  const r = AGENTS[role]?.label ?? role;
  return r ? r.charAt(0).toUpperCase() + r.slice(1) : 'helper';
}

function clip(s: string, n: number): string {
  const t = s.replace(/\s+/g, ' ').trim();
  return t.length > n ? `${t.slice(0, n - 1)}…` : t;
}

/** A short, human label for what a step did (from its recorded tool call). */
export function stepTitle(s: AgentStep): string {
  const a = (s.args ?? {}) as Record<string, unknown>;
  const r = (s.result ?? {}) as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === 'string' ? (a[k] as string) : '');
  const files = Array.isArray(a.files) ? a.files : Array.isArray(r.paths) ? r.paths : null;
  const added = num(r.added) ?? num(a.added);
  const removed = num(r.removed) ?? num(a.removed);
  const counts = added != null || removed != null ? ` (+${added ?? 0} −${removed ?? 0})` : '';
  const task = str('task') || str('request') || str('question');
  const pkg = str('name') || str('package');
  const base: Record<string, string> = {
    project_facts: 'Inspected project structure',
    list_files: `Listed files${str('prefix') ? ` in ${str('prefix')}` : ''}`,
    search: `Searched “${str('query')}”`,
    read_file: `Read ${str('path')}`,
    git_status: 'Checked Git status',
    git_diff: `Git diff of ${str('path')}`,
    recent_changes: 'Reviewed recent changes',
    memory: 'Read project memory',
    search_references: `Searched imported files for “${str('query')}”`,
    read_reference: 'Read an imported file',
    capabilities: 'Checked its live capabilities',
    last_run_output: `Read last ${str('check') || 'run'} output`,
    run_check: `Ran ${str('check')}`,
    remember: 'Saved a project memory',
    delegate: `Asked the ${roleName(str('role'))} agent${task ? `: ${clip(task, 90)}` : '…'}`,
    inspect_preview: 'Looked at the running app',
    web_search: `Searched the web for “${str('query')}”`,
    fetch_url: `Read ${hostOf(str('url'))}`,
    write_file: `Wrote ${str('path')}${counts}`,
    edit_file: `Edited ${str('path')}${counts}`,
    write_files: files ? `Wrote ${files.length} file${files.length === 1 ? '' : 's'}` : 'Wrote files',
    preview_errors: 'Checked the preview for errors',
    add_dependency: pkg ? `Added the ${pkg} package` : 'Added a package',
    todo: 'Updated the plan',
  };
  return base[s.tool] ?? `Used ${s.tool.replace(/_/g, ' ')}`;
}

/** Title plus the recorded summary, without saying the same thing twice. */
export function stepLabel(s: AgentStep): string {
  const label = stepTitle(s);
  const summary = (s.summary ?? '').trim();
  if (!s.ok) return `${label} — ${summary || 'failed'}`;
  if (!summary || summary === label) return label;
  if (summary.startsWith(label)) return summary;
  return `${label} — ${summary}`;
}

/** The model turn a step belongs to, when the backend recorded one. */
export function stepTurn(s: AgentStep): number | null {
  return num(s.turn) ?? num((s.result as Record<string, unknown> | undefined)?.turn) ?? num((s.args as Record<string, unknown> | undefined)?.turn) ?? null;
}

function stepTarget(s: AgentStep): string {
  const a = (s.args ?? {}) as Record<string, unknown>;
  for (const k of ['path', 'query', 'url', 'check', 'role', 'name']) if (typeof a[k] === 'string') return `${k}:${a[k] as string}`;
  return JSON.stringify(a);
}

export interface StepView {
  step: AgentStep;
  /** Earlier failed attempts of the same operation that this step superseded. */
  retries: number;
}

export interface StepGroup {
  turn: number | null;
  items: StepView[];
}

/**
 * Steps as shown in the run card: failed attempts that a later identical call fixed are folded
 * into it, and consecutive steps from one model turn are grouped together.
 */
export function groupSteps(steps: AgentStep[] | undefined): StepGroup[] {
  const visible = (steps ?? []).filter((s) => s.tool && s.tool !== 'todo');
  const retries = new Map<string, number>();
  const hidden = new Set<string>();
  visible.forEach((s, i) => {
    if (s.ok) return;
    const fix = visible.slice(i + 1).find((x) => x.ok && x.tool === s.tool && x.agent === s.agent && stepTarget(x) === stepTarget(s));
    if (fix) {
      hidden.add(s.id);
      retries.set(fix.id, (retries.get(fix.id) ?? 0) + 1);
    }
  });
  const groups: StepGroup[] = [];
  for (const step of visible) {
    if (hidden.has(step.id)) continue;
    const turn = stepTurn(step);
    const last = groups[groups.length - 1];
    const item = { step, retries: retries.get(step.id) ?? 0 };
    if (last && turn != null && last.turn === turn && last.items[0].step.agent === step.agent) last.items.push(item);
    else groups.push({ turn, items: [item] });
  }
  return groups;
}

/** Non-empty, trimmed strings only (reports must never render empty bullets). */
export function texts(v: unknown): string[] {
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x.trim().length > 0).map((x) => x.trim()) : [];
}

/** 12345 → "12.3k". */
export function compactNumber(n: number): string {
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 100_000 ? 1 : 0).replace(/\.0$/, '')}k`;
  return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
}

/** 12_400 → "12s", 95_000 → "1m 35s". */
export function duration(ms: number): string {
  const s = Math.max(1, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  return s % 60 ? `${m}m ${s % 60}s` : `${m}m`;
}

/** The subtle line under a finished answer: model · tokens · duration · autopilot. */
export function reportFooter(r: AgentReport | undefined, fallbackMs?: number | null): string[] {
  if (!r) return [];
  const parts: string[] = [];
  const model = r.model || texts(r.usage?.models)[0];
  if (model) parts.push(model.split('/').pop() || model);
  const tokens = r.usage?.totalTokens;
  if (typeof tokens === 'number' && tokens > 0) parts.push(`${compactNumber(tokens)} tokens`);
  const ms = typeof r.durationMs === 'number' && r.durationMs > 0 ? r.durationMs : fallbackMs;
  if (ms && ms > 0) parts.push(duration(ms));
  const ap = r.autopilot;
  if (ap) {
    const n = texts(ap.applied).length;
    const checks = (Array.isArray(ap.checks) ? ap.checks : []).filter((c) => c && typeof c.check === 'string' && c.check);
    const what = n ? `Autopilot applied ${n} file${n === 1 ? '' : 's'}` : 'Autopilot: no file changes';
    parts.push([what, ...checks.map((c) => `${c.check} ${c.ok ? '✓' : '✗'}`)].join(', '));
  }
  return parts;
}

function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url || 'a web page';
  }
}

export interface TodoItem {
  text: string;
  status: 'pending' | 'active' | 'done';
}

/** The agent's latest checklist (from its most recent `todo` tool call), if any. */
export function latestTodo(steps: AgentStep[] | undefined): TodoItem[] {
  const last = [...(steps ?? [])].reverse().find((s) => s.tool === 'todo' && s.ok);
  const items = (last?.args as { items?: unknown } | undefined)?.items;
  if (!Array.isArray(items)) return [];
  return items
    .filter((x): x is { text: string; status?: string } => !!x && typeof (x as { text?: unknown }).text === 'string')
    .map((x) => ({ text: x.text, status: x.status === 'done' || x.status === 'active' ? x.status : 'pending' }));
}

/** What the agent said it is doing right now (narration recorded with its latest step). */
export function latestNote(steps: AgentStep[] | undefined): string | null {
  const last = [...(steps ?? [])].reverse().find((s) => typeof (s.result as { note?: unknown })?.note === 'string' && (s.result as { note: string }).note.trim());
  return last ? (last.result as { note: string }).note.trim() : null;
}

/** A question Genius asked with suggested answers (stored on the message by the backend). */
export interface Ask {
  question: string;
  options: string[];
  multi: boolean;
}

export function askOf(meta: Record<string, unknown> | null | undefined): Ask | null {
  const a = meta?.ask as Partial<Ask> | undefined;
  if (!a || typeof a.question !== 'string' || !Array.isArray(a.options)) return null;
  return { question: a.question, options: a.options.filter((o): o is string => typeof o === 'string'), multi: !!a.multi };
}

export const AGENTS: Record<string, { label: string; hint: string }> = {
  main: { label: 'Genius', hint: 'The main agent: talks with you, inspects and changes the code.' },
  architect: { label: 'architect', hint: 'Plans structure, files and order of work (read-only).' },
  designer: { label: 'designer', hint: 'Creates the visual direction: palette, type, layout (read-only).' },
  ui: { label: 'ui', hint: 'Reviews responsiveness and accessibility (read-only).' },
  reviewer: { label: 'reviewer', hint: 'Checks proposed changes for bugs and risks (read-only).' },
  tester: { label: 'tester', hint: 'Looks at tests and suggests what to cover (read-only).' },
  explorer: { label: 'explorer', hint: 'Explores the codebase to find where things live (read-only).' },
  debugger: { label: 'debugger', hint: 'Investigates errors and finds their root cause (read-only).' },
};

export function reportSummary(r: AgentReport): string {
  const read = texts(r.filesRead).length;
  const checks = Array.isArray(r.checks) ? r.checks : [];
  const parts = [`${r.steps} step${r.steps === 1 ? '' : 's'}`, `${read} file${read === 1 ? '' : 's'} read`];
  if (checks.length) parts.push(`${checks.filter((c) => c.ok).length}/${checks.length} checks passed`);
  else parts.push('no checks run');
  return parts.join(' · ');
}

export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error('could not read file'));
    reader.onload = () => {
      const s = String(reader.result);
      resolve(s.slice(s.indexOf(',') + 1));
    };
    reader.readAsDataURL(file);
  });
}
