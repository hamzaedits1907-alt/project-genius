// The Genius Web runtime end to end with a scripted model: the same command names the interface uses,
// the event bus, conversations in a store, key handling, relay consent and Agent Mode missions.
import { describe, expect, it } from 'vitest';
import { WebRuntime, sessionKeyVault, type KeyVault } from '../web/runtime';
import { MemoryStore } from '../web/store';
import type { WebToolsApi } from '../web/engine/chat';
import type { ChatConversation, MessageRow, Mission, Settings, TaskInfo } from '../lib/types';

const memVault = (): KeyVault => {
  const m = new Map<string, string>();
  return { get: (p) => m.get(p) ?? null, set: (p, k) => void m.set(p, k), remove: (p) => void m.delete(p) };
};
const tools: WebToolsApi = {
  search: async () => [{ title: 'Learn React', url: 'https://react.dev/learn', snippet: 'Hooks' }],
  fetch: async (url) => ({ url, status: 200, contentType: 'text/html', title: 'Learn React', text: 'Hooks let function components use state.', truncated: false }),
};

function make(script: (n: number, system: string, last: string) => string) {
  let n = 0;
  const rt = new WebRuntime({
    store: new MemoryStore(),
    tools,
    accessToken: async () => 't',
    relayUrl: '/api/relay',
    keys: memVault(),
    listModelsFn: async () => [{ id: 'gpt-5-mini', owner: 'openai', created: null }, { id: 'gpt-5', owner: 'openai', created: null }, { id: 'text-embedding-3-large', owner: 'openai', created: null }],
    completeFn: async (_c, msgs) => ({ content: script(n++, msgs[0].content, msgs[msgs.length - 1].content), model: 'gpt-5', usage: { prompt: 5, completion: 5, total: 10 }, latencyMs: 3 }),
  });
  return rt;
}
const call = <T>(rt: WebRuntime, cmd: string, args: Record<string, unknown> = {}) => rt.invoke(cmd, args) as Promise<T>;
const until = async (fn: () => Promise<boolean> | boolean) => {
  for (let i = 0; i < 200; i++) {
    if (await fn()) return;
    await new Promise((r) => setTimeout(r, 10));
  }
  throw new Error('condition not reached');
};

