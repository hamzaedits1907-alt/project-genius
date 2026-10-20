// Owner access control on a REAL PostgreSQL engine (PGlite): open / early access / closed, invites, the
// waitlist, per-user limits, content removal and statistics — and that only an administrator can use any of it.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { PGlite } from '@electric-sql/pglite';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const dir = join(__dirname, '..', '..', 'supabase', 'migrations');
let db: PGlite;
const ids = { owner: '', alice: '', bob: '', carol: '' };

async function as<T>(user: string, fn: () => Promise<T>): Promise<T> {
  await db.exec(`set role authenticated; select set_config('request.jwt.claim.sub', '${user}', false);`);
  try {
    return await fn();
  } finally {
    await db.exec(`reset role; select set_config('request.jwt.claim.sub', '', false);`);
  }
}
const call = async <T = any>(user: string, text: string, params: unknown[] = []): Promise<T> => // eslint-disable-line @typescript-eslint/no-explicit-any
  as(user, async () => (await db.query<{ v: T }>(`select ${text} as v`, params.map((p) => (p !== null && typeof p === 'object' && !Array.isArray(p) ? JSON.stringify(p) : p)))).rows[0].v);
const access = (user: string) => call<any>(user, 'public.my_access()'); // eslint-disable-line @typescript-eslint/no-explicit-any
const setMode = (mode: string, msg = '', waitlist = true) => call(ids.owner, 'public.admin_set_access($1, $2, $3, $4)', [mode, 'early', msg, waitlist]);

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
  for (const f of ['20261019000000_genius_web.sql', '20261019000100_genius_web_storage.sql', '20261019000200_missions.sql', '20261020000000_admin_advanced.sql', '20261021000000_sync.sql', '20261022000000_key_vault.sql', '20261023000000_admin_access.sql']) await db.exec(readFileSync(join(dir, f), 'utf8'));
  const mk = async (email: string) => (await db.query<{ id: string }>(`insert into auth.users (email) values ('${email}') returning id`)).rows[0].id;
  ids.owner = await mk('owner@example.com');
  ids.alice = await mk('alice@example.com');
  ids.bob = await mk('bob@example.com');
  await db.exec(`insert into public.admin_users (user_id) values ('${ids.owner}')`);
});
afterAll(async () => {
  await db.close();
});

describe('access modes', () => {
  it('is open by default: everybody can write', async () => {
    expect((await access(ids.alice)).access).toMatchObject({ mode: 'open', locked: false });
    await as(ids.alice, () => db.query(`insert into public.conversations (mode, title) values ('chat', 'hello')`));
  });

  it('early access: only the cohort and administrators are in; everyone else is locked out by the database', async () => {
    await setMode('early', 'Private beta, ask for an invite.');
    const a = await access(ids.alice);
    expect(a.access).toMatchObject({ mode: 'early', locked: true, message: 'Private beta, ask for an invite.' });
    expect(a.features.chat).toMatchObject({ available: false, reason: 'locked' });
    await expect(as(ids.alice, () => db.query(`insert into public.conversations (mode, title) values ('chat', 'blocked')`))).rejects.toThrow(/row-level security/);
    await expect(call(ids.alice, `public.sync_push('[]'::jsonb, '[]'::jsonb)`)).resolves.toBeTruthy(); // an empty batch writes nothing
    const o = await access(ids.owner);
    expect(o.access.locked).toBe(false);
    expect(o.isAdmin).toBe(true);
    await as(ids.owner, () => db.query(`insert into public.conversations (mode, title) values ('chat', 'owner is never locked out')`));
  });

  it('adding someone to the cohort lets them in at once; removing them locks them out again', async () => {
    await call(ids.owner, `public.admin_cohort_member('early', $1, true)`, [ids.alice]);
    expect((await access(ids.alice)).access.locked).toBe(false);
    await as(ids.alice, () => db.query(`insert into public.conversations (mode, title) values ('chat', 'welcome')`));
    await call(ids.owner, `public.admin_cohort_member('early', $1, false)`, [ids.alice]);
    expect((await access(ids.alice)).access.locked).toBe(true);
  });

  it('closed: even cohort members are out, administrators stay in; opening again restores everyone', async () => {
    await call(ids.owner, `public.admin_cohort_member('early', $1, true)`, [ids.alice]);
    await setMode('closed');
    expect((await access(ids.alice)).access).toMatchObject({ mode: 'closed', locked: true });
    expect((await access(ids.owner)).access.locked).toBe(false);
    await setMode('open');
    expect((await access(ids.alice)).access.locked).toBe(false);
    expect((await access(ids.bob)).access.locked).toBe(false);
  });

  it('refuses nonsense and every non-administrator', async () => {
    await expect(setMode('everyone')).rejects.toThrow(/mode must be/);
    await expect(call(ids.alice, `public.admin_set_access('open', 'early', '', true)`)).rejects.toThrow(/administrator access required/);
    await expect(call(ids.alice, `public.admin_access_overview()`)).rejects.toThrow(/administrator access required/);
    await expect(call(ids.alice, `public.admin_invite_emails(array['x@y.zz'], null)`)).rejects.toThrow(/administrator access required/);
    await expect(call(ids.alice, `public.admin_purge_user_data($1)`, [ids.bob])).rejects.toThrow(/administrator access required/);
    await expect(call(ids.alice, `public.admin_set_user_limits($1, '{}'::jsonb, '')`, [ids.alice])).rejects.toThrow(/administrator access required/);
    await expect(call(ids.alice, `public.admin_system_stats()`)).rejects.toThrow(/administrator access required/);
  });
});

