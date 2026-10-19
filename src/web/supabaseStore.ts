// WebStore backed by Supabase (Postgres + Storage). Every call runs as the signed-in user, so row level
// security (supabase/migrations) is what keeps accounts apart — this code never filters by user id for
// security, only for convenience.
import type { SupabaseClient } from '@supabase/supabase-js';
import type { Mission } from '../lib/types';
import type { Source } from './engine/research';
import type { AttachmentRec, ConversationRec, MessageRec, Mode, SourceRec, WebStore } from './store';

const fail = (what: string, e: { message: string } | null): never => {
  throw new Error(`${what}: ${e?.message ?? 'unknown error'}`);
};

const conv = (r: any): ConversationRec => ({ id: r.id, mode: r.mode, title: r.title, createdAt: r.created_at, updatedAt: r.updated_at, origin: r.origin }); // eslint-disable-line @typescript-eslint/no-explicit-any
const msg = (r: any): MessageRec => ({ id: r.id, conversationId: r.conversation_id, role: r.role, kind: r.kind, content: r.content, meta: r.meta ?? {}, taskId: r.task_id, createdAt: r.created_at }); // eslint-disable-line @typescript-eslint/no-explicit-any
const src = (r: any): SourceRec => ({ id: r.id, conversationId: r.conversation_id, taskId: r.task_id, n: r.n, url: r.url, requestedUrl: r.requested_url, title: r.title, status: r.status, fetchedAt: r.fetched_at, contentHash: r.content_hash, snippet: r.snippet, usedFor: r.used_for, official: !!r.flags?.official, flags: (r.flags?.notes as string[]) ?? [] }); // eslint-disable-line @typescript-eslint/no-explicit-any

/** Transient failures (network drop, 5xx) are retried; the same ids are reused, so a retry can never duplicate a row. */
async function retrying<T>(fn: () => PromiseLike<{ data: T | null; error: { message: string; code?: string } | null }>, tries = 3) {
  let last: { message: string; code?: string } | null = null;
  for (let i = 0; i < tries; i++) {
    try {
      const r = await fn();
      if (!r.error) return r;
      last = r.error;
      if (!/fetch|network|timeout|50d|ECONN/i.test(r.error.message) || r.error.code) return r;
    } catch (e) {
      last = { message: e instanceof Error ? e.message : String(e) };
    }
    await new Promise((res) => setTimeout(res, 300 * (i + 1)));
  }
  return { data: null as T | null, error: last };
}

export class SupabaseStore implements WebStore {
  constructor(private sb: SupabaseClient) {}

  /**
   * Whether the database has the sync migration (tombstones, conversation_summaries, ...). Until the owner has
   * applied supabase/migrations/20261021000000_sync.sql the store keeps working in its earlier shape.
   */
  private v2: Promise<boolean> | null = null;
  isSyncSchema(): Promise<boolean> {
    this.v2 ??= Promise.resolve(this.sb.from('conversations').select('deleted_at').limit(0)).then((r) => !r.error);
    return this.v2;
  }

