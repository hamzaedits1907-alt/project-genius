// Regression: "What was the last route of the A340 before it retired?" was answered with "Which A340 do you
// mean?" (three times in a row) instead of a web search. A bare clarifying question to an answerable question
// gets one nudge to look it up and answer; honest "could not verify" answers are never turned into guesses.
import { describe, expect, it } from 'vitest';
import { runTask, type EngineEnv, type WebToolsApi } from '../web/engine/chat';
import { buildSystemPrompt, clarificationNudge, isIdleClarification } from '../web/engine/prompts';
import type { ChatMsg, CompletionResult } from '../web/engine/providers';
import { MemoryStore } from '../web/store';

describe('isIdleClarification', () => {
  it('recognises a reply that is only a question back', () => {
    expect(isIdleClarification("I'm not sure which A340 you're referring to. Could you clarify which aircraft's last route you want to know about?")).toBe(true);
    expect(isIdleClarification('Which one do you mean?')).toBe(true);
    expect(isIdleClarification('Hangi uçaktan bahsediyorsun?')).toBe(true);
  });
  it('leaves real answers, statements and code alone', () => {
    expect(isIdleClarification('The last scheduled A340 flight was LH 123 from Frankfurt to Munich. Anything else?')).toBe(false);
    expect(isIdleClarification('I could not verify the date. Which sources would you like me to check')).toBe(false);
    expect(isIdleClarification('Here is the code:\n```ts\nconst a = 1;\n```\nWhich do you prefer?')).toBe(false);
    expect(isIdleClarification('')).toBe(false);
  });
});

describe('the prompt', () => {
  const p = buildSystemPrompt({ research: false, assistantName: 'Genius', productCreator: 'Hamza', internet: true, effort: 'balanced', files: [], officialHints: [] });
  it('tells the model to search first, to interpret instead of asking, and never to invent', () => {
    expect(p).toContain('Never answer a factual question with only a question back');
    expect(p).toContain('search the web FIRST');
    expect(p).toContain('Never invent facts, dates, routes, numbers, names or links');
    expect(p).toContain('A bare "I do not know" is not an answer');
  });
  it('the nudge sends the model to the web only when internet access is on', () => {
    expect(clarificationNudge(true)).toContain('web_search');
    expect(clarificationNudge(false)).not.toContain('web_search');
  });
});

describe('the chat loop', () => {
  const PAGE = { url: 'https://example.org/a340', status: 200, contentType: 'text/html', title: 'A340', text: 'The last scheduled passenger flight of the Airbus A340 with Lufthansa was LH 1234.', truncated: false };
  async function setup(script: (msgs: ChatMsg[], n: number) => string) {
    const store = new MemoryStore();
    const log: ChatMsg[][] = [];
    const web: WebToolsApi = { search: async () => [{ title: 'A340', url: PAGE.url, snippet: 'retired' }], fetch: async () => PAGE };
    const env: EngineEnv = {
      store,
      provider: { provider: 'openai', baseUrl: 'https://api.openai.com/v1', apiKey: 'sk-test' },
      settings: { assistantName: 'Genius', productCreator: 'Hamza', internetAccess: true, temperature: 0.2, maxTokens: 4000, limits: { maxModelCalls: 0, maxToolCalls: 0, maxMinutes: 0, maxTokens: 0, maxProviderRetries: 0 } },
      tools: web,
      workspaceId: 'ws',
      resolveModels: async () => ['gpt-5-mini'],
      emit: () => undefined,
      drainSteering: () => [],
      completeFn: async (_c, msgs): Promise<CompletionResult> => {
        log.push(msgs.map((m) => ({ ...m })));
        return { content: script(msgs, log.length - 1), model: 'gpt-5-mini', reportedModel: 'gpt-5-mini', usage: { prompt: 1, completion: 1, total: 2 }, latencyMs: 1 };
      },
    };
    return { env, store, log };
  }
  const ac = () => new AbortController().signal;

  it('turns a bare clarifying question into a search and an answer from the page', async () => {
    const { env, store, log } = await setup((m, n) => {
      if (n === 0) return "<final>I'm not sure which A340 you're referring to. Could you clarify which aircraft's last route you want to know about?</final>";
      if (n === 1) {
        expect(m[m.length - 1].content).toContain('Do not ask the user to clarify');
        return '<tool>{"name":"web_search","args":{"query":"A340 last scheduled passenger flight before retirement"}}</tool>';
      }
      if (n === 2) return '<tool>{"name":"fetch_url","args":{"url":"https://example.org/a340"}}</tool>';
      return '<final>Reading "a340" as the Airbus A340: the last scheduled passenger flight was LH 1234 (source: https://example.org/a340).</final>';
    });
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't1', conv, role: 'chat', message: 'What was the last route of a340 before retire', attachments: [], effort: 'balanced' }, ac());
    expect(r.answer).toContain('LH 1234');
    expect(r.steps.map((s) => s.tool)).toEqual(['web_search', 'fetch_url']);
    expect(log).toHaveLength(4);
  });

  it('nudges only once: a model that insists on asking is not looped', async () => {
    const { env, store, log } = await setup(() => '<final>Which aircraft do you mean?</final>');
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't2', conv, role: 'chat', message: 'tell me about it', attachments: [], effort: 'balanced' }, ac());
    expect(r.answer).toBe('Which aircraft do you mean?');
    expect(log).toHaveLength(2);
  });

  it('does not nudge a model that already used a tool, or answered normally', async () => {
    const { env, store, log } = await setup((_m, n) => (n === 0 ? '<final>Paris is the capital of France.</final>' : '<final>x</final>'));
    const conv = await store.createConversation('chat', 'x');
    const r = await runTask(env, { taskId: 't3', conv, role: 'chat', message: 'capital of France?', attachments: [], effort: 'balanced' }, ac());
    expect(r.answer).toBe('Paris is the capital of France.');
    expect(log).toHaveLength(1);
  });
});
