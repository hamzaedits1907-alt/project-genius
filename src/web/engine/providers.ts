// Provider adapters for Genius Web (BYOK, called from the browser).
//
// Verified browser access (preflight checked against the real endpoints on 2026-10-19):
//   * OpenAI     — direct from the browser (CORS allowed).
//   * Anthropic  — direct, with the `anthropic-dangerous-direct-browser-access` header.
//   * Gemini     — direct, with the `x-goog-api-key` header.
//   * NVIDIA NIM — NOT allowed from a browser (no CORS headers). Requests go through the Genius relay
//                  (Netlify function) which forwards them to NVIDIA with the user's own key and stores
//                  nothing. The user must opt in; see docs/DEPLOY-NETLIFY-SUPABASE.md.
// A shared key is never used: every request carries the signed-in user's own key, held in session memory.

import type { ProviderId } from '../../lib/types';

export interface ChatMsg {
  role: 'system' | 'user' | 'assistant';
  content: string;
  /** Image data URLs (`data:image/png;base64,…`). */
  images?: string[];
}

export interface ProviderConfig {
  provider: ProviderId;
  baseUrl: string;
  apiKey: string;
  /** Present for providers that cannot be called from a browser: where to send the request instead. */
  relay?: { url: string; accessToken: () => Promise<string> };
  fetchImpl?: typeof fetch;
}

export interface CompletionResult {
  content: string;
  model: string;
  /** The model name the provider put in its response (kept apart from the requested model). */
  reportedModel?: string;
  usage?: { prompt: number; completion: number; total: number };
  finishReason?: string;
  latencyMs: number;
}

export interface CompleteOptions {
  model: string;
  signal?: AbortSignal;
  temperature?: number;
  maxTokens?: number;
  onText?: (soFar: string) => void;
  retries?: number;
}

export class ProviderError extends Error {
  constructor(
    public kind: 'auth' | 'not_found' | 'rate_limit' | 'server' | 'bad_request' | 'network' | 'cancelled' | 'cors',
    message: string,
    public status?: number,
  ) {
    super(message);
  }
}

export const DEFAULT_BASE: Record<ProviderId, string> = {
  nvidia: 'https://integrate.api.nvidia.com/v1',
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com/v1',
  gemini: 'https://generativelanguage.googleapis.com/v1beta',
  custom: '',
};

/** Whether the provider's API can be called straight from a browser page. */
export const BROWSER_DIRECT: Record<ProviderId, boolean> = { openai: true, anthropic: true, gemini: true, nvidia: false, custom: false };

export const KEY_LINK: Record<ProviderId, string> = {
  nvidia: 'https://build.nvidia.com/',
  openai: 'https://platform.openai.com/api-keys',
  anthropic: 'https://console.anthropic.com/settings/keys',
  gemini: 'https://aistudio.google.com/app/apikey',
  custom: '',
};

const trimBase = (b: string) => b.trim().replace(/\/+$/, '').replace(/\/(chat\/completions|messages|models)$/, '');

function classify(status: number, body: string): ProviderError {
  const detail = (() => {
    try {
      const j = JSON.parse(body);
      return String(j?.error?.message ?? j?.error ?? j?.message ?? '').slice(0, 300);
    } catch {
      return body.slice(0, 200);
    }
  })();
  if (status === 401 || status === 403) return new ProviderError('auth', `The provider rejected the key (HTTP ${status}). Check it in Settings.${detail ? ` ${detail}` : ''}`, status);
  if (status === 404) return new ProviderError('not_found', `The provider does not know this model or address (HTTP 404).${detail ? ` ${detail}` : ''}`, status);
  if (status === 429) return new ProviderError('rate_limit', `The provider is rate limiting this key (HTTP 429).${detail ? ` ${detail}` : ''}`, status);
  if (status >= 500) return new ProviderError('server', `The provider had a server error (HTTP ${status}).${detail ? ` ${detail}` : ''}`, status);
  return new ProviderError('bad_request', `The provider refused the request (HTTP ${status}).${detail ? ` ${detail}` : ''}`, status);
}

