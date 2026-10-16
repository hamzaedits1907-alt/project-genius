// Pure, framework-free state logic (unit tested in logic.test.ts).
import type { ActivityEvent, EventKind, FileEntry, FileStreamEvent } from './types';

// ---------------------------------------------------------------- activity

/** Insert or replace an event by id, keeping chronological order and a bounded size. */
export function upsertEvent(list: ActivityEvent[], ev: ActivityEvent, max = 2000): ActivityEvent[] {
  const idx = list.findIndex((e) => e.id === ev.id);
  let next: ActivityEvent[];
  if (idx >= 0) {
    next = list.slice();
    next[idx] = ev;
  } else {
    next = [...list, ev];
    if (next.length > 1 && next[next.length - 2].ts > ev.ts) next.sort((a, b) => (a.ts < b.ts ? -1 : a.ts > b.ts ? 1 : 0));
  }
  return next.length > max ? next.slice(next.length - max) : next;
}

export type ActivityFilter = 'all' | 'ai' | 'files' | 'commands' | 'builds' | 'git' | 'errors' | 'approvals';

const FILTER_KINDS: Record<Exclude<ActivityFilter, 'all' | 'errors'>, EventKind[]> = {
  ai: ['ai', 'plan', 'diagnosis', 'task'],
  files: ['file_read', 'file_write', 'changeset', 'export'],
  commands: ['command', 'install', 'preview'],
  builds: ['build', 'test', 'diagnosis'],
  git: ['git', 'github'],
  approvals: ['approval'],
};

export function matchesFilter(e: ActivityEvent, f: ActivityFilter): boolean {
  if (f === 'all') return true;
  if (f === 'errors') return e.status === 'failed';
  return FILTER_KINDS[f].includes(e.kind);
}

// ---------------------------------------------------------------- file tree

export interface TreeNode {
  name: string;
  path: string;
  dir: boolean;
  children: TreeNode[];
  entry?: FileEntry;
}

export function buildTree(entries: { path: string }[]): TreeNode[] {
  const root: TreeNode = { name: '', path: '', dir: true, children: [] };
  for (const e of entries) {
    const parts = e.path.split('/');
    let node = root;
    parts.forEach((part, i) => {
      const isLast = i === parts.length - 1;
      const path = parts.slice(0, i + 1).join('/');
      let child = node.children.find((c) => c.name === part && c.dir === !isLast);
      if (!child) {
        child = { name: part, path, dir: !isLast, children: [], entry: isLast ? (e as FileEntry) : undefined };
        node.children.push(child);
      }
      node = child;
    });
  }
  const sort = (n: TreeNode) => {
    n.children.sort((a, b) => (a.dir === b.dir ? a.name.localeCompare(b.name) : a.dir ? -1 : 1));
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}

// ---------------------------------------------------------------- live drafts

export type DraftStatus = 'streaming' | 'complete' | 'incomplete' | 'delete';

export interface Draft {
  path: string;
  content: string;
  status: DraftStatus;
  taskId: string;
}

/** Apply a streamed file event. Drafts are AI output only — never what is on disk. */
export function applyFileStream(drafts: Record<string, Draft>, ev: FileStreamEvent): Record<string, Draft> {
  switch (ev.type) {
    case 'fileStart':
      return { ...drafts, [ev.path]: { path: ev.path, content: '', status: 'streaming', taskId: ev.taskId } };
    case 'fileDelta': {
      const d = drafts[ev.path] ?? { path: ev.path, content: '', status: 'streaming' as const, taskId: ev.taskId };
      return { ...drafts, [ev.path]: { ...d, content: d.content + ev.text } };
    }
    case 'fileEnd':
      return { ...drafts, [ev.path]: { path: ev.path, content: ev.content, status: ev.complete ? 'complete' : 'incomplete', taskId: ev.taskId } };
    case 'delete':
      return { ...drafts, [ev.path]: { path: ev.path, content: '', status: 'delete', taskId: ev.taskId } };
    default:
      return drafts;
  }
}

// ---------------------------------------------------------------- formatting

export function timeAgo(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return 'never';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return iso;
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 45) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

export function clockTime(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

export function duration(ms: number | null | undefined): string {
  if (ms == null) return '';
  if (ms < 1000) return `${ms} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}m ${Math.round(s % 60)}s`;
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function languageFor(path: string): 'ts' | 'tsx' | 'js' | 'jsx' | 'css' | 'html' | 'json' | 'md' | 'text' {
  const ext = path.split('.').pop()?.toLowerCase() ?? '';
  if (ext === 'ts' || ext === 'mts' || ext === 'cts') return 'ts';
  if (ext === 'tsx') return 'tsx';
  if (ext === 'js' || ext === 'mjs' || ext === 'cjs') return 'js';
  if (ext === 'jsx') return 'jsx';
  if (ext === 'css' || ext === 'scss') return 'css';
  if (ext === 'html' || ext === 'htm' || ext === 'svg') return 'html';
  if (ext === 'json') return 'json';
  if (ext === 'md' || ext === 'markdown') return 'md';
  return 'text';
}

/** Classify unified-diff lines for rendering. */
export function diffLines(diff: string): { kind: 'add' | 'del' | 'hunk' | 'meta' | 'ctx'; text: string }[] {
  return diff.split('\n').map((text) => {
    if (text.startsWith('+++') || text.startsWith('---') || text.startsWith('diff ') || text.startsWith('index ')) return { kind: 'meta', text };
    if (text.startsWith('@@')) return { kind: 'hunk', text };
    if (text.startsWith('+')) return { kind: 'add', text };
    if (text.startsWith('-')) return { kind: 'del', text };
    return { kind: 'ctx', text };
  });
}

export function statusTone(status: string): 'ok' | 'bad' | 'warn' | 'run' | 'idle' {
  switch (status) {
    case 'completed':
    case 'succeeded':
    case 'pass':
    case 'applied':
      return 'ok';
    case 'failed':
    case 'fail':
    case 'spawn_failed':
      return 'bad';
    case 'awaiting_approval':
    case 'warn':
    case 'proposed':
      return 'warn';
    case 'running':
    case 'queued':
    case 'starting':
      return 'run';
    default:
      return 'idle';
  }
}

export const STATUS_LABEL: Record<string, string> = {
  queued: 'Queued',
  running: 'Running',
  completed: 'Completed',
  failed: 'Failed',
  cancelled: 'Cancelled',
  awaiting_approval: 'Awaiting approval',
};
