// Command registry for the command palette (Ctrl/Cmd+K). Any view or panel can contribute
// commands while it is mounted:
//
//   useEffect(() => registerCommands('git-panel', [{ id: 'git.commit', title: 'Commit', run }]), [run]);
//
// registerCommands returns its own cleanup, so it can be returned from an effect directly.
import { useSyncExternalStore, type ReactNode } from 'react';

export interface PaletteCommand {
  /** Unique, stable id (e.g. "ws.run"). */
  id: string;
  title: string;
  /** Section heading in the palette, e.g. "Project" or "Navigation". */
  group?: string;
  /** Secondary text shown after the title (a path, a project description). */
  subtitle?: string;
  /** Extra words that should match the search (synonyms). */
  keywords?: string[];
  /** Shortcut hint, e.g. "Ctrl+," (display only). */
  shortcut?: string;
  icon?: ReactNode;
  /** Only listed when the query is not empty (long lists such as project files). */
  searchOnly?: boolean;
  run: () => void | Promise<void>;
}

const registry = new Map<string, PaletteCommand[]>();
const listeners = new Set<() => void>();
let snapshot: PaletteCommand[] = [];

function changed() {
  snapshot = [...registry.values()].flat();
  for (const l of listeners) l();
}

/** Adds (or replaces) the commands contributed by `owner`. Returns an unregister function. */
export function registerCommands(owner: string, commands: PaletteCommand[]): () => void {
  registry.set(owner, commands);
  changed();
  return () => {
    // Only remove what this call registered: a newer registration under the same owner stays.
    if (registry.get(owner) === commands) unregisterCommands(owner);
  };
}

export function unregisterCommands(owner: string) {
  if (registry.delete(owner)) changed();
}

/** Every registered command (stable array until the registry changes). */
export function registeredCommands(): PaletteCommand[] {
  return snapshot;
}

export function subscribeCommands(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** React hook: the registered commands, re-rendering when they change. */
export function useRegisteredCommands(): PaletteCommand[] {
  return useSyncExternalStore(subscribeCommands, registeredCommands, registeredCommands);
}

export interface FuzzyMatch {
  score: number;
  /** Indices of the matched characters in the text (for highlighting). */
  indices: number[];
}

/**
 * Subsequence fuzzy match: every query character must appear in order. Consecutive runs,
 * word starts and an early first match score higher. Case-insensitive; spaces in the query
 * are ignored.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
  const q = query.toLowerCase().replace(/\s+/g, '');
  if (!q) return { score: 0, indices: [] };
  const t = text.toLowerCase();
  const indices: number[] = [];
  let score = 0;
  let ti = 0;
  let prev = -2;
  for (const ch of q) {
    const i = t.indexOf(ch, ti);
    if (i < 0) return null;
    const wordStart = i === 0 || /[\s/_.\-:]/.test(t[i - 1]) || (text[i] !== t[i] && text[i - 1] === t[i - 1]);
    score += 1;
    if (i === prev + 1) score += 5;
    if (wordStart) score += 8;
    score -= Math.min(i - ti, 10) * 0.2;
    indices.push(i);
    prev = i;
    ti = i + 1;
  }
  if (t.startsWith(q)) score += 15;
  else if (t.includes(q)) score += 8;
  score -= text.length * 0.01;
  return { score, indices };
}

export interface RankedCommand {
  command: PaletteCommand;
  score: number;
  /** Matched character indices in the title. */
  indices: number[];
}

/** Commands matching the query, best first (all non-search-only commands for an empty query). */
export function rankCommands(commands: PaletteCommand[], query: string, limit = 80): RankedCommand[] {
  const q = query.trim();
  if (!q) return commands.filter((c) => !c.searchOnly).map((command) => ({ command, score: 0, indices: [] })).slice(0, limit);
  const out: RankedCommand[] = [];
  for (const command of commands) {
    const title = fuzzyMatch(q, command.title);
    let best = title ? { score: title.score + 2, indices: title.indices } : null;
    for (const extra of [command.subtitle, command.group, ...(command.keywords ?? [])]) {
      if (!extra) continue;
      const m = fuzzyMatch(q, extra);
      if (m && (!best || m.score - 4 > best.score)) best = { score: m.score - 4, indices: [] };
    }
    if (best) out.push({ command, ...best });
  }
  return out.sort((a, b) => b.score - a.score).slice(0, limit);
}