/** Remove reasoning blocks some models stream before their answer. */
export function stripThinking(text: string): string {
  return text.replace(/<think>[\s\S]*?<\/think>\s*/g, '').replace(/^[\s\S]*?<\/think>\s*/, (m) => (m.includes('<think>') ? '' : m));
}

async function* sseLines(res: Response, signal?: AbortSignal): AsyncGenerator<string> {
  if (!res.body) return;
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let buf = '';
  try {
    for (;;) {
      if (signal?.aborted) throw new ProviderError('cancelled', 'Stopped.');
      const { done, value } = await reader.read();
      if (done) break;
      buf += dec.decode(value, { stream: true });
      let nl: number;
      while ((nl = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, nl).replace(/\r$/, '');
        buf = buf.slice(nl + 1);
        if (line.startsWith('data:')) yield line.slice(5).trim();
      }
    }
    if (buf.startsWith('data:')) yield buf.slice(5).trim();
  } finally {
    reader.releaseLock();
  }
}

// ---------------------------------------------------------------------------------------- request bodies

const dataUrlParts = (u: string) => {
  const m = /^data:([^;]+);base64,(.*)$/.exec(u);
  return m ? { mime: m[1], data: m[2] } : null;
};

export function openAiBody(cfg: ProviderConfig, messages: ChatMsg[], o: CompleteOptions) {
  const modern = cfg.provider === 'openai' && /^(o\d|gpt-5)/.test(o.model);
  const body: Record<string, unknown> = {
    model: o.model,
    stream: true,
    stream_options: { include_usage: true },
    messages: messages.map((m) =>
      m.images?.length ? { role: m.role, content: [{ type: 'text', text: m.content }, ...m.images.map((u) => ({ type: 'image_url', image_url: { url: u } }))] } : { role: m.role, content: m.content },
    ),
  };
  if (!modern && o.temperature !== undefined) body.temperature = o.temperature;
  if (o.maxTokens) body[modern ? 'max_completion_tokens' : 'max_tokens'] = o.maxTokens;
  return body;
}

export function anthropicBody(messages: ChatMsg[], o: CompleteOptions) {
  const system = messages.filter((m) => m.role === 'system').map((m) => m.content).join('\n\n');
  const turns: { role: 'user' | 'assistant'; content: unknown[] }[] = [];
  for (const m of messages.filter((x) => x.role !== 'system')) {
    const parts: unknown[] = [];
    for (const u of m.images ?? []) {
      const p = dataUrlParts(u);
      if (p) parts.push({ type: 'image', source: { type: 'base64', media_type: p.mime, data: p.data } });
    }
    parts.push({ type: 'text', text: m.content || ' ' });
    const last = turns[turns.length - 1];
    if (last && last.role === m.role) last.content.push(...parts);
    else turns.push({ role: m.role as 'user' | 'assistant', content: parts });
  }
  if (turns[0]?.role === 'assistant') turns.unshift({ role: 'user', content: [{ type: 'text', text: '(conversation start)' }] });
  const body: Record<string, unknown> = { model: o.model, max_tokens: o.maxTokens ?? 8192, stream: true, messages: turns };
  if (system) body.system = system;
  if (o.temperature !== undefined) body.temperature = o.temperature;
  return body;
}

export function geminiBody(messages: ChatMsg[], o: CompleteOptions) {
  const system = messages.filter((m) => m.role === 'system').map((m) => m.content).join('\n\n');
  const contents: { role: 'user' | 'model'; parts: unknown[] }[] = [];
  for (const m of messages.filter((x) => x.role !== 'system')) {
    const parts: unknown[] = [{ text: m.content || ' ' }];
    for (const u of m.images ?? []) {
      const p = dataUrlParts(u);
      if (p) parts.push({ inlineData: { mimeType: p.mime, data: p.data } });
    }
    const role = m.role === 'assistant' ? 'model' : 'user';
    const last = contents[contents.length - 1];
    if (last && last.role === role) last.parts.push(...parts);
    else contents.push({ role, parts });
  }
  const body: Record<string, unknown> = { contents, generationConfig: { ...(o.temperature !== undefined ? { temperature: o.temperature } : {}), ...(o.maxTokens ? { maxOutputTokens: o.maxTokens } : {}) } };
  if (system) body.systemInstruction = { parts: [{ text: system }] };
  return body;
}

