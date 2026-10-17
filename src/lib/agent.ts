// Pure helpers for presenting agent activity. Labels describe recorded operations only.
import type { AgentReport, AgentStep } from './types';

export function stepLabel(s: AgentStep): string {
  const a = s.args as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === 'string' ? (a[k] as string) : '');
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
    delegate: `Asked the ${str('role')} subagent`,
    web_search: `Searched the web for “${str('query')}”`,
    fetch_url: `Read ${hostOf(str('url'))}`,
    write_files: 'Wrote files',
    todo: 'Updated the plan',
  };
  const label = base[s.tool] ?? `Called ${s.tool}`;
  return s.ok ? `${label} — ${s.summary}` : `${label} — ${s.summary || 'failed'}`;
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
};

export function reportSummary(r: AgentReport): string {
  const parts = [`${r.steps} step${r.steps === 1 ? '' : 's'}`, `${r.filesRead.length} file${r.filesRead.length === 1 ? '' : 's'} read`];
  if (r.checks.length) parts.push(`${r.checks.filter((c) => c.ok).length}/${r.checks.length} checks passed`);
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
