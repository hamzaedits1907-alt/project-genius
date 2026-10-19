// The Genius Web runtime: implements the same command names the desktop UI calls through Tauri
// (`get_settings`, `start_chat`, …) on top of the browser engine, the store and the user's own provider
// key. The shared interface code therefore runs unchanged in the browser. Commands that need a local
// computer (projects, git, terminal, updates) fail with an honest "desktop only" error.

import type { AgentStep, ChatConversation, ChatRequest, DiagReport, DiagStep, MessageRow, ModelList, ProbeResult, ProviderCapabilities, ProviderId, ResolvedModels, ResearchSource, Settings, TaskInfo } from '../lib/types';
import { runTask, type EngineEnv, type WebToolsApi } from './engine/chat';
import { parseFile, SUPPORTED_EXTENSIONS } from './engine/files';
import { BROWSER_DIRECT, complete, DEFAULT_BASE, KEY_LINK, listModels, ProviderError, type ProviderConfig } from './engine/providers';
import { isVisionModel, rank, type Effort, type Role } from './engine/router';
import type { AttachmentRec, ConversationRec, MessageRec, Mode, WebStore } from './store';
import { MissionRuntime } from './missions';

export const WORKSPACE_ID = 'genius-web';

/** The Code workspace of Genius Web: a code assistant that cannot touch the user's computer. */
export const CODE_SYSTEM = `CODE WORKSPACE (browser): you help the user write, review, debug and explain code. Put every complete file in its own fenced code block with the language, and as the first line inside the block a comment naming the file path (for example // src/App.tsx or # app.py). When the user wants a web page, produce ONE self-contained HTML file (inline CSS and JavaScript) so it can be previewed in the workspace. You cannot run, build or test anything here: say plainly which parts you did not run, and never claim code works because you wrote it.`;
export const DESKTOP_ONLY = (what: string) => ({ kind: 'unsupported', message: `${what} is only available in Genius Desktop. A browser cannot access your computer's files, terminal or Git.` });

type Listener = (payload: unknown) => void;

export interface RuntimeOptions {
  store: WebStore;
  tools: WebToolsApi;
  /** Supabase access token for the relay and the web proxy (null in local-only mode). */
  accessToken: () => Promise<string>;
  relayUrl: string;
  /** Where the user's provider keys live for this browser session. */
  keys: KeyVault;
  fetchImpl?: typeof fetch;
  completeFn?: typeof complete;
  listModelsFn?: typeof listModels;
  now?: () => number;
}

export interface KeyVault {
  get(p: ProviderId): string | null;
  set(p: ProviderId, key: string): void;
  remove(p: ProviderId): void;
}

/** Keys are kept for this tab's session only (sessionStorage): closing the tab forgets them. Never sent to Supabase. */
export function sessionKeyVault(): KeyVault {
  const k = (p: ProviderId) => `genius.key.${p}`;
  const mem = new Map<string, string>();
  return {
    get: (p) => {
      try {
        return sessionStorage.getItem(k(p)) ?? mem.get(k(p)) ?? null;
      } catch {
        return mem.get(k(p)) ?? null;
      }
    },
    set: (p, key) => {
      mem.set(k(p), key);
      try {
        sessionStorage.setItem(k(p), key);
      } catch {
        /* memory only */
      }
    },
    remove: (p) => {
      mem.delete(k(p));
      try {
        sessionStorage.removeItem(k(p));
      } catch {
        /* nothing stored */
      }
    },
  };
}

export const defaultSettings = (): Settings => ({
  baseUrl: DEFAULT_BASE.openai,
  model: null,
  modelMode: 'auto',
  effort: 'max',
  thinking: true,
  temperature: 0.2,
  maxTokens: 8192,
  contextBudgetChars: 60_000,
  workspaceDir: null,
  approvals: { applyAiEdits: 'ask', dependencyInstall: 'ask', gitCommit: 'ask', gitPush: 'ask' },
  verifyAfterApply: true,
  maxRepairAttempts: 3,
  theme: 'dark',
  onboardingDone: false,
  uiMode: 'simple',
  autoCheckUpdates: false,
  internetAccess: true,
  assistantName: 'Genius',
  productCreator: 'Hamza',
  provider: 'openai',
  limits: { maxModelCalls: 100, maxToolCalls: 150, maxMinutes: 45, maxTokens: 0, maxConcurrentTasks: 3, maxProviderRetries: 2 },
  relayConsent: false,
});

