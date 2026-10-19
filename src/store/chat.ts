// Genius Chat / Research state. Conversations live in the hidden general workspace on the same
// store as project conversations; this store only tracks what the chat views show. Switching
// between Chat, Code and Research keeps it (the store outlives the views).
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { create } from 'zustand';
import { api, errorMessage } from '../lib/api';
import type { ActivityEvent, AgentStep, Attachment, ChatConversation, ChatMode, MessageRow, ResearchSource, TaskInfo } from '../lib/types';
import { useApp } from './app';

export interface ChatQueued {
  id: string;
  taskId: string | null;
  text: string;
  status: 'queued' | 'delivered' | 'undelivered';
}

interface ChatState {
  workspaceId: string | null;
  conversations: Record<ChatMode, ChatConversation[]>;
  /** Every Chat and Research conversation, newest first (the sidebar history). */
  all: ChatConversation[];
  active: Record<ChatMode, string | null>;
  query: string;
  messages: MessageRow[];
  sources: ResearchSource[];
  tasks: TaskInfo[];
  steps: Record<string, AgentStep[]>;
  queue: ChatQueued[];
  /** A prompt typed on Home in another mode, consumed by the view that opens. */
  pending: { mode: ChatMode; text: string } | null;
  error: string | null;

  load: (mode: ChatMode) => Promise<void>;
  /** (Re)load the sidebar history, optionally filtered by a search over titles and message text. */
  loadAll: (query?: string) => Promise<void>;
  /** Start a blank conversation: nothing is created until the first message is sent. */
  fresh: (mode: ChatMode) => void;
  search: (mode: ChatMode, query: string) => Promise<void>;
  select: (mode: ChatMode, id: string | null) => Promise<void>;
  create: (mode: ChatMode) => Promise<ChatConversation | null>;
  rename: (mode: ChatMode, id: string, title: string) => Promise<void>;
  remove: (mode: ChatMode, id: string) => Promise<void>;
  refresh: (mode: ChatMode) => Promise<void>;
  send: (mode: ChatMode, text: string, attachments?: Attachment[]) => Promise<void>;
  setPending: (p: ChatState['pending']) => void;
}

const empty = { chat: [], research: [] } as Record<ChatMode, ChatConversation[]>;
let qid = 0;