describe('waitlist and invites', () => {
  it('a locked user asks to be let in; the owner approves and they are in', async () => {
    await setMode('early', 'Closed beta');
    await call(ids.owner, `public.admin_cohort_member('early', $1, false)`, [ids.alice]);
    expect(await call(ids.bob, `public.request_access('please let me in')`)).toBe('pending');
    expect((await access(ids.bob)).access.request).toBe('pending');
    const pending = await as(ids.owner, async () => (await db.query<{ email: string; note: string }>(`select * from public.admin_list_access_requests('pending')`)).rows);
    expect(pending.map((r) => r.email)).toEqual(['bob@example.com']);
    expect(pending[0].note).toBe('please let me in');
    await call(ids.owner, `public.admin_decide_access_request($1, true)`, [ids.bob]);
    expect((await access(ids.bob)).access).toMatchObject({ locked: false, request: 'approved' });
  });

  it('a declined request stays declined and the user stays out', async () => {
    await call(ids.alice, `public.request_access('me too')`);
    await call(ids.owner, `public.admin_decide_access_request($1, false)`, [ids.alice]);
    expect(await call(ids.alice, `public.request_access('again please')`)).toBe('declined');
    expect((await access(ids.alice)).access.locked).toBe(true);
  });

  it('a user cannot approve themselves or read other requests', async () => {
    await expect(call(ids.alice, `public.admin_decide_access_request($1, true)`, [ids.alice])).rejects.toThrow(/administrator access required/);
    expect(await as(ids.alice, async () => (await db.query(`select * from public.access_requests`)).rows)).toHaveLength(1); // only their own
    await expect(as(ids.alice, () => db.query(`update public.access_requests set status = 'approved'`))).rejects.toThrow(/permission denied/);
  });

  it('invites: a registered address joins at once, a new address joins when it signs up, bad addresses are reported', async () => {
    const r = await call<{ invited: number; addedNow: number; invalid: string[] }>(ids.owner, `public.admin_invite_emails(array['Alice@Example.com', 'newbie@example.com', 'not-an-email', '  '], null)`);
    expect(r).toMatchObject({ invited: 1, addedNow: 1, invalid: ['not-an-email'] });
    expect((await access(ids.alice)).access.locked).toBe(false); // alice was invited (case-insensitive) and already had an account
    const carol = (await db.query<{ id: string }>(`insert into auth.users (email) values ('Newbie@Example.com') returning id`)).rows[0].id;
    ids.carol = carol;
    expect((await access(carol)).access.locked).toBe(false); // claimed the invite at registration
    const list = await as(ids.owner, async () => (await db.query<{ email: string; claimed_by: string | null }>(`select * from public.admin_list_invites()`)).rows);
    expect(list.every((i) => i.claimed_by !== null)).toBe(true);
    await call(ids.owner, `public.admin_revoke_invite('newbie@example.com')`);
    expect((await access(carol)).access.locked).toBe(true);
  });

  it('too many addresses at once are refused', async () => {
    await expect(call(ids.owner, `public.admin_invite_emails(array(select 'u' || g || '@example.com' from generate_series(1, 201) g), null)`)).rejects.toThrow(/at most 200/);
  });

  it('overview counts match', async () => {
    const o = await call<any>(ids.owner, 'public.admin_access_overview()'); // eslint-disable-line @typescript-eslint/no-explicit-any
    expect(o.access.mode).toBe('early');
    expect(o.users).toBe(4);
    expect(o.allowedNow).toBeGreaterThanOrEqual(2);
    await setMode('open');
  });
});