const PROVIDER_LABEL: Record<ProviderId, string> = { nvidia: 'NVIDIA NIM', openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Google Gemini', custom: 'OpenAI-compatible endpoint' };
const categoryOf = (id: string) => (/embed|rerank|guard|safety|whisper|tts|asr|clip|ocr/i.test(id) ? 'embedding' : /code|coder/i.test(id) ? 'code' : 'chat');
const uid = () => crypto.randomUUID();
const iso = (ms: number) => new Date(ms).toISOString();

export class WebRuntime {
  private listeners = new Map<string, Set<Listener>>();
  private settings: Settings = defaultSettings();
  private loaded = false;
  private models: { id: string; owner: string | null; created: number | null }[] | null = null;
  private modelsAt: string | null = null;
  private tasks = new Map<string, TaskInfo & { abort: AbortController }>();
  private steps = new Map<string, AgentStep[]>();
  private steering = new Map<string, string[]>();
  private streams = new Map<string, string>();
  readonly missions: MissionRuntime;

  constructor(public opts: RuntimeOptions) {
    this.missions = new MissionRuntime(this);
  }

  // ------------------------------------------------------------------ events
  on(channel: string, cb: Listener): () => void {
    if (!this.listeners.has(channel)) this.listeners.set(channel, new Set());
    this.listeners.get(channel)!.add(cb);
    return () => this.listeners.get(channel)?.delete(cb);
  }
  emit = (channel: string, payload: unknown) => {
    for (const cb of [...(this.listeners.get(channel) ?? [])]) {
      try {
        cb(payload);
      } catch (e) {
        console.error('listener failed', channel, e);
      }
    }
  };
  private now = () => (this.opts.now ? this.opts.now() : Date.now());

  // ------------------------------------------------------------------ settings and provider
  async load(): Promise<Settings> {
    if (!this.loaded) {
      const saved = await this.opts.store.getPrefs().catch(() => null);
      this.settings = { ...defaultSettings(), ...(saved as Partial<Settings> | null) };
      this.settings.provider = (['nvidia', 'openai', 'anthropic', 'gemini', 'custom'] as const).includes(this.settings.provider) ? this.settings.provider : 'openai';
      this.loaded = true;
    }
    return this.settings;
  }
  get cfg(): Settings {
    return this.settings;
  }
  async saveSettings(s: Settings): Promise<Settings> {
    const prevProvider = this.settings.provider;
    const next: Settings = { ...defaultSettings(), ...s };
    if (next.assistantName.trim().length < 1 || next.assistantName.trim().length > 40) throw { kind: 'validation', message: 'assistant name must be 1 to 40 characters' };
    if (next.productCreator.length > 80) throw { kind: 'validation', message: 'product creator must be at most 80 characters' };
    if (next.provider === 'custom' && next.baseUrl && !/^https:\/\//i.test(next.baseUrl.trim())) throw { kind: 'validation', message: 'the endpoint address must start with https://' };
    if (next.provider !== prevProvider) {
      next.baseUrl = DEFAULT_BASE[next.provider] || next.baseUrl;
      this.models = null;
    } else if (next.baseUrl !== this.settings.baseUrl) this.models = null;
    this.settings = next;
    await this.opts.store.savePrefs({ ...next } as unknown as Record<string, unknown>);
    return next;
  }
  keyStatus() {
    const key = this.opts.keys.get(this.settings.provider);
    return { stored: !!key, hint: key ? `…${key.slice(-4)}` : null, backend: 'this browser tab (session only)', error: null };
  }
  providerConfig(): ProviderConfig {
    const s = this.settings;
    const key = this.opts.keys.get(s.provider);
    if (!key) throw { kind: 'no_key', message: `Add your ${PROVIDER_LABEL[s.provider]} API key in Settings first. It stays in this browser tab.` };
    const direct = BROWSER_DIRECT[s.provider];
    if (!direct && !s.relayConsent) throw { kind: 'relay_consent', message: `${PROVIDER_LABEL[s.provider]} does not allow calls straight from a browser. In Settings you can allow Genius Web to pass your requests through its relay (it forwards them to the provider with your key and stores nothing).` };
    return { provider: s.provider, baseUrl: s.baseUrl || DEFAULT_BASE[s.provider], apiKey: key, relay: direct ? undefined : { url: this.opts.relayUrl, accessToken: this.opts.accessToken }, fetchImpl: this.opts.fetchImpl };
  }
  async modelList(refresh: boolean): Promise<ModelList> {
    if (!this.models || refresh) {
      const lister = this.opts.listModelsFn ?? listModels;
      this.models = await lister(this.providerConfig());
      this.modelsAt = iso(this.now());
    }
    return { models: this.models.map((m) => ({ ...m, inferredCategory: categoryOf(m.id) })), fetchedAt: this.modelsAt, source: 'live', stale: false, baseUrl: this.settings.baseUrl, probes: {} };
  }
  /** Models to try for a job, best first. */
  async resolveModels(role: Role, effort: Effort): Promise<string[]> {
    const s = this.settings;
    const chosen = s.model?.trim() || null;
    if (s.modelMode === 'manual' && chosen) {
      if (role !== 'vision' || isVisionModel(chosen)) return [chosen];
    }
    if (s.provider === 'custom') return role === 'vision' ? (chosen && isVisionModel(chosen) ? [chosen] : []) : chosen ? [chosen] : [];
    let ids: string[] = [];
    try {
      ids = (await this.modelList(false)).models.filter((m) => m.inferredCategory !== 'embedding').map((m) => m.id);
    } catch (e) {
      if (chosen) return role === 'vision' && !isVisionModel(chosen) ? [] : [chosen];
      throw e;
    }
    const ranked = rank(ids, role, effort);
    if (chosen && !ranked.includes(chosen) && (role !== 'vision' || isVisionModel(chosen))) ranked.push(chosen);
    return ranked;
  }

  // ------------------------------------------------------------------ conversations
  private toConv(c: ConversationRec & { snippet?: string; messageCount?: number }): ChatConversation {
    return { id: c.id, projectId: WORKSPACE_ID, title: c.title, createdAt: c.createdAt, updatedAt: c.updatedAt, messageCount: c.messageCount ?? 0, mode: c.mode === 'research' ? 'research' : 'chat', snippet: c.snippet ?? '', origin: c.origin as ChatConversation['origin'] };
  }
  private toRow(m: MessageRec): MessageRow {
    return { id: m.id, projectId: WORKSPACE_ID, taskId: m.taskId, role: m.role, content: m.content, createdAt: m.createdAt, conversationId: m.conversationId, kind: m.kind, meta: m.meta };
  }
  private toSource(s: Awaited<ReturnType<WebStore['listSources']>>[number]): ResearchSource {
    return { id: s.id, conversationId: s.conversationId, taskId: s.taskId, n: s.n, url: s.url, requestedUrl: s.requestedUrl, title: s.title, status: s.status, fetchedAt: s.fetchedAt, contentHash: s.contentHash, snippet: s.snippet, usedFor: s.usedFor, official: s.official, flags: s.flags };
  }
  private toAttachment(a: AttachmentRec) {
    return { id: a.id, projectId: WORKSPACE_ID, conversationId: a.conversationId, name: a.name, kind: a.kind, mime: a.mime, size: a.size, scope: a.scope, status: a.status, error: a.error, chars: a.text.length, meta: a.meta, createdAt: a.createdAt };
  }

  private engineEnv(): EngineEnv {
    return {
      store: this.opts.store,
      provider: this.providerConfig(),
      settings: { assistantName: this.settings.assistantName, productCreator: this.settings.productCreator, internetAccess: this.settings.internetAccess, temperature: this.settings.temperature, maxTokens: this.settings.maxTokens, limits: this.settings.limits },
      tools: this.opts.tools,
      workspaceId: WORKSPACE_ID,
      resolveModels: (r, e) => this.resolveModels(r, e),
      emit: (c, p) => {
        if (c === 'genius://agent-step') {
          const st = p as AgentStep;
          this.steps.set(st.taskId, [...(this.steps.get(st.taskId) ?? []), st]);
        }
        if (c === 'genius://stream') this.streams.set((p as { taskId: string }).taskId, (p as { text: string }).text);
        this.emit(c, p);
      },
      drainSteering: (t) => {
        const q = this.steering.get(t) ?? [];
        this.steering.set(t, []);
        return q;
      },
      completeFn: this.opts.completeFn,
      now: this.opts.now,
    };
  }

  /** Create and run one Chat/Research task. Returns at once; progress arrives as events. */
  async startChat(conversationId: string, req: ChatRequest): Promise<TaskInfo> {
    await this.load();
    if ([...this.tasks.values()].some((t) => t.kind === 'chat' || t.kind === 'research')) throw { kind: 'busy', message: 'Genius is still answering. Wait for it to finish or stop it.' };
    const conv = await this.opts.store.getConversation(conversationId);
    if (!conv || conv.mode === 'agent') throw { kind: 'not_found', message: 'That conversation does not exist.' };
    const role = conv.mode === 'research' ? 'research' : 'chat';
    if (role === 'research' && !this.settings.internetAccess) throw { kind: 'validation', message: 'Research needs internet access. Turn it on in Settings.' };
    const env = this.engineEnv(); // fails early (and clearly) without a key or relay consent
    const attachments: AttachmentRec[] = [];
    for (const id of req.attachmentIds ?? []) {
      const a = await this.opts.store.getAttachment(id);
      if (a && a.status === 'ready') attachments.push(a);
    }
    const userMsg = await this.opts.store.addMessage({ conversationId, role: 'user', kind: 'chat', content: req.message, taskId: null, meta: { attachments: attachments.map((a) => ({ id: a.id, name: a.name, kind: a.kind })), mode: conv.mode, effort: req.effort ?? null } });
    if (conv.title === 'New conversation') await this.opts.store.renameConversation(conversationId, req.message.split('\n')[0].slice(0, 60));
    const taskId = uid();
    const startedAt = iso(this.now());
    const first = req.message.split('\n')[0].slice(0, 80);
    const info: TaskInfo = { id: taskId, projectId: WORKSPACE_ID, kind: role, title: `${role === 'research' ? 'Research' : 'Chat'}: ${first}`, startedAt };
    const abort = new AbortController();
    this.tasks.set(taskId, { ...info, abort });
    this.activity(info, 'running');
    const effort = (req.effort ?? 'balanced') as Effort;
    void this.opts.store.recordUsage(role);
    void (async () => {
      let status: 'completed' | 'failed' | 'cancelled' = 'completed';
      try {
        const r = await runTask(env, { taskId, conv, role, message: req.message, attachments, effort, userMessageId: userMsg.id, extraSystem: conv.origin?.flavor === 'code' ? CODE_SYSTEM : undefined }, abort.signal);
        await this.opts.store.addMessage({ conversationId, role: 'assistant', kind: 'answer', content: r.answer, taskId, meta: r.meta });
      } catch (e) {
        const cancelled = abort.signal.aborted || (e instanceof ProviderError && e.kind === 'cancelled');
        status = cancelled ? 'cancelled' : 'failed';
        const msg = cancelled ? 'Stopped.' : `I stopped because of an error: ${e instanceof Error ? e.message : (e as { message?: string })?.message ?? String(e)}`;
        await this.opts.store.addMessage({ conversationId, role: 'assistant', kind: 'error', content: msg, taskId, meta: { report: { lifecycle: 'failed', warnings: [], error: msg } } }).catch(() => undefined);
      } finally {
        this.tasks.delete(taskId);
        this.streams.delete(taskId);
        this.activity(info, status);
      }
    })();
    return info;
  }
  private activity(t: TaskInfo, status: 'running' | 'completed' | 'failed' | 'cancelled') {
    const ts = iso(this.now());
    this.emit('genius://activity', { id: `${t.id}-${status}`, projectId: WORKSPACE_ID, taskId: t.id, kind: 'task', status, title: t.title, detail: { taskKind: t.kind }, ts, updatedTs: ts });
  }
  cancel(taskId: string) {
    this.tasks.get(taskId)?.abort.abort();
    this.missions.cancelTask(taskId);
  }
  steer(taskId: string, message: string) {
    if (!this.tasks.has(taskId)) throw { kind: 'not_found', message: 'That task is no longer running.' };
    this.steering.set(taskId, [...(this.steering.get(taskId) ?? []), message]);
  }
  activeTasks(): TaskInfo[] {
    return [...this.tasks.values()].map(({ abort: _a, ...t }) => t);
  }
  taskSteps(taskId: string) {
    return this.steps.get(taskId) ?? [];
  }
  engineEnvFor() {
    return this.engineEnv();
  }

  // ------------------------------------------------------------------ the command table
  async invoke(cmd: string, a: Record<string, any> = {}): Promise<unknown> { // eslint-disable-line @typescript-eslint/no-explicit-any
    const st = this.opts.store;
    await this.load();
    switch (cmd) {
      case 'get_settings':
        return this.settings;
      case 'save_settings':
        return this.saveSettings(a.settings);
      case 'key_status':
        return this.keyStatus();
      case 'save_api_key': {
        const key = String(a.key ?? '').trim();
        if (key.length < 8 || /\s/.test(key)) throw { kind: 'validation', message: 'That does not look like an API key.' };
        this.opts.keys.set(this.settings.provider, key);
        this.models = null;
        return this.keyStatus();
      }
      case 'remove_api_key':
        this.opts.keys.remove(this.settings.provider);
        this.models = null;
        return this.keyStatus();
      case 'list_models':
        return this.modelList(!!a.refresh);
      case 'select_model':
        return this.saveSettings({ ...this.settings, model: String(a.model), modelMode: 'manual' });
      case 'resolved_models': {
        const out: ResolvedModels = { agent: [], code: [], plan: [], subagent: [], quick: [], vision: [] };
        try {
          out.agent = out.code = out.plan = out.subagent = await this.resolveModels('agent', 'max');
          out.quick = await this.resolveModels('quick', 'fast');
          out.vision = await this.resolveModels('vision', 'balanced');
        } catch {
          /* not connected yet */
        }
        return out;
      }
      case 'probe_model':
        return this.probe(String(a.model));
      case 'test_connection': {
        const l = await this.modelList(true);
        return { ok: true, models: l.models.length, provider: this.settings.provider };
      }
      case 'diagnose_provider':
        return this.diagnose();
      case 'provider_capabilities':
        return this.capabilities();
      case 'usage_summary':
        return { promptTokens: 0, completionTokens: 0, totalTokens: 0, calls: 0, byModel: {} };
      case 'app_info':
        return { version: 'web', dataDir: 'browser', workspaceDir: 'browser', secretBackend: 'browser session', platform: 'web' };
      case 'app_version':
        return 'web';
      case 'check_for_update':
        return { currentVersion: 'web', latestVersion: 'web', available: false, notes: '', publishedAt: null, releaseUrl: null, asset: null, detail: 'Genius Web is always up to date: reload the page to get the newest version.' };
      case 'detect_tools':
      case 'list_projects':
      case 'pending_approvals':
      case 'list_activity':
      case 'list_commands':
      case 'list_runs':
      case 'active_runs':
      case 'list_memories':
        return [];
      case 'supported_import_types':
        return SUPPORTED_EXTENSIONS;
      case 'record_ui_error':
        return 'logged in the browser console only';
      case 'chat_workspace':
        return { id: WORKSPACE_ID, name: 'Genius Web', description: '', path: '', kind: 'chat', spec: null, notes: '', status: {}, createdAt: iso(0), updatedAt: iso(0), lastOpenedAt: null };
      case 'list_chat_conversations':
        return (await st.listConversations(a.mode ?? null, a.query ?? undefined)).filter((c) => c.mode !== 'agent').map((c) => this.toConv(c));
      case 'web_create_code_session':
        return this.toConv(await st.createConversation('chat', 'New conversation', { flavor: 'code' }));
      case 'create_chat_conversation':
        return this.toConv(await st.createConversation(a.mode === 'research' ? 'research' : 'chat', a.title ?? 'New conversation'));
      case 'rename_conversation':
        return st.renameConversation(a.conversationId, a.title);
      case 'delete_conversation':
        return st.deleteConversation(a.conversationId);
      case 'conversation_messages':
        return (await st.listMessages(a.conversationId)).map((m) => this.toRow(m));
      case 'research_sources':
        return (await st.listSources(a.conversationId)).map((s) => this.toSource(s));
      case 'start_chat':
        return this.startChat(a.conversationId, a.request);
      case 'cancel_task':
        return this.cancel(a.taskId);
      case 'active_tasks':
        return this.activeTasks();
      case 'agent_steps':
        return this.taskSteps(a.taskId);
      case 'steer_agent':
        return this.steer(a.taskId, a.message);
      case 'rewind_conversation': {
        const removed = await st.deleteFrom(a.conversationId, a.messageId);
        return { removedMessages: removed, revertedChangesets: 0, conflicts: [] };
      }
      case 'import_attachment_data': {
        const bin = atob(String(a.dataBase64));
        const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
        const parsed = await parseFile({ name: a.name, size: bytes.length, arrayBuffer: async () => bytes.buffer.slice(0) as ArrayBuffer });
        const rec = await st.addAttachment({ conversationId: a.conversationId ?? null, scope: a.scope === 'message' ? 'message' : 'reference', ...parsed });
        return this.toAttachment(rec);
      }
      case 'list_attachments':
        return (await st.listAttachments(undefined)).map((x) => this.toAttachment(x));
      case 'attachment_preview': {
        const rec = await st.getAttachment(a.attachmentId);
        if (!rec) throw { kind: 'not_found', message: 'That file no longer exists.' };
        return { attachment: this.toAttachment(rec), preview: rec.text.slice(0, 4000), imageUrl: rec.imageDataUrl };
      }
      case 'set_attachment_scope': {
        const rec = await st.updateAttachment(a.attachmentId, { scope: a.scope });
        if (!rec) throw { kind: 'not_found', message: 'That file no longer exists.' };
        return this.toAttachment(rec);
      }
      case 'delete_attachment':
        return st.deleteAttachment(a.attachmentId);
      case 'capability_report':
        return { generatedAt: iso(this.now()), provider: { name: PROVIDER_LABEL[this.settings.provider], baseUrl: this.settings.baseUrl, model: this.settings.model, maxTokens: this.settings.maxTokens, keyStored: this.keyStatus().stored }, capabilities: [], agentTools: [], integrations: [], permissions: {}, project: null, limitations: ['Genius Web cannot access local files, a terminal or Git. Use Genius Desktop for the Code workspace.'] };
      default:
        if (this.missions.handles(cmd)) return this.missions.invoke(cmd, a);
        throw DESKTOP_ONLY(`“${cmd.replace(/_/g, ' ')}”`);
    }
  }

  private async probe(model: string): Promise<ProbeResult> {
    const cfg = this.providerConfig();
    const t0 = this.now();
    const fn = this.opts.completeFn ?? complete;
    const r = await fn(cfg, [{ role: 'user', content: 'Reply with the single word OK.' }], { model, maxTokens: 16, temperature: 0, retries: 0 });
    return { model, chat: true, streaming: true, latencyMs: this.now() - t0 || r.latencyMs, sample: r.content.slice(0, 40) };
  }

  private async diagnose(): Promise<DiagReport> {
    const s = this.settings;
    const steps: DiagStep[] = [];
    const add = (id: string, title: string, status: DiagStep['status'], detail: string, hint: string | null = null, ms: number | null = null) => steps.push({ id, title, status, detail, hint, ms });
    const key = this.opts.keys.get(s.provider);
    add('key', 'Key', key ? 'ok' : 'fail', key ? 'A key is set for this browser tab.' : 'No key is set.', key ? null : 'Paste your key in Settings → AI provider.');
    let host = '';
    try {
      host = new URL(s.baseUrl || DEFAULT_BASE[s.provider]).host;
    } catch {
      add('address', 'Address', 'fail', 'The endpoint address is not a valid URL.');
    }
    if (!BROWSER_DIRECT[s.provider]) add('browser', 'Browser access', s.relayConsent ? 'info' : 'warn', s.relayConsent ? `${PROVIDER_LABEL[s.provider]} cannot be called straight from a browser, so requests use the Genius relay (which stores nothing).` : `${PROVIDER_LABEL[s.provider]} does not allow direct browser calls. Allow the relay in Settings to use it here.`);
    else add('browser', 'Browser access', 'ok', `${PROVIDER_LABEL[s.provider]} can be called directly from the browser.`);
    let model: string | null = null;
    if (key && (BROWSER_DIRECT[s.provider] || s.relayConsent)) {
      const t0 = this.now();
      try {
        const l = await this.modelList(true);
        add('models', 'Reach and authenticate', 'ok', `${l.models.length} models are listed for this key.`, null, this.now() - t0);
        const chain = await this.resolveModels('chat', 'balanced').catch(() => []);
        model = chain[0] ?? null;
        if (!model) add('model', 'Model', 'fail', 'No usable chat model was found.', 'Choose a model in Settings.');
        else {
          add('model', 'Model', 'ok', `${model} is available to this key${s.modelMode === 'manual' ? '' : ' (chosen automatically)'}.`);
          const t1 = this.now();
          try {
            const p = await this.probe(model);
            add('message', 'Test message', 'ok', `${model} answered "${p.sample}".`, null, this.now() - t1);
          } catch (e) {
            add('message', 'Test message', 'fail', e instanceof Error ? e.message : String(e));
          }
          add('image', 'Image input', isVisionModel(model) ? 'info' : 'skipped', isVisionModel(model) ? 'This model family reads images (not tested with a picture).' : 'This model is not known to read images.');
        }
      } catch (e) {
        add('models', 'Reach and authenticate', 'fail', e instanceof Error ? e.message : String(e), e instanceof ProviderError && e.kind === 'auth' ? 'Check the key in Settings.' : null);
      }
    }
    const ok = steps.every((x) => x.status !== 'fail') && !!model;
    return { provider: s.provider, label: PROVIDER_LABEL[s.provider], integrationVerified: 'contract-tests', host, model, ok, verdict: ok ? 'The provider answers from this browser.' : (steps.find((x) => x.status === 'fail')?.detail ?? 'Not ready yet.'), steps, checkedAt: iso(this.now()) };
  }

  private capabilities(): ProviderCapabilities[] {
    const mk = (p: ProviderId, wire: string, notes: string[], sel: ProviderCapabilities['autoSelection']): ProviderCapabilities => ({ provider: p, label: PROVIDER_LABEL[p], wire, streaming: true, usageReporting: true, modelListing: true, imageInput: 'by-model', autoRouting: false, autoSelection: sel, toolProtocol: 'genius-text-tools', verification: 'contract-tests', notes: [...notes, `Get a key: ${KEY_LINK[p] || 'from your provider'}`] });
    return [
      mk('nvidia', 'openai-chat-completions', ['NVIDIA does not allow calls straight from a browser (checked: no CORS headers). Genius Web passes requests through its relay, which forwards them with your own key and stores nothing. You must allow this in Settings.', 'Not verified live from the browser.'], 'tuned'),
      mk('openai', 'openai-chat-completions', ['Called directly from your browser (the API allows it). Not verified live from Genius Web.'], 'by-family'),
      mk('anthropic', 'anthropic-messages', ['Called directly from your browser with the browser-access header Anthropic documents. Not verified live from Genius Web.'], 'by-family'),
      mk('gemini', 'gemini-generate-content', ['Called directly from your browser. Not verified live from Genius Web.'], 'by-family'),
      mk('custom', 'openai-chat-completions', ['Any OpenAI-compatible HTTPS endpoint. Requests use the relay unless the server allows browser calls. Not verified against any particular server.'], 'manual-only'),
    ];
  }

  asConversationRec(mode: Mode, id: string): Promise<ConversationRec | null> {
    void mode;
    return this.opts.store.getConversation(id);
  }
}