// ---------------------------------------------------------------------------------------- transport

async function send(cfg: ProviderConfig, url: string, headers: Record<string, string>, body: unknown, signal?: AbortSignal, method: 'POST' | 'GET' = 'POST'): Promise<Response> {
  const f = cfg.fetchImpl ?? fetch;
  let target = url;
  let h = headers;
  if (cfg.relay) {
    // Browser-blocked providers: the relay receives the target and the user's key, forwards, and keeps nothing.
    h = { 'content-type': 'application/json', authorization: `Bearer ${await cfg.relay.accessToken()}`, 'x-genius-target': url, 'x-genius-key': cfg.apiKey, 'x-genius-method': method };
    target = cfg.relay.url;
  }
  try {
    return await f(target, { method: cfg.relay ? 'POST' : method, headers: h, body: method === 'POST' ? JSON.stringify(body) : undefined, signal });
  } catch (e) {
    if ((e as Error)?.name === 'AbortError' || signal?.aborted) throw new ProviderError('cancelled', 'Stopped.');
    throw new ProviderError(cfg.relay ? 'network' : 'cors', cfg.relay ? 'Could not reach the Genius relay. Check your connection and try again.' : 'The browser could not reach the provider (network error or the provider blocks browser requests).');
  }
}

/** One completion, streamed; `onText` receives the answer so far. Retries rate limits and server errors. */
export async function complete(cfg: ProviderConfig, messages: ChatMsg[], o: CompleteOptions): Promise<CompletionResult> {
  const retries = o.retries ?? 2;
  for (let attempt = 0; ; attempt++) {
    try {
      return await completeOnce(cfg, messages, o);
    } catch (e) {
      if (e instanceof ProviderError && (e.kind === 'rate_limit' || e.kind === 'server') && attempt < retries && !o.signal?.aborted) {
        await new Promise((r) => setTimeout(r, 800 * 2 ** attempt));
        continue;
      }
      throw e;
    }
  }
}

