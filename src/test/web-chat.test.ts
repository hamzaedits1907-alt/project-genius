// The Genius Web conversation loop with a scripted model and scripted web tools (no network, no keys).
// Proves the loop's behaviour — tool dialects, citations against the ledger, identity prompt, residue,
// limits — not the behaviour of any live model or provider.
import { describe, expect, it } from 'vitest';
import { runTask, type EngineEnv, type WebToolsApi } from '../web/engine/chat';
import type { ChatMsg, CompletionResult } from '../web/engine/providers';
import { MemoryStore } from '../web/store';

const PAGE = { url: 'https://react.dev/learn', status: 200, contentType: 'text/html', title: 'Learn React', text: 'Hooks let function components use state. Call Hooks only at the top level.', truncated: false };

async function setup(script: (msgs: ChatMsg[], n: number) => string, over: Partial<EngineEnv['settings']> = {}) {
  const store = new MemoryStore();
  const log: ChatMsg[][] = [];
  const events: [string, unknown][] = [];
  const web: WebToolsApi & { fetched: string[]; searched: string[] } = {
    fetched: [],
    searched: [],
    async search(q) {
      this.searched.push(q);
      return [{ title: 'Learn React', url: PAGE.url, snippet: 'Hooks' }];
    },
    async fetch(url) {
      this.fetched.push(url);
      if (url.includes('missing')) return { ...PAGE, url, status: 404, text: 'nope' };
      return { ...PAGE, url: url.startsWith('https://react.dev') ? PAGE.url : url };
    },
  };
  const env: EngineEnv = {
    store,
    provider: { provider: 'openai', baseUrl: 'https://api.openai.com/v1', apiKey: 'sk-test' },
    settings: { assistantName: 'Genius', productCreator: 'Hamza', internetAccess: true, temperature: 0.2, maxTokens: 4000, limits: { maxModelCalls: 0, maxToolCalls: 0, maxMinutes: 0, maxTokens: 0, maxProviderRetries: 0 }, ...over },
    tools: web,
    workspaceId: 'ws',
    resolveModels: async () => ['gpt-5-mini'],
    emit: (c, p) => events.push([c, p]),
    drainSteering: () => [],
    completeFn: async (_cfg, msgs): Promise<CompletionResult> => {
      log.push(msgs.map((m) => ({ ...m })));
      return { content: script(msgs, log.length - 1), model: 'gpt-5-mini', reportedModel: 'gpt-5-mini-2026', usage: { prompt: 11, completion: 7, total: 18 }, latencyMs: 5 };
    },
  };
  return { env, store, web, log, events };
}

const last = (m: ChatMsg[]) => m[m.length - 1].content;
const ac = () => new AbortController().signal;

