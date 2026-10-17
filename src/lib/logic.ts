// Pure, framework-free state logic (unit tested in logic.test.ts).
import type { ActivityEvent, EventKind, FileEntry, FileStreamEvent, PreviewState, PreviewStatus } from './types';

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

/** Name of the innermost component in a React component stack ("    at Foo (url)" lines). */
export function firstComponent(stack: string): string | null {
  const m = /^\s*(?:at\s+)?([A-Z][\w$.]*)/m.exec(stack.trim());
  return m ? m[1] : null;
}

// ---------------------------------------------------------------- preview


export function idlePreview(projectId: string | null): PreviewState {
  return { projectId: projectId ?? '', status: 'idle', message: 'The preview is not running.', url: null, port: null, runId: null, problem: null, modulesChecked: 0, generation: 0, inspector: false, updatedAt: '' };
}

/**
 * Accept a backend preview state only if it belongs to the open project and is not older
 * than what is shown (events of a previous server generation never overwrite a newer one).
 */
export function applyPreviewState(cur: PreviewState, next: PreviewState, openProjectId: string | null): PreviewState {
  if (!openProjectId || next.projectId !== openProjectId) return cur;
  if (cur.projectId === next.projectId && next.generation < cur.generation) return cur;
  return next;
}

export const PREVIEW_STEPS: { status: PreviewStatus; label: string }[] = [
  { status: 'checking', label: 'Check project' },
  { status: 'installing', label: 'Install dependencies' },
  { status: 'starting', label: 'Start server' },
  { status: 'compiling', label: 'Compile app' },
  { status: 'ready', label: 'Ready' },
];

export function previewBusy(s: PreviewStatus): boolean {
  return s === 'checking' || s === 'installing' || s === 'starting' || s === 'compiling' || s === 'stopping';
}

/** The iframe may show the page only when a real server of this run is answering. */
export function canShowFrame(p: PreviewState): boolean {
  return !!p.url && !!p.runId && (p.status === 'ready' || (p.status === 'failed' && p.problem?.stage === 'compile'));
}

export function previewLabel(p: PreviewState): { text: string; tone: 'ok' | 'bad' | 'run' | 'idle' } {
  if (p.status === 'ready') return { text: 'Preview running', tone: 'ok' };
  if (p.status === 'failed') return { text: p.problem?.stage === 'compile' ? 'App has an error' : 'Preview failed', tone: 'bad' };
  if (previewBusy(p.status)) return { text: p.status === 'stopping' ? 'Stopping…' : 'Starting…', tone: 'run' };
  return { text: 'Preview stopped', tone: 'idle' };
}

// ---------------------------------------------------------------- beginner activity summary

export interface ActivityItem {
  id: string;
  kind: EventKind;
  status: ActivityEvent['status'];
  title: string;
  ts: string;
  files?: string[];
}

/**
 * Plain-language view of real activity events: consecutive file reads and writes are grouped
 * ("Read 3 files: …"), model token traffic is reduced to "Thinking", and nothing is invented —
 * every item maps to recorded events and keeps their real status.
 */
export function summarizeActivity(events: ActivityEvent[], max = 40): ActivityItem[] {
  const out: ActivityItem[] = [];
  for (const e of events) {
    const prev = out[out.length - 1];
    if (e.kind === 'file_read' || e.kind === 'file_write') {
      const path = typeof e.detail.path === 'string' ? e.detail.path : null;
      const files = path ? [path] : Array.isArray(e.detail.files) ? (e.detail.files as string[]) : [];
      if (prev && prev.kind === e.kind && prev.status === e.status && prev.files) {
        prev.files = [...new Set([...prev.files, ...files])];
        prev.title = `${e.kind === 'file_read' ? 'Read' : 'Changed'} ${prev.files.length} file${prev.files.length === 1 ? '' : 's'}: ${prev.files.slice(0, 4).map((f) => f.split('/').pop()).join(', ')}${prev.files.length > 4 ? '…' : ''}`;
        prev.ts = e.ts;
        continue;
      }
      out.push({ id: e.id, kind: e.kind, status: e.status, ts: e.ts, files, title: files.length ? `${e.kind === 'file_read' ? 'Read' : 'Changed'} ${files.length} file${files.length === 1 ? '' : 's'}: ${files.slice(0, 4).map((f) => f.split('/').pop()).join(', ')}` : e.title });
      continue;
    }
    if (e.kind === 'ai') {
      const title = e.status === 'running' ? 'Thinking…' : e.status === 'failed' ? `AI request failed: ${(e.detail.error as { message?: string } | undefined)?.message ?? e.title}` : 'Thought about the next step';
      if (prev && prev.kind === 'ai' && prev.status !== 'failed') {
        Object.assign(prev, { id: e.id, status: e.status, title, ts: e.ts });
        continue;
      }
      out.push({ id: e.id, kind: e.kind, status: e.status, ts: e.ts, title });
      continue;
    }
    out.push({ id: e.id, kind: e.kind, status: e.status, ts: e.ts, title: e.title });
  }
  return out.slice(-max);
}