describe('per-user limits, purge and statistics', () => {
  it('a limit override changes what the user is told and what the daily cap allows', async () => {
    await call(ids.owner, `public.admin_set_user_limits($1, $2::jsonb, 'friend of the team')`, [ids.bob, JSON.stringify({ dailyResearch: 1, maxModelCalls: 7 })]);
    const ctx = await call<any>(ids.bob, 'public.my_context()'); // eslint-disable-line @typescript-eslint/no-explicit-any
    expect(ctx.limits).toMatchObject({ dailyResearch: 1, maxModelCalls: 7, maxToolCalls: 80 }); // overridden keys replace, the rest stays from the plan
    await as(ids.bob, () => db.query(`insert into public.usage_events (kind) values ('research')`));
    await expect(as(ids.bob, () => db.query(`insert into public.usage_events (kind) values ('research')`))).rejects.toThrow(/Daily limit reached/);
    await call(ids.owner, `public.admin_set_user_limits($1, '{}'::jsonb, '')`, [ids.bob]);
    expect((await call<any>(ids.bob, 'public.my_context()')).limits.dailyResearch).toBe(10); // eslint-disable-line @typescript-eslint/no-explicit-any
  });

  it('rejects unknown or negative limits', async () => {
    await expect(call(ids.owner, `public.admin_set_user_limits($1, '{"hack": 1}'::jsonb, '')`, [ids.bob])).rejects.toThrow(/unknown limit/);
    await expect(call(ids.owner, `public.admin_set_user_limits($1, '{"dailyResearch": -1}'::jsonb, '')`, [ids.bob])).rejects.toThrow(/non-negative/);
  });

  it("purge removes a user's content but not the account, and never an administrator's", async () => {
    await as(ids.bob, () => db.query(`insert into public.conversations (mode, title) values ('chat', 'to be removed')`));
    const r = await call<{ conversations: number }>(ids.owner, 'public.admin_purge_user_data($1)', [ids.bob]);
    expect(r.conversations).toBeGreaterThanOrEqual(1);
    expect((await db.query(`select count(*)::int as n from public.conversations where user_id = '${ids.bob}'`)).rows[0]).toEqual({ n: 0 });
    expect((await db.query(`select count(*)::int as n from public.profiles where id = '${ids.bob}'`)).rows[0]).toEqual({ n: 1 });
    await expect(call(ids.owner, 'public.admin_purge_user_data($1)', [ids.owner])).rejects.toThrow(/cannot be purged/);
  });

  it('statistics are computed and every change above is in the audit log', async () => {
    const s = await call<any>(ids.owner, 'public.admin_system_stats()'); // eslint-disable-line @typescript-eslint/no-explicit-any
    expect(s.users).toBeGreaterThanOrEqual(3);
    expect(typeof s.messages).toBe('number');
    const actions = (await db.query<{ action: string }>(`select distinct action from public.admin_audit`)).rows.map((r) => r.action);
    for (const a of ['set_access', 'access_approve', 'access_decline', 'invite_emails', 'revoke_invite', 'set_user_limits', 'purge_user_data']) expect(actions).toContain(a);
  });
});
