// Persistence for Genius Web. The runtime talks to the `WebStore` interface only; there are two
// implementations: SupabaseStore (accounts, row level security — the real thing) and MemoryStore
// (tests and the explicitly local-only development mode).
import type { Source } from './engine/research';
import type { Mission } from '../lib/types';

export type Mode = 'chat' | 'research' | 'agent';

export interface ConversationRec {
  id: string;
  mode: Mode;
  title: string;
  createdAt: string;
  updatedAt: string;
  origin: Record<string, unknown> | null;
}
export interface MessageRec {
  id: string;
  conversationId: string;
  role: 'user' | 'assistant';
  kind: string;
  content: string;
  meta: Record<string, unknown>;
  taskId: string | null;
  createdAt: string;
}
export interface SourceRec extends Source {
  id: string;
  conversationId: string;
  taskId: string | null;
}
export interface AttachmentRec {
  id: string;
  conversationId: string | null;
  name: string;
  kind: string;
  mime: string;
  size: number;
  scope: 'message' | 'reference';
  status: 'ready' | 'failed';
  error: string | null;
  /** Extracted text (empty for images). */
  text: string;
  /** Images only: the picture as a data URL. */
  imageDataUrl: string | null;
  meta: Record<string, unknown>;
  createdAt: string;
}

export interface WebStore {
  /** Conversations newest first; `query` matches titles and message text. */
  listConversations(mode: Mode | null, query?: string): Promise<(ConversationRec & { snippet: string; messageCount: number })[]>;
  getConversation(id: string): Promise<ConversationRec | null>;
  createConversation(mode: Mode, title: string, origin?: Record<string, unknown> | null): Promise<ConversationRec>;
  renameConversation(id: string, title: string): Promise<void>;
  deleteConversation(id: string): Promise<void>;
  listMessages(conversationId: string): Promise<MessageRec[]>;
  addMessage(m: Omit<MessageRec, 'id' | 'createdAt'> & { id?: string; createdAt?: string }): Promise<MessageRec>;
  /** Remove `messageId` and every later message of the conversation (and the sources of removed tasks). */
  deleteFrom(conversationId: string, messageId: string): Promise<number>;
  listSources(conversationId: string): Promise<SourceRec[]>;
  /** Adds a fetched page; the same final URL keeps its number. */
  upsertSource(conversationId: string, taskId: string | null, s: Omit<Source, 'n'>): Promise<SourceRec>;
  setSourceUsage(conversationId: string, n: number, usedFor: string): Promise<void>;
  addAttachment(a: Omit<AttachmentRec, 'id' | 'createdAt'>): Promise<AttachmentRec>;
  listAttachments(conversationId?: string | null): Promise<AttachmentRec[]>;
  getAttachment(id: string): Promise<AttachmentRec | null>;
  updateAttachment(id: string, patch: Partial<Pick<AttachmentRec, 'scope' | 'conversationId'>>): Promise<AttachmentRec | null>;
  deleteAttachment(id: string): Promise<void>;
  getPrefs(): Promise<Record<string, unknown> | null>;
  savePrefs(settings: Record<string, unknown>): Promise<void>;
  listMissions(): Promise<Mission[]>;
  saveMission(m: Mission): Promise<void>;
  /** Count one use for the (aggregate-only) usage summary. */
  recordUsage(kind: 'chat' | 'research' | 'agent' | 'code' | 'web_tool'): Promise<void>;
}