export const useChat = create<ChatState>((set, get) => ({
  workspaceId: null,
  conversations: empty,
  all: [],
  active: { chat: null, research: null },
  query: '',
  messages: [],
  sources: [],
  tasks: [],
  steps: {},
  queue: [],
  pending: null,
  error: null,

  setPending: (pending) => set({ pending }),

  load: async (mode) => {
    try {
      const ws = get().workspaceId ?? (await api.chatWorkspace()).id;
      const list = await api.listChatConversations(mode, get().query || undefined);
      const tasks = await api.activeTasks(ws);
      set({ workspaceId: ws, conversations: { ...get().conversations, [mode]: list }, tasks, error: null });
      // A conversation stays open only if the user opened it; otherwise start blank (like a new chat).
      const cur = get().active[mode];
      await get().select(mode, cur && list.some((c) => c.id === cur) ? cur : null);
      void get().loadAll();
    } catch (e) {
      set({ error: errorMessage(e) });
    }
  },

  loadAll: async (query) => {
    const q = query ?? get().query;
    const all = await api.listChatConversations(null, q || undefined).catch(() => get().all);
    set({ all });
  },

  fresh: (mode) => set({ active: { ...get().active, [mode]: null }, messages: [], sources: [], queue: [] }),

  search: async (mode, query) => {
    set({ query });
    const list = await api.listChatConversations(mode, query || undefined).catch(() => []);
    set({ conversations: { ...get().conversations, [mode]: list } });
    await get().loadAll(query);
  },

  select: async (mode, id) => {
    set({ active: { ...get().active, [mode]: id }, messages: [], sources: [] });
    if (!id) return;
    const [messages, sources] = await Promise.all([api.conversationMessages(id), mode === 'research' ? api.researchSources(id) : Promise.resolve([])]);
    if (get().active[mode] === id) set({ messages, sources });
  },

  create: async (mode) => {
    try {
      const c = await api.createChatConversation(mode);
      set({ conversations: { ...get().conversations, [mode]: [c, ...get().conversations[mode]] }, all: [c, ...get().all], active: { ...get().active, [mode]: c.id }, messages: [], sources: [] });
      return c;
    } catch (e) {
      useApp.getState().toast('bad', errorMessage(e));
      return null;
    }
  },

  rename: async (mode, id, title) => {
    await api.renameConversation(id, title.trim());
    await get().refresh(mode);
  },

  remove: async (mode, id) => {
    await api.deleteConversation(id);
    if (get().active[mode] === id) set({ active: { ...get().active, [mode]: null }, messages: [], sources: [] });
    await get().load(mode);
  },

  refresh: async (mode) => {
    const id = get().active[mode];
    const list = await api.listChatConversations(mode, get().query || undefined).catch(() => get().conversations[mode]);
    set({ conversations: { ...get().conversations, [mode]: list } });
    void get().loadAll();
    if (id) {
      const [messages, sources] = await Promise.all([api.conversationMessages(id), mode === 'research' ? api.researchSources(id) : Promise.resolve([])]);
      if (get().active[mode] === id) set({ messages, sources });
    }
  },

  send: async (mode, text, attachments = []) => {
    const trimmed = text.trim();
    if (!trimmed) return;
    let id = get().active[mode];
    if (!id) id = (await get().create(mode))?.id ?? null;
    if (!id) return;
    const running = get().tasks.find((t) => ['chat', 'research'].includes(t.kind));
    if (running) {
      // steering: the running agent reads it after its current step
      const q: ChatQueued = { id: `q${++qid}`, taskId: running.id, text: trimmed, status: 'queued' };
      set({ queue: [...get().queue, q] });
      api.steerAgent(running.id, trimmed).catch(() => set({ queue: get().queue.map((x) => (x.id === q.id ? { ...x, status: 'undelivered' } : x)) }));
      return;
    }
    try {
      const task = await api.startChat(id, { message: trimmed, attachmentIds: attachments.filter((a) => a.status === 'ready').map((a) => a.id) });
      set({ tasks: [...get().tasks, task], messages: await api.conversationMessages(id) });
    } catch (e) {
      useApp.getState().toast('bad', errorMessage(e));
      set({ messages: await api.conversationMessages(id).catch(() => get().messages) });
    }
  },
}));

/** Backend events for the chat views (only those of the general workspace). */
export async function subscribeChat(): Promise<UnlistenFn> {
  const st = () => useChat.getState();
  const mine = (pid?: string | null) => !!pid && pid === st().workspaceId;
  const modeOf = (): ChatMode | null => {
    const a = st().active;
    return a.research && !a.chat ? 'research' : a.chat ? 'chat' : null;
  };
  const un = await Promise.all([
    listen<ActivityEvent>('genius://activity', async ({ payload }) => {
      if (!mine(payload.projectId) || payload.kind !== 'task') return;
      const ws = st().workspaceId!;
      const tasks = await api.activeTasks(ws).catch(() => st().tasks);
      useChat.setState({ tasks });
      if (payload.status !== 'running') {
        useChat.setState({ queue: st().queue.filter((q) => q.status === 'queued' && tasks.some((t) => t.id === q.taskId)) });
        for (const m of ['chat', 'research'] as ChatMode[]) if (st().active[m]) await st().refresh(m);
        void modeOf;
      }
    }),
    listen<AgentStep>('genius://agent-step', ({ payload }) => {
      if (!mine(payload.projectId)) return;
      const cur = st().steps[payload.taskId] ?? [];
      if (!cur.some((x) => x.id === payload.id)) useChat.setState({ steps: { ...st().steps, [payload.taskId]: [...cur, payload] } });
    }),
    listen<{ taskId: string; projectId: string; message: string }>('genius://agent-steered', ({ payload }) => {
      if (!mine(payload.projectId)) return;
      useChat.setState({ queue: st().queue.map((q) => (q.taskId === payload.taskId && q.text.trim() === payload.message.trim() ? { ...q, status: 'delivered' as const } : q)) });
    }),
  ]);
  return () => un.forEach((u) => u());
}