async function completeOnce(cfg: ProviderConfig, messages: ChatMsg[], o: CompleteOptions): Promise<CompletionResult> {
  const base = trimBase(cfg.baseUrl || DEFAULT_BASE[cfg.provider]);
  const started = performance.now();
  let url: string;
  let headers: Record<string, string>;
  let body: unknown;
  if (cfg.provider === 'anthropic') {
    url = `${base}/messages`;
    headers = { 'content-type': 'application/json', 'x-api-key': cfg.apiKey, 'anthropic-version': '2023-06-01', 'anthropic-dangerous-direct-browser-access': 'true' };
    body = anthropicBody(messages, o);
  } else if (cfg.provider === 'gemini') {
    url = `${base}/models/${o.model.replace(/^models\//, '')}:streamGenerateContent?alt=sse`;
    headers = { 'content-type': 'application/json', 'x-goog-api-key': cfg.apiKey };
    body = geminiBody(messages, o);
  } else {
    url = `${base}/chat/completions`;
    headers = { 'content-type': 'application/json', authorization: `Bearer ${cfg.apiKey}` };
    body = openAiBody(cfg, messages, o);
  }
  const res = await send(cfg, url, headers, body, o.signal);
  if (!res.ok) throw classify(res.status, await res.text().catch(() => ''));

  let text = '';
  let reported: string | undefined;
  let finish: string | undefined;
  let usage: CompletionResult['usage'];
  const emit = () => o.onText?.(stripThinking(text));
  for await (const data of sseLines(res, o.signal)) {
    if (!data || data === '[DONE]') continue;
    let j: any; // eslint-disable-line @typescript-eslint/no-explicit-any
    try {
      j = JSON.parse(data);
    } catch {
      continue;
    }
    if (cfg.provider === 'anthropic') {
      if (j.type === 'message_start') {
        reported = j.message?.model ?? reported;
        const u = j.message?.usage;
        if (u) usage = { prompt: u.input_tokens ?? 0, completion: u.output_tokens ?? 0, total: (u.input_tokens ?? 0) + (u.output_tokens ?? 0) };
      } else if (j.type === 'content_block_delta' && j.delta?.type === 'text_delta') {
        text += j.delta.text ?? '';
        emit();
      } else if (j.type === 'message_delta') {
        finish = j.delta?.stop_reason ?? finish;
        if (j.usage?.output_tokens !== undefined) usage = { prompt: usage?.prompt ?? 0, completion: j.usage.output_tokens, total: (usage?.prompt ?? 0) + j.usage.output_tokens };
      } else if (j.type === 'error') {
        throw new ProviderError('server', String(j.error?.message ?? 'The provider reported an error while streaming.'));
      }
    } else if (cfg.provider === 'gemini') {
      for (const p of j.candidates?.[0]?.content?.parts ?? []) if (typeof p.text === 'string' && !p.thought) text += p.text;
      finish = j.candidates?.[0]?.finishReason ?? finish;
      reported = j.modelVersion ?? reported;
      if (j.usageMetadata) usage = { prompt: j.usageMetadata.promptTokenCount ?? 0, completion: j.usageMetadata.candidatesTokenCount ?? 0, total: j.usageMetadata.totalTokenCount ?? 0 };
      emit();
    } else {
      reported = j.model ?? reported;
      const ch = j.choices?.[0];
      if (ch?.delta?.content) {
        text += ch.delta.content;
        emit();
      }
      finish = ch?.finish_reason ?? finish;
      if (j.usage) usage = { prompt: j.usage.prompt_tokens ?? 0, completion: j.usage.completion_tokens ?? 0, total: j.usage.total_tokens ?? 0 };
    }
  }
  const content = stripThinking(text).trim();
  if (!content) throw new ProviderError('server', 'The provider returned an empty answer.');
  return { content, model: o.model, reportedModel: reported && reported !== o.model ? reported : reported, usage, finishReason: finish, latencyMs: Math.round(performance.now() - started) };
}

// ---------------------------------------------------------------------------------------- models

export interface ModelEntry {
  id: string;
  owner: string | null;
  created: number | null;
}

export async function listModels(cfg: ProviderConfig, signal?: AbortSignal): Promise<ModelEntry[]> {
  const base = trimBase(cfg.baseUrl || DEFAULT_BASE[cfg.provider]);
  let headers: Record<string, string>;
  if (cfg.provider === 'anthropic') headers = { 'x-api-key': cfg.apiKey, 'anthropic-version': '2023-06-01', 'anthropic-dangerous-direct-browser-access': 'true' };
  else if (cfg.provider === 'gemini') headers = { 'x-goog-api-key': cfg.apiKey };
  else headers = { authorization: `Bearer ${cfg.apiKey}` };
  const res = await send(cfg, `${base}/models${cfg.provider === 'gemini' ? '?pageSize=200' : cfg.provider === 'anthropic' ? '?limit=100' : ''}`, headers, undefined, signal, 'GET');
  if (!res.ok) throw classify(res.status, await res.text().catch(() => ''));
  const j = (await res.json()) as any; // eslint-disable-line @typescript-eslint/no-explicit-any
  if (cfg.provider === 'gemini') {
    return (j.models ?? []).filter((m: any) => (m.supportedGenerationMethods ?? []).includes('generateContent')).map((m: any) => ({ id: String(m.name).replace(/^models\//, ''), owner: 'google', created: null })); // eslint-disable-line @typescript-eslint/no-explicit-any
  }
  return (j.data ?? []).map((m: any) => ({ id: String(m.id), owner: m.owned_by ?? m.type ?? null, created: typeof m.created === 'number' ? m.created : null })); // eslint-disable-line @typescript-eslint/no-explicit-any
}
