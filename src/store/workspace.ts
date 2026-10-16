import { create } from 'zustand';
import { api, errorMessage } from '../lib/api';
import { applyFileStream, upsertEvent, type Draft } from '../lib/logic';
import type { ActivityEvent, AgentStep, FileEntry, FileStreamEvent, Project, RunInfo, RunOutcome, TaskInfo } from '../lib/types';
import { useApp } from './app';

export type TabId = string; // "file:<path>" | "draft:<path>" | "review:<changesetId>" | "plan" | "commit:<sha>"

export interface RunView {
  info: RunInfo;
  lines: { stream: 'stdout' | 'stderr'; line: string }[];
  state: 'running' | 'succeeded' | 'failed' | 'cancelled';
  outcome?: RunOutcome;
}

/** Element selected in the live preview with the inspector. */
export interface PickedElement {
  tag: string;
  selector: string;
  text: string;
  attributes: Record<string, string>;
  html: string;
  page: string;
}

export function describePicked(p: PickedElement): string {
  const attrs = Object.entries(p.attributes).map(([k, v]) => `${k}="${v}"`).join(' ');
  return `Selected element in the running app (page ${p.page}): <${p.tag}> at "${p.selector}"${p.text ? ` with text "${p.text}"` : ''}${attrs ? ` [${attrs}]` : ''}.\nHTML: ${p.html}`;
}

export interface OpenFile {
  content: string; // editor buffer
  saved: string; // content on disk at last load/save
  error?: string;
}

interface WorkspaceState {
  projectId: string | null;
  project: Project | null;
  files: FileEntry[];
  tabs: TabId[];
  active: TabId | null;
  buffers: Record<string, OpenFile>;
  drafts: Record<string, Draft>;
  followDrafts: boolean;
  aiStream: Record<string, string>;
  activity: ActivityEvent[];
  runs: Record<string, RunView>;
  runOrder: string[];
  preview: { status: 'stopped' | 'starting' | 'ready' | 'failed'; url?: string; error?: string; nonce: number };
  tasks: TaskInfo[];
  gitVersion: number;
  historyVersion: number;
  bottomTab: 'activity' | 'terminal' | 'readiness';
  selectedRun: string | null;
  picked: PickedElement | null;
  picking: boolean;
  rightTab: 'assistant' | 'preview';
  /** Live agent tool calls by task id (real operations pushed by the backend). */
  agentSteps: Record<string, AgentStep[]>;
  onAgentStep: (step: AgentStep) => void;

  open: (projectId: string) => Promise<void>;
  close: () => void;
  refreshProject: () => Promise<void>;
  refreshFiles: () => Promise<void>;
  refreshTasks: () => Promise<void>;
  openTab: (tab: TabId) => void;
  closeTab: (tab: TabId) => void;
  openFile: (path: string) => Promise<void>;
  reloadFile: (path: string, force?: boolean) => Promise<void>;
  setBuffer: (path: string, content: string) => void;
  saveFile: (path: string) => Promise<void>;
  onActivity: (ev: ActivityEvent) => void;
  onAiDelta: (taskId: string, text: string) => void;
  onFileStream: (ev: FileStreamEvent) => void;
  onProcessOutput: (runId: string, stream: 'stdout' | 'stderr', line: string) => void;
  onProcessState: (run: RunInfo, state: RunView['state'], outcome?: RunOutcome) => void;
  setPreview: (p: Partial<WorkspaceState['preview']>) => void;
  set: (p: Partial<WorkspaceState>) => void;
}

const MAX_LINES = 5000;

const initial = {
  projectId: null,
  project: null,
  files: [],
  tabs: [],
  active: null,
  buffers: {},
  drafts: {},
  followDrafts: true,
  aiStream: {},
  activity: [],
  runs: {},
  runOrder: [],
  preview: { status: 'stopped' as const, nonce: 0 },
  tasks: [],
  gitVersion: 0,
  historyVersion: 0,
  bottomTab: 'activity' as const,
  selectedRun: null as string | null,
  picked: null as PickedElement | null,
  picking: false,
  rightTab: 'assistant' as const,
  agentSteps: {} as Record<string, AgentStep[]>,
};

