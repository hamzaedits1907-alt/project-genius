// Conversation sync on a REAL PostgreSQL engine (PGlite) with the real migrations: idempotent push, cursor
// pull, tombstones, ordering, search, and that one account can never read or change another's conversations.
// Like rls.test.ts this proves the database logic, not a configured Supabase project.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { PGlite } from '@electric-sql/pglite';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const dir = join(__dirname, '..', '..', 'supabase', 'migrations');
const sql = (f: string) => readFileSync(join(dir, f), 'utf8');

let db: PGlite;
const ids = { alice: '', bob: '' };
const U = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, '0')}`;

async function as<T>(user: string, fn: () => Promise<T>): Promise<T> {
  await db.exec(`set role authenticated; select set_config('request.jwt.claim.sub', '${user}', false);`);
  try {
    return await fn();
  } finally {
    await db.exec(`reset role; select set_config('request.jwt.claim.sub', '', false);`);
  }
}
const rpc = async <T = any>(user: string, fn: string, ...args: unknown[]): Promise<T> => // eslint-disable-line @typescript-eslint/no-explicit-any
  as(user, async () => {
    const ph = args.map((_, i) => `$${i + 1}`).join(', ');
    const tableFn = fn === 'conversation_summaries';
    const r = await db.query<{ v: T }>(tableFn ? `select coalesce(jsonb_agg(t), '[]'::jsonb) as v from public.${fn}(${ph}) t` : `select public.${fn}(${ph}) as v`, args.map((a) => (a !== null && typeof a === 'object' ? JSON.stringify(a) : a)));
    return r.rows[0].v;
  });
const q = async <R = Record<string, unknown>>(text: string, params: unknown[] = []) => (await db.query<R>(text, params)).rows;

const conv = (n: number, extra: Record<string, unknown> = {}) => ({ id: U(n), mode: 'chat', title: `Conversation ${n}`, origin: null, createdAt: '2026-10-01T10:00:00Z', updatedAt: '2026-10-01T10:00:00Z', metaUpdatedAt: '2026-10-01T10:00:00Z', ...extra });
const msg = (n: number, c: number, extra: Record<string, unknown> = {}) => ({ id: U(1000 + n), conversationId: U(c), role: n % 2 ? 'user' : 'assistant', kind: 'chat', content: `message ${n}`, meta: {}, taskId: null, createdAt: `2026-10-01T10:00:0${n % 10}Z`, updatedAt: `2026-10-01T10:00:0${n % 10}Z`, ...extra });

beforeAll(async () => {
  db = new PGlite();
  await db.exec(`
    create role anon nologin; create role authenticated nologin; create role service_role nologin;
    create schema auth;
    create table auth.users (id uuid primary key default gen_random_uuid(), email text);
    create function auth.uid() returns uuid language sql stable as $$ select nullif(current_setting('request.jwt.claim.sub', true), '')::uuid $$;
    create schema storage;
    create table storage.buckets (id text primary key, name text, public boolean, file_size_limit bigint, allowed_mime_types text[]);
    create table storage.objects (id uuid primary key default gen_random_uuid(), bucket_id text, name text);
    alter table storage.objects enable row level security;
    create function storage.foldername(name text) returns text[] language sql immutable as $$ select (string_to_array(name, '/'))[1:array_length(string_to_array(name, '/'), 1) - 1] $$;
    grant usage on schema public, auth, storage to anon, authenticated;
    grant select, insert, update, delete on storage.objects to authenticated;
    grant select on storage.buckets to authenticated;
    alter default privileges in schema public grant all on tables to anon, authenticated;
    alter default privileges in schema public grant all on sequences to anon, authenticated;
  `);
  for (const f of ['20261019000000_genius_web.sql', '20261019000100_genius_web_storage.sql', '20261019000200_missions.sql', '20261020000000_admin_advanced.sql', '20261021000000_sync.sql']) await db.exec(sql(f));
  ids.alice = (await q<{ id: string }>(`insert into auth.users (email) values ('alice@example.com') returning id`))[0].id;
  ids.bob = (await q<{ id: string }>(`insert into auth.users (email) values ('bob@example.com') returning id`))[0].id;
});
afterAll(async () => {
  await db.close();
});

describe('sync_push', () => {
  it('stores a conversation with its messages and is idempotent when the same batch is sent again', async () => {
    const batch = [[conv(1)], [msg(1, 1), msg(2, 1)]] as const;
    expect(await rpc(ids.alice, 'sync_push', batch[0], batch[1])).toEqual({ conversations: 1, messages: 2 });
    await rpc(ids.alice, 'sync_push', batch[0], batch[1]); // a retry after a lost response
    expect(await q(`select count(*)::int as n from public.messages where conversation_id = '${U(1)}'`)).toEqual([{ n: 2 }]);
    expect(await q(`select count(*)::int as n from public.conversations where id = '${U(1)}'`)).toEqual([{ n: 1 }]);
  });

  it('keeps the original timestamps (imports of old desktop history stay in their real order)', async () => {
    const r = await q<{ created_at: string }>(`select created_at from public.messages where id = '${U(1001)}'`);
    expect(new Date(r[0].created_at).toISOString()).toBe('2026-10-01T10:00:01.000Z');
  });

  it('newest metadata wins: an older rename cannot overwrite a newer one', async () => {
    await rpc(ids.alice, 'sync_push', [conv(1, { title: 'Newer title', metaUpdatedAt: '2026-10-02T10:00:00Z' })], []);
    await rpc(ids.alice, 'sync_push', [conv(1, { title: 'Stale title', metaUpdatedAt: '2026-10-01T12:00:00Z' })], []);
    expect((await q<{ title: string }>(`select title from public.conversations where id = '${U(1)}'`))[0].title).toBe('Newer title');
  });

  it('a delete is a tombstone that an old copy cannot undo', async () => {
    await rpc(ids.alice, 'sync_push', [conv(2)], [msg(3, 2)]);
    await rpc(ids.alice, 'delete_conversation', U(2));
    await rpc(ids.alice, 'sync_push', [conv(2)], [msg(3, 2)]); // another device still holding the old copy uploads it
    expect((await q<{ deleted_at: string | null }>(`select deleted_at from public.conversations where id = '${U(2)}'`))[0].deleted_at).not.toBeNull();
    const list = await rpc<{ id: string }[]>(ids.alice, 'conversation_summaries', null, null, 100, null);
    expect(list.some((c) => c.id === U(2))).toBe(false);
  });

  it('rejects malformed input instead of storing it', async () => {
    await expect(rpc(ids.alice, 'sync_push', [conv(3, { mode: 'agent' })], [])).rejects.toThrow(/unsupported conversation mode/);
    await expect(rpc(ids.alice, 'sync_push', [conv(3)], [msg(5, 3, { role: 'tool' })])).rejects.toThrow(/unsupported message role/);
    await expect(rpc(ids.alice, 'sync_push', Array.from({ length: 301 }, (_, i) => conv(5000 + i)), [])).rejects.toThrow(/batch too large/);
    // a rejected batch is rolled back as a whole: nothing of it (not even its valid conversation) was stored
    expect(await q(`select count(*)::int as n from public.conversations where id = '${U(3)}'`)).toEqual([{ n: 0 }]);
  });
});

describe('account isolation', () => {
  it("never shows another account's conversations in a pull or the sidebar list", async () => {
    await rpc(ids.alice, 'sync_push', [conv(10, { title: 'Alice secret plans' })], [msg(10, 10)]);
    const bobConvs = await rpc<{ id: string }[]>(ids.bob, 'sync_pull_conversations', null, null, 200);
    const bobMsgs = await rpc<{ id: string }[]>(ids.bob, 'sync_pull_messages', null, null, 300);
    const bobList = await rpc<{ id: string }[]>(ids.bob, 'conversation_summaries', null, null, 100, null);
    expect(bobConvs).toEqual([]);
    expect(bobMsgs).toEqual([]);
    expect(bobList).toEqual([]);
    expect((await rpc<unknown[]>(ids.bob, 'conversation_summaries', null, 'secret', 100, null)).length).toBe(0);
  });

  it("cannot overwrite, delete or add to another account's conversation by reusing its ids", async () => {
    await rpc(ids.bob, 'sync_push', [conv(10, { title: 'Hijacked', metaUpdatedAt: '2030-01-01T00:00:00Z', deletedAt: '2030-01-01T00:00:00Z' })], []).catch(() => undefined);
    await expect(rpc(ids.bob, 'sync_push', [], [msg(11, 10)])).rejects.toThrow();
    await rpc(ids.bob, 'delete_conversation', U(10));
    await rpc(ids.bob, 'delete_messages_from', U(10), U(1010));
    const row = (await q<{ title: string; deleted_at: string | null }>(`select title, deleted_at from public.conversations where id = '${U(10)}'`))[0];
    expect(row).toEqual({ title: 'Alice secret plans', deleted_at: null });
    expect(await q(`select count(*)::int as n from public.messages where conversation_id = '${U(10)}' and deleted_at is null`)).toEqual([{ n: 1 }]);
    expect(await q(`select count(*)::int as n from public.messages where conversation_id = '${U(10)}'`)).toEqual([{ n: 1 }]); // Bob's message was not added
  });

  it('a suspended account cannot write', async () => {
    await db.exec(`update public.profiles set status = 'suspended' where id = '${ids.bob}'`);
    await expect(rpc(ids.bob, 'sync_push', [conv(20)], [])).rejects.toThrow();
    await db.exec(`update public.profiles set status = 'active' where id = '${ids.bob}'`);
    await rpc(ids.bob, 'sync_push', [conv(20, { title: 'Bob chat' })], []);
    expect((await rpc<{ id: string }[]>(ids.bob, 'conversation_summaries', null, null, 100, null)).map((c) => c.id)).toEqual([U(20)]);
  });
});

describe('sync_pull cursors', () => {
  it('pages through everything exactly once, in change order, and reports tombstones', async () => {
    const all: { id: string; syncedAt: string; deletedAt: string | null }[] = [];
    let ts: string | null = null;
    let id: string | null = null;
    for (let i = 0; i < 20; i++) {
      const page = await rpc<typeof all>(ids.alice, 'sync_pull_conversations', ts, id, 2);
      if (!page.length) break;
      all.push(...page);
      ts = page[page.length - 1].syncedAt;
      id = page[page.length - 1].id;
    }
    const seen = all.map((c) => c.id);
    expect(new Set(seen).size).toBe(seen.length); // no duplicates
    expect(seen).toEqual(expect.arrayContaining([U(1), U(2), U(10)]));
    expect(all.find((c) => c.id === U(2))?.deletedAt).not.toBeNull();
    expect(all.map((c) => c.syncedAt)).toEqual([...all.map((c) => c.syncedAt)].sort());
  });

  it('a change made after the cursor shows up in the next pull', async () => {
    const first = await rpc<{ id: string; syncedAt: string }[]>(ids.alice, 'sync_pull_messages', null, null, 500);
    const last = first[first.length - 1];
    await rpc(ids.alice, 'sync_push', [], [msg(30, 10, { content: 'late message' })]);
    const next = await rpc<{ id: string; content: string }[]>(ids.alice, 'sync_pull_messages', last.syncedAt, last.id, 500);
    expect(next.map((m) => m.content)).toEqual(['late message']);
  });

  it('does not hand Code sessions to clients that only sync chat and research', async () => {
    await rpc(ids.alice, 'sync_push', [conv(40, { origin: { flavor: 'code' } })], []).catch(() => undefined);
    await as(ids.alice, () => db.query(`insert into public.conversations (id, mode, title, origin) values ('${U(41)}', 'chat', 'web code session', '{"flavor":"code"}'), ('${U(42)}', 'agent', 'mission', null)`));
    const ids2 = (await rpc<{ id: string }[]>(ids.alice, 'sync_pull_conversations', null, null, 500)).map((c) => c.id);
    expect(ids2).not.toContain(U(41));
    expect(ids2).not.toContain(U(42));
  });
});

describe('ordering, rewind and the sidebar list', () => {
  it('orders messages with identical timestamps deterministically by their database sequence', async () => {
    await rpc(ids.alice, 'sync_push', [conv(50)], [msg(1, 50, { id: U(2001), content: 'first', createdAt: '2026-10-03T00:00:00Z', updatedAt: '2026-10-03T00:00:00Z' }), msg(2, 50, { id: U(2002), content: 'second', createdAt: '2026-10-03T00:00:00Z', updatedAt: '2026-10-03T00:00:00Z' }), msg(3, 50, { id: U(2003), content: 'third', createdAt: '2026-10-03T00:00:00Z', updatedAt: '2026-10-03T00:00:00Z' })]);
    const rows = await as(ids.alice, () => q<{ content: string }>(`select content from public.messages where conversation_id = '${U(50)}' order by created_at, seq`));
    expect(rows.map((r) => r.content)).toEqual(['first', 'second', 'third']);
  });

  it('delete_messages_from tombstones the message and everything after it', async () => {
    expect(await rpc(ids.alice, 'delete_messages_from', U(50), U(2002))).toBe(2);
    const live = await as(ids.alice, () => q<{ content: string }>(`select content from public.messages where conversation_id = '${U(50)}' and deleted_at is null order by created_at, seq`));
    expect(live.map((r) => r.content)).toEqual(['first']);
  });

  it('summaries count only live messages, show the latest snippet, search text and escape wildcards', async () => {
    const list = await rpc<{ id: string; message_count: number; snippet: string }[]>(ids.alice, 'conversation_summaries', 'chat', null, 100, null);
    const c50 = list.find((c) => c.id === U(50))!;
    expect(Number(c50.message_count)).toBe(1);
    expect(c50.snippet).toBe('first');
    expect((await rpc<{ id: string }[]>(ids.alice, 'conversation_summaries', null, 'late message', 100, null)).map((c) => c.id)).toEqual([U(10)]);
    await rpc(ids.alice, 'sync_push', [conv(51, { title: '100% done_ok' })], []);
    expect((await rpc<{ id: string }[]>(ids.alice, 'conversation_summaries', null, '100%', 100, null)).map((c) => c.id)).toEqual([U(51)]);
    expect(await rpc<unknown[]>(ids.alice, 'conversation_summaries', null, '%', 100, null)).toHaveLength(1); // a literal %, not "match everything"
  });
});