const nowIso = () => new Date().toISOString();
const uid = () => (crypto.randomUUID ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(16).slice(2)}`);

/** In-memory store, optionally mirrored to `localStorage` (local-only development mode). */
export class MemoryStore implements WebStore {
  private convs: ConversationRec[] = [];
  private msgs: MessageRec[] = [];
  private srcs: SourceRec[] = [];
  private atts: AttachmentRec[] = [];
  private prefs: Record<string, unknown> | null = null;
  private missions: Mission[] = [];
  usage: Record<string, number> = {};

  constructor(private persistKey?: string) {
    if (persistKey) {
      try {
        const raw = localStorage.getItem(persistKey);
        if (raw) Object.assign(this, JSON.parse(raw));
      } catch {
        /* start empty */
      }
    }
  }
  private save() {
    if (!this.persistKey) return;
    try {
      const { convs, msgs, srcs, prefs, missions } = this;
      localStorage.setItem(this.persistKey, JSON.stringify({ convs, msgs, srcs, prefs, missions, atts: this.atts.map((a) => ({ ...a, imageDataUrl: null })) }));
    } catch {
      /* storage full or unavailable: the session still works */
    }
  }

  async listConversations(mode: Mode | null, query?: string) {
    const q = query?.trim().toLowerCase();
    return this.convs
      .filter((c) => (!mode || c.mode === mode) && (!q || c.title.toLowerCase().includes(q) || this.msgs.some((m) => m.conversationId === c.id && m.content.toLowerCase().includes(q))))
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
      .map((c) => {
        const ms = this.msgs.filter((m) => m.conversationId === c.id);
        const hit = q ? ms.find((m) => m.content.toLowerCase().includes(q)) : ms[ms.length - 1];
        return { ...c, snippet: (hit?.content ?? '').replace(/\s+/g, ' ').slice(0, 140), messageCount: ms.length };
      });
  }
  async getConversation(id: string) {
    return this.convs.find((c) => c.id === id) ?? null;
  }
  async createConversation(mode: Mode, title: string, origin: Record<string, unknown> | null = null) {
    const c: ConversationRec = { id: uid(), mode, title, createdAt: nowIso(), updatedAt: nowIso(), origin };
    this.convs.push(c);
    this.save();
    return c;
  }
  async renameConversation(id: string, title: string) {
    const c = this.convs.find((x) => x.id === id);
    if (c) c.title = title.slice(0, 200);
    this.save();
  }
  async deleteConversation(id: string) {
    this.convs = this.convs.filter((c) => c.id !== id);
    this.msgs = this.msgs.filter((m) => m.conversationId !== id);
    this.srcs = this.srcs.filter((s) => s.conversationId !== id);
    this.atts = this.atts.filter((a) => a.conversationId !== id);
    this.save();
  }
  async listMessages(conversationId: string) {
    return this.msgs.filter((m) => m.conversationId === conversationId);
  }
  async addMessage(m: Omit<MessageRec, 'id' | 'createdAt'> & { id?: string; createdAt?: string }) {
    const rec: MessageRec = { ...m, id: m.id ?? uid(), createdAt: m.createdAt ?? nowIso() };
    this.msgs.push(rec);
    const c = this.convs.find((x) => x.id === m.conversationId);
    if (c) c.updatedAt = nowIso();
    this.save();
    return rec;
  }
  async deleteFrom(conversationId: string, messageId: string) {
    const own = this.msgs.filter((m) => m.conversationId === conversationId);
    const i = own.findIndex((m) => m.id === messageId);
    if (i < 0) return 0;
    const drop = new Set(own.slice(i).map((m) => m.id));
    const keptTasks = new Set(own.slice(0, i).map((m) => m.taskId).filter(Boolean));
    this.msgs = this.msgs.filter((m) => !drop.has(m.id));
    this.srcs = this.srcs.filter((s) => s.conversationId !== conversationId || !s.taskId || keptTasks.has(s.taskId));
    this.save();
    return drop.size;
  }
  async listSources(conversationId: string) {
    return this.srcs.filter((s) => s.conversationId === conversationId).sort((a, b) => a.n - b.n);
  }
  async upsertSource(conversationId: string, taskId: string | null, s: Omit<Source, 'n'>) {
    const key = s.url.replace(/#.*$/, '').replace(/\/$/, '');
    const existing = this.srcs.find((x) => x.conversationId === conversationId && x.url.replace(/#.*$/, '').replace(/\/$/, '') === key);
    if (existing) {
      Object.assign(existing, { fetchedAt: s.fetchedAt, contentHash: s.contentHash, snippet: s.snippet, status: s.status, flags: s.flags, title: s.title || existing.title });
      this.save();
      return existing;
    }
    const n = Math.max(0, ...this.srcs.filter((x) => x.conversationId === conversationId).map((x) => x.n)) + 1;
    const rec: SourceRec = { ...s, n, id: uid(), conversationId, taskId };
    this.srcs.push(rec);
    this.save();
    return rec;
  }
  async setSourceUsage(conversationId: string, n: number, usedFor: string) {
    const s = this.srcs.find((x) => x.conversationId === conversationId && x.n === n);
    if (s) s.usedFor = usedFor;
    this.save();
  }
  async addAttachment(a: Omit<AttachmentRec, 'id' | 'createdAt'>) {
    const rec: AttachmentRec = { ...a, id: uid(), createdAt: nowIso() };
    this.atts.push(rec);
    this.save();
    return rec;
  }
  async listAttachments(conversationId?: string | null) {
    return this.atts.filter((a) => conversationId === undefined || a.conversationId === conversationId);
  }
  async getAttachment(id: string) {
    return this.atts.find((a) => a.id === id) ?? null;
  }
  async updateAttachment(id: string, patch: Partial<Pick<AttachmentRec, 'scope' | 'conversationId'>>) {
    const a = this.atts.find((x) => x.id === id);
    if (a) Object.assign(a, patch);
    this.save();
    return a ?? null;
  }
  async deleteAttachment(id: string) {
    this.atts = this.atts.filter((a) => a.id !== id);
    this.save();
  }
  async getPrefs() {
    return this.prefs;
  }
  async savePrefs(settings: Record<string, unknown>) {
    this.prefs = settings;
    this.save();
  }
  async listMissions() {
    return [...this.missions];
  }
  async saveMission(m: Mission) {
    const i = this.missions.findIndex((x) => x.id === m.id);
    if (i >= 0) this.missions[i] = m;
    else this.missions.push(m);
    this.save();
  }
  async recordUsage(kind: string) {
    this.usage[kind] = (this.usage[kind] ?? 0) + 1;
  }
}