export const useWorkspace = create<WorkspaceState>((set, get) => ({
  ...initial,

  onAgentStep: (step) => {
    if (step.projectId !== get().projectId) return;
    const cur = get().agentSteps[step.taskId] ?? [];
    if (cur.some((x) => x.id === step.id)) return;
    set({ agentSteps: { ...get().agentSteps, [step.taskId]: [...cur, step] } });
  },

  open: async (projectId) => {
    if (get().projectId === projectId) return;
    set({ ...initial, projectId });
    try {
      const [project, files, activity, tasks, runs, active, url] = await Promise.all([
        api.openProject(projectId),
        api.listFiles(projectId).catch(() => [] as FileEntry[]),
        api.listActivity(projectId),
        api.activeTasks(projectId),
        api.listRuns(projectId),
        api.activeRuns(projectId),
        api.previewStatus(projectId),
      ]);
      if (get().projectId !== projectId) return;
      // Rehydrate terminal history from persisted run tails (real output from previous runs).
      const runViews: Record<string, RunView> = {};
      const order: string[] = [];
      for (const r of runs.slice().reverse()) {
        const live = active.find((a) => a.id === r.id);
        runViews[r.id] = {
          info: { id: r.id, projectId: r.projectId, op: r.op, command: r.command, cwd: r.cwd, reason: r.reason, startedAt: r.startedAt, port: live?.port ?? null },
          lines: r.outputTail ? r.outputTail.split('\n').map((line) => ({ stream: 'stdout' as const, line })) : [],
          state: live ? 'running' : r.status === 'succeeded' ? 'succeeded' : r.status === 'running' ? 'running' : r.status === 'cancelled' || r.status === 'interrupted' ? 'cancelled' : 'failed',
          outcome: r.exitCode != null || r.endedAt ? { id: r.id, op: r.op, status: r.status as RunOutcome['status'], exitCode: r.exitCode, durationMs: r.endedAt ? Date.parse(r.endedAt) - Date.parse(r.startedAt) : 0, outputTail: '' } : undefined,
        };
        order.push(r.id);
      }
      const hasCode = files.some((f) => f.path === 'src/App.tsx');
      const showPlan = !hasCode && (!!project.spec || project.status.stage === 'created' || tasks.some((t) => t.kind === 'plan'));
      set({
        project,
        files,
        activity,
        tasks,
        runs: runViews,
        runOrder: order,
        preview: url ? { status: 'ready', url, nonce: 0 } : { status: 'stopped', nonce: 0 },
        tabs: showPlan ? ['plan'] : [],
        active: showPlan ? 'plan' : null,
      });
      if (hasCode) void get().openFile('src/App.tsx');
    } catch (e) {
      useApp.getState().toast('bad', `Could not open project: ${errorMessage(e)}`);
    }
  },

  close: () => set({ ...initial }),

  refreshProject: async () => {
    const id = get().projectId;
    if (!id) return;
    try {
      const project = await api.getProject(id);
      if (get().projectId === id) set({ project });
    } catch {
      /* project may have been deleted */
    }
  },

  refreshFiles: async () => {
    const id = get().projectId;
    if (!id) return;
    try {
      const files = await api.listFiles(id);
      if (get().projectId === id) set({ files });
    } catch (e) {
      useApp.getState().toast('bad', errorMessage(e));
    }
  },

  refreshTasks: async () => {
    const id = get().projectId;
    if (!id) return;
    const tasks = await api.activeTasks(id).catch(() => []);
    if (get().projectId === id) set({ tasks });
  },

  openTab: (tab) => {
    const { tabs } = get();
    set({ tabs: tabs.includes(tab) ? tabs : [...tabs, tab], active: tab });
  },

  closeTab: (tab) => {
    const { tabs, active } = get();
    const i = tabs.indexOf(tab);
    const next = tabs.filter((t) => t !== tab);
    set({ tabs: next, active: active === tab ? (next[Math.min(i, next.length - 1)] ?? null) : active });
  },

  openFile: async (path) => {
    get().openTab(`file:${path}`);
    if (!get().buffers[path]) await get().reloadFile(path, true);
  },

  reloadFile: async (path, force = false) => {
    const id = get().projectId;
    if (!id) return;
    const cur = get().buffers[path];
    if (cur && cur.content !== cur.saved && !force) return; // keep unsaved user edits
    try {
      const content = await api.readFile(id, path);
      set({ buffers: { ...get().buffers, [path]: { content, saved: content } } });
    } catch (e) {
      set({ buffers: { ...get().buffers, [path]: { content: '', saved: '', error: errorMessage(e) } } });
    }
  },

  setBuffer: (path, content) => {
    const b = get().buffers[path];
    if (!b) return;
    set({ buffers: { ...get().buffers, [path]: { ...b, content } } });
  },

  saveFile: async (path) => {
    const id = get().projectId;
    const b = get().buffers[path];
    if (!id || !b || b.error) return;
    try {
      await api.saveFile(id, path, b.content);
      set({ buffers: { ...get().buffers, [path]: { ...b, saved: b.content } }, historyVersion: get().historyVersion + 1 });
    } catch (e) {
      useApp.getState().toast('bad', `Save failed: ${errorMessage(e)}`);
    }
  },

  onActivity: (ev) => {
    const s = get();
    if (ev.projectId !== s.projectId) return;
    set({ activity: upsertEvent(s.activity, ev) });
    if (ev.kind === 'task') {
      void s.refreshTasks();
      if (ev.detail.taskKind === 'plan' && ev.status === 'completed') get().openTab('plan');
      if (ev.status !== 'running') {
        void s.refreshProject();
        set({ gitVersion: get().gitVersion + 1, historyVersion: get().historyVersion + 1 });
      }
    }
    if (ev.kind === 'changeset' || ev.kind === 'file_write') set({ historyVersion: get().historyVersion + 1, gitVersion: get().gitVersion + 1 });
    if (ev.kind === 'git' || ev.kind === 'github') set({ gitVersion: get().gitVersion + 1 });
    if (ev.kind === 'changeset' && ev.status === 'awaiting_approval' && typeof ev.detail.changesetId === 'string') {
      get().openTab(`review:${ev.detail.changesetId}`);
    }
    if (ev.kind === 'build' || ev.kind === 'test' || ev.kind === 'install') void s.refreshProject();
  },

  onAiDelta: (taskId, text) => {
    const cur = get().aiStream[taskId] ?? '';
    const next = (cur + text).slice(-200_000);
    set({ aiStream: { ...get().aiStream, [taskId]: next } });
  },

  onFileStream: (ev) => {
    if (ev.projectId !== get().projectId) return;
    set({ drafts: applyFileStream(get().drafts, ev) });
    if (ev.type === 'fileStart' && get().followDrafts) get().openTab(`draft:${ev.path}`);
  },

  onProcessOutput: (runId, stream, line) => {
    const r = get().runs[runId];
    if (!r) return;
    const lines = r.lines.length >= MAX_LINES ? r.lines.slice(r.lines.length - MAX_LINES + 1) : r.lines.slice();
    lines.push({ stream, line });
    set({ runs: { ...get().runs, [runId]: { ...r, lines } } });
  },

  onProcessState: (run, state, outcome) => {
    if (run.projectId !== get().projectId) return;
    const existing = get().runs[run.id];
    const runs = { ...get().runs, [run.id]: { info: run, lines: existing?.lines ?? [], state, outcome } };
    const runOrder = get().runOrder.includes(run.id) ? get().runOrder : [run.id, ...get().runOrder];
    set({ runs, runOrder });
  },

  setPreview: (p) => set({ preview: { ...get().preview, ...p } }),
  set: (p) => set(p),
}));