describe('Genius Web runtime', () => {
  it('keeps the key in the session vault only and reports it without revealing it', async () => {
    const rt = make(() => '<final>x</final>');
    expect(await call(rt, 'key_status')).toMatchObject({ stored: false });
    const st = await call<{ stored: boolean; hint: string }>(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    expect(st).toMatchObject({ stored: true, hint: '…7890' });
    expect(JSON.stringify(await call(rt, 'get_settings'))).not.toContain('sk-test');
    await expect(call(rt, 'save_api_key', { key: 'bad key' })).rejects.toMatchObject({ kind: 'validation' });
    expect(sessionKeyVault().get('openai')).toBeNull();
  });

  it('chats: stores the user message, streams a task through events and stores the answer', async () => {
    const rt = make(() => "<final>I'm Genius.</final>");
    await call(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    const events: { status: string }[] = [];
    rt.on('genius://activity', (p) => events.push(p as { status: string }));
    const conv = await call<ChatConversation>(rt, 'create_chat_conversation', { mode: 'chat' });
    const task = await call<TaskInfo>(rt, 'start_chat', { conversationId: conv.id, request: { message: 'Hi! Who are you?', attachmentIds: [] } });
    expect(task.kind).toBe('chat');
    expect((await call<TaskInfo[]>(rt, 'active_tasks')).length).toBe(1);
    await expect(call(rt, 'start_chat', { conversationId: conv.id, request: { message: 'again', attachmentIds: [] } })).rejects.toMatchObject({ kind: 'busy' });
    await until(async () => (await call<TaskInfo[]>(rt, 'active_tasks')).length === 0);
    const msgs = await call<MessageRow[]>(rt, 'conversation_messages', { conversationId: conv.id });
    expect(msgs.map((m) => [m.role, m.kind])).toEqual([['user', 'chat'], ['assistant', 'answer']]);
    expect(msgs[1].content).toBe("I'm Genius.");
    expect(events.map((e) => e.status)).toEqual(['running', 'completed']);
    expect((await call<ChatConversation[]>(rt, 'list_chat_conversations', { mode: null }))[0].title).toBe('Hi! Who are you?');
  });

  it('a failing provider becomes an honest error message, not a hang', async () => {
    const rt = new WebRuntime({ store: new MemoryStore(), tools, accessToken: async () => 't', relayUrl: '/api/relay', keys: memVault(), listModelsFn: async () => [{ id: 'gpt-5', owner: null, created: null }], completeFn: async () => { throw new Error('boom'); } });
    await call(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    const conv = await call<ChatConversation>(rt, 'create_chat_conversation', { mode: 'chat' });
    await call(rt, 'start_chat', { conversationId: conv.id, request: { message: 'hello', attachmentIds: [] } });
    await until(async () => (await call<TaskInfo[]>(rt, 'active_tasks')).length === 0);
    const last = (await call<MessageRow[]>(rt, 'conversation_messages', { conversationId: conv.id })).pop()!;
    expect(last.kind).toBe('error');
    expect(last.content).toContain('boom');
  });

  it('refuses to send anything without a key, and without consent for a provider that blocks browsers', async () => {
    const rt = make(() => '<final>x</final>');
    const conv = await call<ChatConversation>(rt, 'create_chat_conversation', { mode: 'chat' });
    await expect(call(rt, 'start_chat', { conversationId: conv.id, request: { message: 'hi', attachmentIds: [] } })).rejects.toMatchObject({ kind: 'no_key' });
    await call<Settings>(rt, 'save_settings', { settings: { ...(await call<Settings>(rt, 'get_settings')), provider: 'nvidia' } });
    await call(rt, 'save_api_key', { key: 'nvapi-test-1234567890' });
    await expect(call(rt, 'start_chat', { conversationId: conv.id, request: { message: 'hi', attachmentIds: [] } })).rejects.toMatchObject({ kind: 'relay_consent' });
    const s = await call<Settings>(rt, 'get_settings');
    await call(rt, 'save_settings', { settings: { ...s, relayConsent: true } });
    await call(rt, 'start_chat', { conversationId: conv.id, request: { message: 'hi', attachmentIds: [] } });
    await until(async () => (await call<TaskInfo[]>(rt, 'active_tasks')).length === 0);
  });

  it('Auto picks the best listed chat model and never an embedding model', async () => {
    const rt = make(() => '<final>x</final>');
    await call(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    const r = await call<{ agent: string[]; vision: string[] }>(rt, 'resolved_models');
    expect(r.agent[0]).toBe('gpt-5');
    expect(r.agent).not.toContain('text-embedding-3-large');
    expect(r.vision.length).toBeGreaterThan(0);
  });

  it('desktop-only commands fail with an honest explanation', async () => {
    const rt = make(() => '<final>x</final>');
    await expect(call(rt, 'git_view', { projectId: 'p' })).rejects.toMatchObject({ kind: 'unsupported' });
    await expect(call(rt, 'start_operation', { projectId: 'p', op: 'install' })).rejects.toMatchObject({ message: expect.stringContaining('Genius Desktop') });
  });

  it('files: reads a CSV, rejects a renamed non-image and unsupported types', async () => {
    const rt = make(() => '<final>x</final>');
    const conv = await call<ChatConversation>(rt, 'create_chat_conversation', { mode: 'chat' });
    const b64 = (s: string) => btoa(s);
    const ok = await call<{ status: string; chars: number; kind: string }>(rt, 'import_attachment_data', { name: 'a.csv', dataBase64: b64('x,y\n1,2\n'), scope: 'message', conversationId: conv.id });
    expect(ok).toMatchObject({ status: 'ready', kind: 'csv' });
    const fake = await call<{ status: string; error: string }>(rt, 'import_attachment_data', { name: 'photo.png', dataBase64: b64('not a png'), scope: 'message', conversationId: conv.id });
    expect(fake.status).toBe('failed');
    expect(fake.error).toContain('not a PNG');
    const bad = await call<{ status: string; error: string }>(rt, 'import_attachment_data', { name: 'x.exe', dataBase64: b64('MZ'), scope: 'message', conversationId: conv.id });
    expect(bad.error).toContain('unsupported file type');
  });

  it('Agent Mode: plans, runs steps with tools, ends completed but NOT verified, with an honest summary', async () => {
    const rt = make((n, system) => {
      if (system.includes('coordinator of Genius Web Agent Mode')) return '{"summary": "two steps", "tasks": [{"id": "t1", "title": "Find the docs", "description": "Fetch the React docs", "dependsOn": []}, {"id": "t2", "title": "Summarise", "description": "Summarise them", "dependsOn": ["t1"]}]}';
      return n % 2 === 0 ? '<tool>{"name":"fetch_url","args":{"url":"https://react.dev/learn"}}</tool>' : '<final>Hooks let function components use state (read https://react.dev/learn).</final>';
    });
    await call(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    const updates: Mission[] = [];
    rt.on('genius://mission-update', (p) => updates.push(p as Mission));
    const m = await call<Mission>(rt, 'create_mission', { projectId: 'browser-workspace', goal: 'Summarise the React docs on hooks', teamId: null });
    expect(m.status).toBe('planning');
    await until(async () => (await call<Mission>(rt, 'get_mission', { missionId: m.id })).status === 'ready');
    const planned = await call<Mission>(rt, 'get_mission', { missionId: m.id });
    expect(planned.tasks.map((t) => t.title)).toEqual(['Find the docs', 'Summarise']);
    expect(planned.tasks[1].dependsOn).toEqual([planned.tasks[0].id]);
    await call(rt, 'start_mission', { missionId: m.id });
    await until(async () => ['completed', 'failed'].includes((await call<Mission>(rt, 'get_mission', { missionId: m.id })).status));
    const done = await call<Mission>(rt, 'get_mission', { missionId: m.id });
    expect(done.status).toBe('completed');
    expect(done.tasks.every((t) => t.status === 'done' && t.verification.build === 'unknown' && t.verification.tests === 'unknown')).toBe(true);
    expect(done.summary).toContain('not verified');
    expect(done.summary).toContain('2 of 2 step(s) done');
    expect(updates.some((u) => u.status === 'running')).toBe(true);
    const logs = await call<{ kind: string }[]>(rt, 'agent_logs', { agentId: 'web-researcher' });
    expect(logs.some((l) => l.kind === 'tool')).toBe(true);
  });

  it('Agent Mode refuses goals that need a computer, saying so', async () => {
    const rt = make(() => '{"summary": "", "unsupported": "it must edit files and run npm", "tasks": []}');
    await call(rt, 'save_api_key', { key: 'sk-test-1234567890' });
    const m = await call<Mission>(rt, 'create_mission', { projectId: 'x', goal: 'Refactor my repository and run its tests', teamId: null });
    await until(async () => (await call<Mission>(rt, 'get_mission', { missionId: m.id })).status === 'failed');
    expect((await call<Mission>(rt, 'get_mission', { missionId: m.id })).summary).toContain('needs Genius Desktop');
  });
});
