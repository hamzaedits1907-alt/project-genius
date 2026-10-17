// Slash commands, @-mentions and composer preferences (pure helpers, no React).
import type { Effort, FileEntry, SlashCommand } from '../../../lib/types';

/** Commands the UI runs itself. Used as-is when the backend cannot list its commands. */
export const BUILTIN_COMMANDS: SlashCommand[] = [
  { name: 'clear', description: 'Start a new conversation (the old one is kept)', kind: 'action' },
  { name: 'plan', description: 'Plan only: Genius explains what it would do without changing files', kind: 'action' },
  { name: 'build', description: 'Build mode: Genius can change files', kind: 'action' },
  { name: 'effort', description: 'How hard Genius works on each request', args: '<fast|balanced|max>', kind: 'action' },
  { name: 'autopilot', description: 'Let Genius apply, check and fix its own changes', args: '<on|off>', kind: 'action' },
  { name: 'undo', description: 'Undo the last request and the file changes it made', kind: 'action' },
  { name: 'compact', description: 'Summarize this conversation to free up context', kind: 'action' },
  { name: 'model', description: 'Choose the AI model (opens Settings)', kind: 'action' },
  { name: 'help', description: 'Show every command', kind: 'action' },
];

export const ACTION_NAMES = new Set(BUILTIN_COMMANDS.map((c) => c.name));

/** Backend list plus any built-in action it did not mention, without duplicates. */
export function mergeCommands(remote: SlashCommand[]): SlashCommand[] {
  const seen = new Set<string>();
  const out: SlashCommand[] = [];
  for (const c of [...remote, ...BUILTIN_COMMANDS]) {
    const name = c.name.replace(/^\//, '');
    if (!name || seen.has(name)) continue;
    seen.add(name);
    // Names the UI implements always run client-side, whatever the backend calls them.
    out.push({ ...c, name, kind: ACTION_NAMES.has(name) ? 'action' : c.kind });
  }
  return out;
}

/** The slash query being typed ("/rev" → "rev"), or null when the popup should be closed. */
export function slashQuery(text: string, caret: number): string | null {
  if (!text.startsWith('/')) return null;
  const head = text.slice(0, caret);
  return /^\/[\w-]*$/.test(head) ? head.slice(1) : null;
}

export function filterCommands(commands: SlashCommand[], query: string): SlashCommand[] {
  const q = query.toLowerCase();
  if (!q) return commands;
  const prefix = commands.filter((c) => c.name.toLowerCase().startsWith(q));
  const inName = commands.filter((c) => !prefix.includes(c) && c.name.toLowerCase().includes(q));
  const inDesc = commands.filter((c) => !prefix.includes(c) && !inName.includes(c) && c.description.toLowerCase().includes(q));
  return [...prefix, ...inName, ...inDesc];
}

/** "/effort fast" → { name: 'effort', arg: 'fast' }. */
export function parseCommand(text: string): { name: string; arg: string } | null {
  const m = text.trim().match(/^\/([\w-]+)(?:\s+([\s\S]*))?$/);
  return m ? { name: m[1].toLowerCase(), arg: (m[2] ?? '').trim() } : null;
}

/** The @-mention being typed right before the caret, with where it starts. */
export function mentionQuery(text: string, caret: number): { query: string; start: number } | null {
  const m = text.slice(0, caret).match(/(?:^|\s)@([^\s@]*)$/);
  if (!m) return null;
  return { query: m[1], start: caret - m[1].length - 1 };
}

/** Subsequence fuzzy score (higher is better), or -1 when `query` does not match. */
export function fuzzyScore(path: string, query: string): number {
  if (!query) return 1;
  const p = path.toLowerCase();
  const q = query.toLowerCase();
  const base = p.slice(p.lastIndexOf('/') + 1);
  if (base.startsWith(q)) return 1000 - p.length;
  if (p.includes(q)) return 800 - p.indexOf(q) - p.length / 10;
  let score = 0;
  let at = 0;
  let prev = -2;
  for (const ch of q) {
    const i = p.indexOf(ch, at);
    if (i < 0) return -1;
    score += i === prev + 1 ? 6 : 1; // reward consecutive runs
    if (i === 0 || '/._-'.includes(p[i - 1])) score += 4; // word starts
    prev = i;
    at = i + 1;
  }
  return score - p.length / 20;
}

export function filterFiles(files: FileEntry[], query: string, limit = 8): FileEntry[] {
  return files
    .filter((f) => f.isText && !f.sensitive)
    .map((f) => ({ f, s: fuzzyScore(f.path, query) }))
    .filter((x) => x.s >= 0)
    .sort((a, b) => b.s - a.s || a.f.path.localeCompare(b.f.path))
    .slice(0, limit)
    .map((x) => x.f);
}

export const EFFORTS: { value: Effort; label: string; hint: string }[] = [
  { value: 'fast', label: '⚡ Fast', hint: 'Fast: quick answers and small edits, fewer steps and checks.' },
  { value: 'balanced', label: 'Balanced', hint: 'Balanced: a good mix of speed and care for everyday changes.' },
  { value: 'max', label: '🧠 Max', hint: 'Max: Genius reads more, thinks longer and double-checks its work. Slower, best results.' },
];

export function isEffort(v: unknown): v is Effort {
  return v === 'fast' || v === 'balanced' || v === 'max';
}

export const AUTOPILOT_HINT = 'Genius applies its changes, runs the type check/build and fixes errors on its own before answering. Everything stays revertible in History.';

const autopilotKey = (mode: 'simple' | 'advanced') => `genius.autopilot.${mode}`;

/** Autopilot is remembered per interface mode: on by default in Simple, off in Advanced. */
export function loadAutopilot(mode: 'simple' | 'advanced'): boolean {
  try {
    const v = localStorage.getItem(autopilotKey(mode));
    if (v === 'on') return true;
    if (v === 'off') return false;
  } catch {
    /* storage unavailable: use the default */
  }
  return mode === 'simple';
}

export function saveAutopilot(mode: 'simple' | 'advanced', on: boolean) {
  try {
    localStorage.setItem(autopilotKey(mode), on ? 'on' : 'off');
  } catch {
    /* storage unavailable: the choice lasts for this session only */
  }
}