describe('chat loop', () => {
  it('answers a greeting directly with the product identity and the configured creator, never naming a model', async () => {
    const { env, store, log } = await setup(() => "<final>I'm Genius, the AI assistant built into Project Genius.</final>");
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't1', conv, role: 'chat', message: 'Hi! Who are you?', attachments: [], effort: 'balanced' }, ac());
    expect(r.answer).toBe("I'm Genius, the AI assistant built into Project Genius.");
    expect(log).toHaveLength(1);
    const system = log[0][0].content;
    expect(system).toContain('created by Hamza');
    expect(system).toContain('never assume the user is Hamza');
    expect(system.toLowerCase()).not.toMatch(/gpt|claude|nemotron|gemini/);
    const report = r.meta.report as Record<string, unknown>;
    expect(report.model).toBe('gpt-5-mini');
    expect(report.reportedModel).toBe('gpt-5-mini-2026');
  });

  it('runs a tool-named tag with a Markdown link once and never shows the syntax', async () => {
    const { env, store, web } = await setup((_m, n) => (n === 0 ? 'Let me open it.\n<fetch_url {"url": "[the page](https://react.dev/learn)"}>' : '<final>The page explains Hooks.</final>'));
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't2', conv, role: 'chat', message: 'Summarize the page', attachments: [], effort: 'balanced' }, ac());
    expect(web.fetched).toEqual(['https://react.dev/learn']);
    expect(r.answer).toBe('The page explains Hooks.');
    expect(r.steps.map((s) => [s.tool, s.ok])).toEqual([['fetch_url', true]]);
  });

  it('corrects fabricated tool markup once, then strips what remains', async () => {
    const a = await setup((_m, n) => (n === 0 ? '<final>Found it. <tool_result name="web_search" ok="true">invented</tool_result></final>' : '<final>I could not look that up.</final>'));
    const c1 = await a.store.createConversation('chat', 'x');
    const r1 = await runTask(a.env, { taskId: 't3', conv: c1, role: 'chat', message: 'What is that site?', attachments: [], effort: 'balanced' }, ac());
    expect(r1.answer).toBe('I could not look that up.');
    expect(a.log).toHaveLength(2);
    const b = await setup(() => '<final>Found it. <tool_result name="web_search" ok="true">invented</tool_result> Great. <websearch {"query": "x"}></final>');
    const c2 = await b.store.createConversation('chat', 'x');
    const r2 = await runTask(b.env, { taskId: 't4', conv: c2, role: 'chat', message: 'What?', attachments: [], effort: 'balanced' }, ac());
    expect(r2.answer).not.toMatch(/tool_result|websearch|invented/);
  });

  it('never repeats an identical failing call endlessly', async () => {
    const { env, store, web } = await setup((_m, n) => (n < 6 ? '<tool>{"name":"fetch_url","args":{"url":"https://react.dev/missing"}}</tool>' : '<final>That page does not exist.</final>'));
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't5', conv, role: 'chat', message: 'Open it', attachments: [], effort: 'balanced' }, ac());
    expect(web.fetched.length).toBe(1);
    expect(r.answer).toContain('does not exist');
  });

  it('honours Genius\'s own task limit and says which limit stopped it', async () => {
    const { env, store } = await setup(() => '<tool>{"name":"calculate","args":{"expression":"1+1"}}</tool>', { limits: { maxModelCalls: 2, maxToolCalls: 0, maxMinutes: 0, maxTokens: 0, maxProviderRetries: 0 } });
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't6', conv, role: 'chat', message: 'Loop', attachments: [], effort: 'balanced' }, ac());
    expect(r.answer).toMatch(/model-call limit/);
    expect((r.meta.report as { lifecycle: string }).lifecycle).toBe('blocked');
  });

  it('arithmetic and table statistics come from tools', async () => {
    const { env, store } = await setup((m, n) => {
      if (n === 0) return `<tool>{"name":"data_summary","args":{"id":"${att}"}}</tool><tool>{"name":"calculate","args":{"expression":"1030 * 0.19"}}</tool>`;
      expect(last(m)).toContain('sum 1030');
      expect(last(m)).toContain('1030 * 0.19 = 195.7');
      return '<final>Total 1030; 19% is 195.7.</final>';
    });
    const conv = await store.createConversation('chat', 'x');
    const a = await store.addAttachment({ conversationId: conv.id, name: 'sales.csv', kind: 'csv', mime: 'text/csv', size: 40, scope: 'message', status: 'ready', error: null, text: 'city,sales\nBerlin,10\nBerlin,20\nParis,"1,000"\n', imageDataUrl: null, meta: {} });
    att = a.id;
    const r = await runTask(env, { taskId: 't7', conv, role: 'chat', message: 'Total?', attachments: [a], effort: 'balanced' }, ac());
    expect(r.answer).toContain('195.7');
  });

  it('answers a specific question about an attached table with data_query (grouped totals), nested arguments written as JSON text included', async () => {
    const { env, store, events } = await setup((m, n) => {
      if (n === 0) {
        const call = (args: object) => `<tool>${JSON.stringify({ name: 'data_query', args })}</tool>`;
        // the first call writes its nested argument as JSON text (a common model habit), the second names a column that does not exist
        return call({ file: att, group_by: 'city', metrics: JSON.stringify([{ column: 'sales', fn: 'sum' }]) }) + call({ id: att, group_by: 'nope' });
      }
      expect(last(m)).toContain('| Paris | 1 | 1000 |');
      expect(last(m)).toContain('| Berlin | 2 | 30 |');
      expect(last(m)).toContain('no column "nope"'); // a wrong column is explained, not guessed
      return '<final>Paris 1000, Berlin 30.</final>';
    });
    const conv = await store.createConversation('chat', 'x');
    const a = await store.addAttachment({ conversationId: conv.id, name: 'sales.csv', kind: 'csv', mime: 'text/csv', size: 40, scope: 'message', status: 'ready', error: null, text: 'city,sales\nBerlin,10\nBerlin,20\nParis,"1,000"\n', imageDataUrl: null, meta: {} });
    att = a.id;
    const r = await runTask(env, { taskId: 't7b', conv, role: 'chat', message: 'Sales per city?', attachments: [a], effort: 'balanced' }, ac());
    expect(r.answer).toContain('Paris 1000');
    expect(r.steps.map((x) => [x.tool, x.ok])).toEqual([['data_query', true], ['data_query', false]]);
    void events;
  });
});
let att = '';

describe('research loop', () => {
  it('records only fetched pages, validates the report against the ledger and builds the Sources list itself', async () => {
    const { env, store } = await setup((_m, n) => {
      if (n === 0) return '<todo>\n{"items": [{"text": "Read the docs", "status": "active"}]}\n</todo>\n<fetchurl {"url": "https://react.dev/learn"}';
      return '<final>## Summary\nHooks let function components use state and other React features [1](http://tauri.localhost/#src-1).\n\n## Findings\n- Hooks must be called at the top level of a component [1].\n\n## Sources\n[1] Invented — https://invented.example</final>';
    });
    const conv = await store.createConversation('research', 'x');
    const r = await runTask(env, { taskId: 't8', conv, role: 'research', message: 'How do hooks work?', attachments: [], effort: 'balanced' }, ac());
    expect(r.steps.map((s) => s.tool)).toEqual(['todo', 'fetch_url']);
    expect(r.answer).not.toMatch(/tauri\.localhost|#src-|invented/);
    expect(r.answer).toContain('## Sources\n[1] Learn React — https://react.dev/learn (fetched ');
    const v = r.meta.validation as { ok: boolean; cited: number[] };
    expect(v.ok).toBe(true);
    expect(v.cited).toEqual([1]);
    expect((await store.listSources(conv.id)).map((s) => s.url)).toEqual(['https://react.dev/learn']);
  });

  it('flags a report that cites nothing it read instead of accepting it silently', async () => {
    const { env, store } = await setup((_m, n) => (n === 0 ? '<tool>{"name":"fetch_url","args":{"url":"https://react.dev/learn"}}</tool>' : '<final>## Findings\nHooks were introduced in version sixteen point eight of the library [4].</final>'));
    const conv = await store.createConversation('research', 'x');
    const r = await runTask(env, { taskId: 't9', conv, role: 'research', message: 'Q', attachments: [], effort: 'balanced' }, ac());
    const v = r.meta.validation as { ok: boolean; unknownCitations: number[]; repaired: boolean };
    expect(v.ok).toBe(false);
    expect(v.unknownCitations).toEqual([4]);
    expect(v.repaired).toBe(true);
    expect(r.answer.startsWith('> **Citation check:**')).toBe(true);
    expect((await store.listSources(conv.id)).length).toBe(1);
  });
});