  async listConversations(mode: Mode | null, query?: string) {
    if (await this.isSyncSchema()) {
      const { data, error } = await this.sb.rpc('conversation_summaries', { p_mode: mode, p_query: query?.trim() || null, p_limit: 300 });
      if (error) fail('Could not load conversations', error);
      return ((data ?? []) as any[]).map((r) => ({ id: r.id, mode: r.mode, title: r.title, createdAt: r.created_at, updatedAt: r.updated_at, origin: r.origin, snippet: String(r.snippet ?? ''), messageCount: Number(r.message_count ?? 0) })); // eslint-disable-line @typescript-eslint/no-explicit-any
    }
    let q = this.sb.from('conversations').select('*').order('updated_at', { ascending: false }).limit(300);
    if (mode) q = q.eq('mode', mode);
    const { data, error } = await q;
    if (error) fail('Could not load conversations', error);
    let rows = (data ?? []).map(conv);
    const term = query?.trim();
    let hits = new Map<string, string>();
    if (term) {
      const like = `%${term.replace(/[%_\\]/g, (c) => `\\${c}`)}%`;
      const { data: m } = await this.sb.from('messages').select('conversation_id, content').ilike('content', like).limit(200);
      hits = new Map((m ?? []).map((x: any) => [x.conversation_id as string, x.content as string])); // eslint-disable-line @typescript-eslint/no-explicit-any
      rows = rows.filter((c) => c.title.toLowerCase().includes(term.toLowerCase()) || hits.has(c.id));
    }
    const ids = rows.map((r) => r.id);
    const counts = new Map<string, number>();
    const last = new Map<string, string>();
    if (ids.length) {
      const { data: m } = await this.sb.from('messages').select('conversation_id, content, created_at').in('conversation_id', ids).order('created_at', { ascending: false }).limit(2000);
      for (const x of (m ?? []) as any[]) { // eslint-disable-line @typescript-eslint/no-explicit-any
        counts.set(x.conversation_id, (counts.get(x.conversation_id) ?? 0) + 1);
        if (!last.has(x.conversation_id)) last.set(x.conversation_id, x.content);
      }
    }
    return rows.map((c) => ({ ...c, snippet: (hits.get(c.id) ?? last.get(c.id) ?? '').replace(/\s+/g, ' ').slice(0, 140), messageCount: counts.get(c.id) ?? 0 }));
  }
  async getConversation(id: string) {
    let q = this.sb.from('conversations').select('*').eq('id', id);
    if (await this.isSyncSchema()) q = q.is('deleted_at', null);
    const { data } = await q.maybeSingle();
    return data ? conv(data) : null;
  }
  async createConversation(mode: Mode, title: string, origin: Record<string, unknown> | null = null) {
    const id = crypto.randomUUID();
    const { data, error } = await retrying(() => this.sb.from('conversations').upsert({ id, mode, title, origin }, { onConflict: 'id', ignoreDuplicates: true }).select().maybeSingle());
    if (error) fail('Could not create the conversation', error);
    return conv(data ?? (await this.sb.from('conversations').select('*').eq('id', id).single()).data);
  }
  async renameConversation(id: string, title: string) {
    const { error } = await this.sb.from('conversations').update({ title: title.slice(0, 200) }).eq('id', id);
    if (error) fail('Could not rename the conversation', error);
  }
  async deleteConversation(id: string) {
    const files = await this.listAttachments(id);
    const paths = files.filter((f) => f.meta.imagePath).map((f) => String(f.meta.imagePath));
    if (paths.length) await this.sb.storage.from('attachments').remove(paths);
    if (await this.isSyncSchema()) {
      const { error } = await this.sb.rpc('delete_conversation', { p_id: id });
      if (error) fail('Could not delete the conversation', error);
      return;
    }
    const { error } = await this.sb.from('conversations').delete().eq('id', id);
    if (error) fail('Could not delete the conversation', error);
  }
  async listMessages(conversationId: string) {
    if (await this.isSyncSchema()) {
      // the NEWEST 1000 live messages, oldest first (a long conversation must show its end, not its beginning)
      const { data, error } = await this.sb.from('messages').select('*').eq('conversation_id', conversationId).is('deleted_at', null).order('created_at', { ascending: false }).order('seq', { ascending: false }).limit(1000);
      if (error) fail('Could not load messages', error);
      return (data ?? []).reverse().map(msg);
    }
    const { data, error } = await this.sb.from('messages').select('*').eq('conversation_id', conversationId).order('created_at', { ascending: true }).limit(1000);
    if (error) fail('Could not load messages', error);
    return (data ?? []).map(msg);
  }
  async addMessage(m: Omit<MessageRec, 'id' | 'createdAt'> & { id?: string; createdAt?: string }) {
    // The id is chosen here, so a retry after a lost response (or a double click) stores the message once.
    const row: Record<string, unknown> = { id: m.id ?? crypto.randomUUID(), conversation_id: m.conversationId, role: m.role, kind: m.kind, content: m.content, meta: m.meta, task_id: m.taskId };
    if (m.createdAt) row.created_at = m.createdAt;
    const { data, error } = await retrying(() => this.sb.from('messages').upsert(row, { onConflict: 'id', ignoreDuplicates: true }).select().maybeSingle());
    if (error) fail('Could not save the message', error);
    return msg(data ?? (await this.sb.from('messages').select('*').eq('id', row.id as string).single()).data);
  }
  async deleteFrom(conversationId: string, messageId: string) {
    const all = await this.listMessages(conversationId);
    const i = all.findIndex((m) => m.id === messageId);
    if (i < 0) return 0;
    const drop = all.slice(i).map((m) => m.id);
    if (await this.isSyncSchema()) {
      const { error } = await this.sb.rpc('delete_messages_from', { p_conversation: conversationId, p_message: messageId });
      if (error) fail('Could not remove messages', error);
    } else {
      const { error } = await this.sb.from('messages').delete().in('id', drop);
      if (error) fail('Could not remove messages', error);
    }
    const keep = new Set(all.slice(0, i).map((m) => m.taskId).filter(Boolean));
    const srcs = await this.listSources(conversationId);
    const gone = srcs.filter((s) => s.taskId && !keep.has(s.taskId)).map((s) => s.id);
    if (gone.length) await this.sb.from('research_sources').delete().in('id', gone);
    return drop.length;
  }
  async listSources(conversationId: string) {
    const { data, error } = await this.sb.from('research_sources').select('*').eq('conversation_id', conversationId).order('n', { ascending: true });
    if (error) fail('Could not load sources', error);
    return (data ?? []).map(src);
  }
  async upsertSource(conversationId: string, taskId: string | null, s: Omit<Source, 'n'>) {
    const key = (u: string) => u.replace(/#.*$/, '').replace(/\/$/, '');
    const existing = (await this.listSources(conversationId)).find((x) => key(x.url) === key(s.url));
    const flags = { official: s.official, notes: s.flags };
    if (existing) {
      const { data, error } = await this.sb.from('research_sources').update({ fetched_at: s.fetchedAt, content_hash: s.contentHash, snippet: s.snippet, status: s.status, flags, title: s.title || existing.title }).eq('id', existing.id).select().single();
      if (error) fail('Could not update the source', error);
      return src(data);
    }
    const all = await this.listSources(conversationId);
    const n = Math.max(0, ...all.map((x) => x.n)) + 1;
    const { data, error } = await this.sb.from('research_sources').insert({ conversation_id: conversationId, task_id: taskId, n, url: s.url, requested_url: s.requestedUrl, title: s.title, status: s.status, fetched_at: s.fetchedAt, content_hash: s.contentHash, snippet: s.snippet, flags }).select().single();
    if (error) fail('Could not record the source', error);
    return src(data);
  }
  async setSourceUsage(conversationId: string, n: number, usedFor: string) {
    await this.sb.from('research_sources').update({ used_for: usedFor }).eq('conversation_id', conversationId).eq('n', n);
  }
  private att(r: any, imageDataUrl: string | null = null): AttachmentRec { // eslint-disable-line @typescript-eslint/no-explicit-any
    return { id: r.id, conversationId: r.conversation_id, name: r.name, kind: r.kind, mime: r.mime, size: Number(r.size), scope: r.scope, status: r.status, error: r.error, text: r.text_content ?? '', imageDataUrl, meta: { ...(r.meta ?? {}), ...(r.image_path ? { imagePath: r.image_path } : {}) }, createdAt: r.created_at };
  }
  async addAttachment(a: Omit<AttachmentRec, 'id' | 'createdAt'>) {
    const id = crypto.randomUUID();
    let imagePath: string | null = null;
    if (a.imageDataUrl) {
      const uidRes = await this.sb.auth.getUser();
      const user = uidRes.data.user?.id;
      if (!user) throw new Error('Not signed in.');
      const m = /^data:([^;]+);base64,(.*)$/.exec(a.imageDataUrl);
      if (m) {
        const bytes = Uint8Array.from(atob(m[2]), (c) => c.charCodeAt(0));
        imagePath = `${user}/${id}`;
        const { error } = await this.sb.storage.from('attachments').upload(imagePath, bytes, { contentType: m[1], upsert: false });
        if (error) fail('Could not store the image', error);
      }
    }
    const { data, error } = await this.sb.from('attachments').insert({ id, conversation_id: a.conversationId, name: a.name, kind: a.kind, mime: a.mime, size: a.size, scope: a.scope, status: a.status, error: a.error, text_content: a.text, image_path: imagePath, meta: a.meta }).select().single();
    if (error) fail('Could not save the file', error);
    return this.att(data, a.imageDataUrl);
  }
  async listAttachments(conversationId?: string | null) {
    let q = this.sb.from('attachments').select('*').order('created_at', { ascending: false }).limit(300);
    if (conversationId) q = q.eq('conversation_id', conversationId);
    const { data, error } = await q;
    if (error) fail('Could not load files', error);
    return (data ?? []).map((r) => this.att(r));
  }
  async getAttachment(id: string) {
    const { data } = await this.sb.from('attachments').select('*').eq('id', id).maybeSingle();
    if (!data) return null;
    let url: string | null = null;
    if (data.image_path) {
      const { data: blob } = await this.sb.storage.from('attachments').download(data.image_path);
      if (blob) {
        const buf = new Uint8Array(await blob.arrayBuffer());
        let bin = '';
        for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
        url = `data:${data.mime};base64,${btoa(bin)}`;
      }
    }
    return this.att(data, url);
  }
  async updateAttachment(id: string, patch: Partial<Pick<AttachmentRec, 'scope' | 'conversationId'>>) {
    const row: Record<string, unknown> = {};
    if (patch.scope) row.scope = patch.scope;
    if (patch.conversationId !== undefined) row.conversation_id = patch.conversationId;
    const { data } = await this.sb.from('attachments').update(row).eq('id', id).select().maybeSingle();
    return data ? this.att(data) : null;
  }
  async deleteAttachment(id: string) {
    const { data } = await this.sb.from('attachments').select('image_path').eq('id', id).maybeSingle();
    if (data?.image_path) await this.sb.storage.from('attachments').remove([data.image_path]);
    await this.sb.from('attachments').delete().eq('id', id);
  }
  async getPrefs() {
    const { data } = await this.sb.from('user_preferences').select('settings').maybeSingle();
    return (data?.settings as Record<string, unknown>) ?? null;
  }
  async savePrefs(settings: Record<string, unknown>) {
    const { error } = await this.sb.from('user_preferences').upsert({ settings });
    if (error) fail('Could not save preferences', error);
  }
  async listMissions() {
    const { data, error } = await this.sb.from('missions').select('data').order('updated_at', { ascending: false }).limit(100);
    if (error) fail('Could not load missions', error);
    return (data ?? []).map((r: any) => r.data as Mission); // eslint-disable-line @typescript-eslint/no-explicit-any
  }
  async saveMission(m: Mission) {
    const { error } = await this.sb.from('missions').upsert({ id: m.id, title: m.title.slice(0, 300), goal: m.goal.slice(0, 20000), status: m.status, data: m, updated_at: new Date().toISOString() });
    if (error) fail('Could not save the mission', error);
  }
  async recordUsage(kind: 'chat' | 'research' | 'agent' | 'code' | 'web_tool') {
    await this.sb.from('usage_events').insert({ kind });
  }
}
