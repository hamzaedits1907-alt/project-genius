// Row level security and the admin boundary, tested against a REAL PostgreSQL engine (PGlite, WASM)
// with the real migrations applied. Supabase itself is not involved: the `auth` schema is a minimal
// stand-in whose auth.uid() reads the same claim a Supabase request carries. This proves the policies
// and functions behave as designed; it does NOT prove a configured Supabase project is wired correctly
// (that needs the live checklist in docs/DEPLOY-NETLIFY-SUPABASE.md).
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { PGlite } from '@electric-sql/pglite';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const dir = join(__dirname, '..', '..', 'supabase', 'migrations');
const sql = (f: string) => readFileSync(join(dir, f), 'utf8');

let db: PGlite;
const ids = { alice: '', bob: '', owner: '' };

async function as<T>(user: string | null, fn: () => Promise<T>): Promise<T> {
  await db.exec(`set role ${user ? 'authenticated' : 'anon'}; select set_config('request.jwt.claim.sub', '${user ?? ''}', false);`);
  try {
    return await fn();
  } finally {
    await db.exec(`reset role; select set_config('request.jwt.claim.sub', '', false);`);
  }
}
const q = async <R = Record<string, unknown>>(text: string, params: unknown[] = []) => (await db.query<R>(text, params)).rows;
const fails = async (p: Promise<unknown>, re: RegExp) => {
  await expect(p).rejects.toThrow(re);
};

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
  await db.exec(sql('20261019000000_genius_web.sql'));
  await db.exec(sql('20261019000100_genius_web_storage.sql'));
  await db.exec(sql('20261020000000_admin_advanced.sql'));
  ids.alice = (await q<{ id: string }>(`insert into auth.users (email) values ('alice@example.com') returning id`))[0].id;
  ids.bob = (await q<{ id: string }>(`insert into auth.users (email) values ('bob@example.com') returning id`))[0].id;
  ids.owner = (await q<{ id: string }>(`insert into auth.users (email) values ('owner@example.com') returning id`))[0].id;
  await db.exec(`insert into public.admin_users (user_id) values ('${ids.owner}')`);
});
afterAll(async () => {
  await db.close();
});

describe('profiles', () => {
  it('are created by the sign-up trigger and each user sees only their own', async () => {
    expect(await q(`select count(*)::int as n from public.profiles`)).toEqual([{ n: 3 }]);
    const own = await as(ids.alice, () => q<{ email: string }>(`select email from public.profiles`));
    expect(own).toEqual([{ email: 'alice@example.com' }]);
  });

  it('cannot be used to escalate: status and email are not editable by the user', async () => {
    await as(ids.alice, () => db.query(`update public.profiles set display_name = 'Alice' where id = '${ids.alice}'`));
    await fails(as(ids.alice, () => db.query(`update public.profiles set status = 'active', email = 'x@y.z' where id = '${ids.alice}'`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`update public.profiles set status = 'suspended' where id = '${ids.alice}'`)), /permission denied/);
    expect((await q<{ display_name: string }>(`select display_name from public.profiles where id = '${ids.alice}'`))[0].display_name).toBe('Alice');
  });

  it('give nobody the administrator role from the client', async () => {
    await fails(as(ids.alice, () => db.query(`insert into public.admin_users (user_id) values ('${ids.alice}')`)), /permission denied/);
    expect(await as(ids.alice, async () => (await q(`select public.is_admin() as a`))[0])).toEqual({ a: false });
    await fails(as(ids.alice, () => db.query(`select * from public.admin_list_users('', 10)`)), /administrator access required/);
  });
});

describe('user data is private', () => {
  let convA = '';
  it('owner can create and read; another user sees nothing', async () => {
    convA = (await as(ids.alice, () => q<{ id: string }>(`insert into public.conversations (mode, title) values ('chat', 'Alice secrets') returning id`)))[0].id;
    await as(ids.alice, () => db.query(`insert into public.messages (conversation_id, role, content) values ($1, 'user', 'my private question')`, [convA]));
    expect((await as(ids.alice, () => q(`select * from public.messages`))).length).toBe(1);
    for (const t of ['conversations', 'messages', 'research_sources', 'attachments', 'usage_events', 'user_preferences']) {
      expect(await as(ids.bob, () => q(`select * from public.${t}`)), t).toEqual([]);
    }
  });

  it('another user cannot change, delete or write into it', async () => {
    expect((await as(ids.bob, () => db.query(`update public.conversations set title = 'hacked' where id = $1`, [convA]))).affectedRows).toBe(0);
    expect((await as(ids.bob, () => db.query(`delete from public.conversations where id = $1`, [convA]))).affectedRows).toBe(0);
    await fails(as(ids.bob, () => db.query(`insert into public.messages (conversation_id, role, content) values ($1, 'user', 'injected')`, [convA])), /does not belong/);
    await fails(as(ids.bob, () => db.query(`insert into public.messages (conversation_id, user_id, role, content) values ($1, $2, 'user', 'spoofed')`, [convA, ids.alice])), /row-level security/);
    await fails(as(ids.bob, () => db.query(`insert into public.research_sources (conversation_id, n, url, requested_url, content_hash) values ($1, 1, 'https://a.example', 'https://a.example', 'h')`, [convA])), /does not belong/);
    await fails(as(ids.bob, () => db.query(`insert into public.attachments (conversation_id, name, kind, mime, size) values ($1, 'x.txt', 'text', 'text/plain', 1)`, [convA])), /does not belong/);
    expect((await q<{ title: string }>(`select title from public.conversations where id = '${convA}'`))[0].title).toBe('Alice secrets');
  });

  it('is invisible to signed-out visitors', async () => {
    await fails(as(null, () => db.query(`select * from public.conversations`)), /permission denied/);
    await fails(as(null, () => db.query(`select * from public.profiles`)), /permission denied/);
  });

  it('deleting a conversation removes its messages and sources', async () => {
    await as(ids.alice, () => db.query(`insert into public.research_sources (conversation_id, n, url, requested_url, content_hash) values ($1, 1, 'https://a.example', 'https://a.example', 'h')`, [convA]));
    await as(ids.alice, () => db.query(`delete from public.conversations where id = $1`, [convA]));
    expect(await q(`select * from public.messages where conversation_id = '${convA}'`)).toEqual([]);
    expect(await q(`select * from public.research_sources where conversation_id = '${convA}'`)).toEqual([]);
  });
});

describe('preferences never hold secrets', () => {
  it('stores ordinary settings and rejects anything that looks like a provider key', async () => {
    await as(ids.alice, () => db.query(`insert into public.user_preferences (settings) values ('{"theme":"dark","model":"claude-sonnet-5-5","provider":"anthropic"}')`));
    for (const bad of [`{"apiKey":"abc"}`, `{"k":"sk-abcdefghijklmnopqrstuvwx"}`, `{"nvidia":"nvapi-ABCDEFGHIJKLMNOP"}`, `{"a":"sk-ant-abcdefghijklmnop"}`, `{"g":"AIzaSyDabcdefghijklmnopqrstuvwxyz"}`, `{"h":"Bearer abcdefghijklmnopqrstuvwxyz0123"}`]) {
      await fails(as(ids.alice, () => db.query(`update public.user_preferences set settings = $1::jsonb`, [bad])), /must not contain provider keys/);
    }
    expect((await as(ids.bob, () => q(`select * from public.user_preferences`))).length).toBe(0);
  });
});

describe('plans, grants and flags: a user can read their own state but never change it', () => {
  it('has no client path to a plan, grant, flag, cohort or audit write', async () => {
    await fails(as(ids.alice, () => db.query(`insert into public.entitlements (user_id, plan) values ('${ids.alice}', 'pro')`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`insert into public.feature_grants (user_id, feature) values ('${ids.alice}', 'agent_mode')`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`update public.feature_flags set kill = false`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`insert into public.admin_audit (actor, action) values ('${ids.alice}', 'x')`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`select public.admin_set_plan('${ids.alice}', 'pro')`)), /administrator access required/);
    await fails(as(ids.alice, () => db.query(`select public.admin_log('x', null, '{}')`)), /permission denied/);
    expect(await as(ids.alice, () => q(`select * from public.admin_audit`))).toEqual([]);
  });

  it('a free user sees Pro features as needing Pro', async () => {
    const a = (await as(ids.alice, () => q<{ a: { plan: string; features: Record<string, { available: boolean; reason: string }> } }>(`select public.my_access() as a`)))[0].a;
    expect(a.plan).toBe('free');
    expect(a.features.chat).toMatchObject({ available: true, reason: 'ok' });
    expect(a.features.agent_mode).toMatchObject({ available: false, reason: 'needs_pro' });
  });
});

describe('the owner admin boundary', () => {
  it('lets an administrator search users and make Bob Pro, with an audit record', async () => {
    const users = await as(ids.owner, () => q<{ email: string }>(`select * from public.admin_list_users('bo', 10)`));
    expect(users.map((u) => u.email)).toEqual(['bob@example.com']);
    await as(ids.owner, () => db.query(`select public.admin_set_plan($1, 'pro', null, 'beta tester')`, [ids.bob]));
    const bob = (await as(ids.bob, () => q<{ a: { plan: string; features: Record<string, { available: boolean }> } }>(`select public.my_access() as a`)))[0].a;
    expect(bob.plan).toBe('pro');
    expect(bob.features.agent_mode.available).toBe(true);
    const audit = await as(ids.owner, () => q<{ action: string; actor: string; target_user: string }>(`select action, actor, target_user from public.admin_audit order by id`));
    expect(audit).toEqual([{ action: 'set_plan', actor: ids.owner, target_user: ids.bob }]);
    expect(await as(ids.bob, () => q(`select * from public.admin_audit`))).toEqual([]);
  });

  it('expired Pro falls back to free', async () => {
    await as(ids.owner, () => db.query(`select public.admin_set_plan($1, 'pro', now() - interval '1 day', 'expired')`, [ids.alice]));
    const a = (await as(ids.alice, () => q<{ a: { plan: string } }>(`select public.my_access() as a`)))[0].a;
    expect(a.plan).toBe('free');
    await as(ids.owner, () => db.query(`select public.admin_set_plan($1, 'free')`, [ids.alice]));
  });

  it('feature grants (temporary access) give one feature without Pro', async () => {
    await as(ids.owner, () => db.query(`select public.admin_grant_feature($1, 'agent_mode', now() + interval '7 days', 'trial')`, [ids.alice]));
    let a = (await as(ids.alice, () => q<{ a: { plan: string; features: Record<string, { available: boolean }> } }>(`select public.my_access() as a`)))[0].a;
    expect(a.plan).toBe('free');
    expect(a.features.agent_mode.available).toBe(true);
    expect(a.features.custom_agents.available).toBe(false);
    await as(ids.owner, () => db.query(`select public.admin_grant_feature($1, 'agent_mode', now() - interval '1 minute', 'expired')`, [ids.alice]));
    a = (await as(ids.alice, () => q<{ a: { features: Record<string, { available: boolean }> } }>(`select public.my_access() as a`)))[0].a as never;
    expect(a.features.agent_mode.available).toBe(false);
    await as(ids.owner, () => db.query(`select public.admin_revoke_feature($1, 'agent_mode')`, [ids.alice]));
  });

  it('a kill switch turns a feature off for everyone, even Pro; a rollout and a cohort stage it', async () => {
    const reason = async (u: string) => (await as(u, () => q<{ a: { features: Record<string, { available: boolean; reason: string }> } }>(`select public.my_access() as a`)))[0].a.features.agent_mode;
    await as(ids.owner, () => db.query(`select public.admin_set_flag('agent_mode', true, true, 100, null, 'incident')`));
    expect(await reason(ids.bob)).toMatchObject({ available: false, reason: 'switched_off' });
    await as(ids.owner, () => db.query(`select public.admin_set_flag('agent_mode', true, false, 0, null, 'staged')`));
    expect(await reason(ids.bob)).toMatchObject({ available: false, reason: 'not_rolled_out' });
    await as(ids.owner, () => db.query(`select public.admin_save_cohort('early', 'Early access', 'first testers')`));
    await as(ids.owner, () => db.query(`select public.admin_set_flag('agent_mode', true, false, 0, 'early', 'cohort only')`));
    expect(await reason(ids.bob)).toMatchObject({ available: false });
    await as(ids.owner, () => db.query(`select public.admin_cohort_member('early', $1, true)`, [ids.bob]));
    expect(await reason(ids.bob)).toMatchObject({ available: true, reason: 'ok' });
    await as(ids.owner, () => db.query(`select public.admin_set_flag('agent_mode', true, false, 100, null, 'open')`));
    expect(await reason(ids.bob)).toMatchObject({ available: true });
  });

  it('suspension blocks writes and is reported; an admin cannot suspend themselves or another admin', async () => {
    await fails(as(ids.owner, () => db.query(`select public.admin_set_status($1, 'suspended', 'oops')`, [ids.owner])), /cannot change your own/);
    await as(ids.owner, () => db.query(`select public.admin_set_status($1, 'suspended', 'abuse')`, [ids.bob]));
    await fails(as(ids.bob, () => db.query(`insert into public.conversations (mode) values ('chat')`)), /row-level security/);
    const a = (await as(ids.bob, () => q<{ a: { status: string; features: Record<string, { reason: string }> } }>(`select public.my_access() as a`)))[0].a;
    expect(a.status).toBe('suspended');
    expect(a.features.chat.reason).toBe('suspended');
    await as(ids.owner, () => db.query(`select public.admin_set_status($1, 'active')`, [ids.bob]));
    await as(ids.bob, () => db.query(`insert into public.conversations (mode) values ('chat')`));
  });

  it('a suspended administrator loses administrator powers immediately', async () => {
    await db.exec(`update public.profiles set status = 'suspended' where id = '${ids.owner}'`);
    await fails(as(ids.owner, () => db.query(`select * from public.admin_list_users('', 5)`)), /administrator access required/);
    await db.exec(`update public.profiles set status = 'active' where id = '${ids.owner}'`);
  });

  it('usage summary is aggregate only (no message content)', async () => {
    const s = (await as(ids.owner, () => q<{ s: Record<string, unknown> }>(`select public.admin_usage_summary() as s`)))[0].s;
    expect(Object.keys(s).sort()).toEqual(['activeLast7Days', 'conversations', 'eventsLast7Days', 'messages', 'pro', 'suspended', 'users']);
    expect(s.users).toBe(3);
  });
});

describe('attachment storage', () => {
  it('lets a user write only inside their own folder', async () => {
    await as(ids.alice, () => db.query(`insert into storage.objects (bucket_id, name) values ('attachments', '${ids.alice}/pic.png')`));
    await fails(as(ids.alice, () => db.query(`insert into storage.objects (bucket_id, name) values ('attachments', '${ids.bob}/pic.png')`)), /row-level security/);
    expect(await as(ids.bob, () => q(`select * from storage.objects`))).toEqual([]);
    expect((await q<{ public: boolean }>(`select public from storage.buckets where id = 'attachments'`))[0].public).toBe(false);
  });
});

describe('advanced administration', () => {
  const rpc = <R = Record<string, unknown>>(user: string | null, text: string, params: unknown[] = []) => as(user, () => q<R>(text, params));
  type Access = { a: { plan: string; features: Record<string, { reason: string }>; maintenance: { message: string } | null } };
  const access = async (user: string) => ((await rpc(user, `select public.my_access() as a`))[0] as Access).a;

  it('is closed to ordinary users and to visitors', async () => {
    for (const call of [
      `select public.admin_set_setting('maintenance', '{"enabled": true, "message": "x"}')`,
      `select * from public.admin_search_users('', 'all', 'all', 'created', 10, 0)`,
      `select public.admin_user_detail('${ids.bob}')`,
      `select public.admin_bulk_set_plan(array['${ids.bob}']::uuid[], 'pro')`,
      `select * from public.admin_usage_series(7)`,
      `select * from public.admin_list_admins()`,
      `select public.admin_set_note('${ids.bob}', 'hello')`,
    ]) {
      await fails(rpc(ids.alice, call), /administrator access required/);
    }
    await fails(rpc(null, `select * from public.admin_search_users()`), /permission denied/);
    await fails(as(ids.alice, () => db.query(`update public.app_settings set value = '{"open": true}' where key = 'signups'`)), /permission denied/);
    await fails(as(ids.alice, () => db.query(`insert into public.announcements (title) values ('spam')`)), /permission denied/);
    expect(await rpc(ids.alice, `select * from public.admin_notes`)).toEqual([]);
  });

  it('searches with filters, paging and a total, and shows one user in detail', async () => {
    const all = await rpc(ids.owner, `select * from public.admin_search_users('', 'all', 'all', 'created', 2, 0)`);
    expect(all).toHaveLength(2);
    expect(Number((all[0] as { total_count: unknown }).total_count)).toBe(3);
    const pros = await rpc<{ plan: string }>(ids.owner, `select plan from public.admin_search_users('', 'pro', 'all', 'created', 10, 0)`);
    expect(pros.every((r) => r.plan === 'pro')).toBe(true);
    expect(await rpc(ids.owner, `select email from public.admin_search_users('alice', 'all', 'all', 'email', 10, 0)`)).toEqual([{ email: 'alice@example.com' }]);
    await rpc(ids.owner, `select public.admin_set_note('${ids.alice}', 'asked for early access')`);
    const d = (await rpc(ids.owner, `select public.admin_user_detail('${ids.alice}') as d`))[0] as { d: { profile: { email: string }; note: string; audit: unknown[]; isAdmin: boolean } };
    expect(d.d.profile.email).toBe('alice@example.com');
    expect(d.d.note).toBe('asked for early access');
    expect(d.d.isAdmin).toBe(false);
    expect(d.d.audit.length).toBeGreaterThan(0);
    expect(await rpc(ids.alice, `select * from public.admin_notes`)).toEqual([]);
  });

  it('changes plans in bulk, bounded, and writes one audit record', async () => {
    const before = Number((await rpc(ids.owner, `select count(*) n from public.admin_audit`))[0].n);
    const n = (await rpc(ids.owner, `select public.admin_bulk_set_plan(array['${ids.alice}','${ids.bob}']::uuid[], 'pro', null, 'beta') as n`))[0] as { n: number };
    expect(n.n).toBe(2);
    expect(Number((await rpc(ids.owner, `select count(*) n from public.admin_audit`))[0].n)).toBe(before + 1);
    expect((await access(ids.alice)).plan).toBe('pro');
    await rpc(ids.owner, `select public.admin_bulk_set_plan(array['${ids.alice}','${ids.bob}']::uuid[], 'free')`);
    const many = `array[${Array.from({ length: 201 }, () => `'${ids.bob}'`).join(',')}]::uuid[]`;
    await fails(rpc(ids.owner, `select public.admin_bulk_set_plan(${many}, 'pro')`), /at most 200/);
  });

  it('maintenance mode switches features off for users but not for the owner', async () => {
    await rpc(ids.owner, `select public.admin_set_setting('maintenance', '{"enabled": true, "message": "Back at 18:00"}', 'upgrade')`);
    const a = await access(ids.alice);
    expect(a.features.chat.reason).toBe('maintenance');
    expect(a.maintenance?.message).toBe('Back at 18:00');
    expect((await access(ids.owner)).features.chat.reason).toBe('ok');
    await rpc(ids.owner, `select public.admin_set_setting('maintenance', '{"enabled": false, "message": ""}')`);
    expect((await access(ids.alice)).features.chat.reason).toBe('ok');
  });

  it('can close registrations and reopen them', async () => {
    await rpc(ids.owner, `select public.admin_set_setting('signups', '{"open": false}')`);
    await fails(db.query(`insert into auth.users (email) values ('late@example.com')`), /registrations are closed/i);
    await rpc(ids.owner, `select public.admin_set_setting('signups', '{"open": true}')`);
    await db.query(`insert into auth.users (email) values ('late@example.com')`);
    expect(Number((await q(`select count(*) n from public.profiles where email = 'late@example.com'`))[0].n)).toBe(1);
  });

  it('refuses settings that contain secrets or negative limits', async () => {
    await fails(rpc(ids.owner, `select public.admin_set_setting('limits_free', '{"note": "nvapi-123"}')`), /secrets|non-negative/);
    await fails(rpc(ids.owner, `select public.admin_set_setting('limits_free', '{"maxModelCalls": -1}')`), /non-negative/);
    await fails(rpc(ids.owner, `select public.admin_set_setting('anything_else', '{}')`), /unknown setting/);
  });

  it('enforces a plan-specific daily cap on recorded Genius usage', async () => {
    await rpc(ids.owner, `select public.admin_set_setting('limits_free', '{"dailyAgentMissions": 1, "dailyWebToolCalls": null}')`);
    await as(ids.alice, () => db.query(`insert into public.usage_events (kind) values ('agent')`));
    await fails(as(ids.alice, () => db.query(`insert into public.usage_events (kind) values ('agent')`)), /Daily limit reached/);
    await as(ids.alice, () => db.query(`insert into public.usage_events (kind) values ('chat')`)); // kinds without a cap are untouched
    await rpc(ids.owner, `select public.admin_bulk_set_plan(array['${ids.alice}']::uuid[], 'pro')`);
    await as(ids.alice, () => db.query(`insert into public.usage_events (kind) values ('agent')`)); // Pro: no mission cap by default
    await rpc(ids.owner, `select public.admin_bulk_set_plan(array['${ids.alice}']::uuid[], 'free')`);
    const limits = ((await rpc(ids.alice, `select public.my_context() as c`))[0] as { c: { limits: { dailyAgentMissions: number } } }).c.limits;
    expect(limits.dailyAgentMissions).toBe(1);
  });

  it('shows users only the announcements that are in force', async () => {
    const id = ((await rpc(ids.owner, `select public.admin_save_announcement(null, 'Planned downtime', 'Friday 18:00', 'warning', true, null) as id`))[0] as { id: string }).id;
    await rpc(ids.owner, `select public.admin_save_announcement(null, 'Draft', '', 'info', false, null)`);
    expect(await rpc(ids.bob, `select title from public.announcements`)).toEqual([{ title: 'Planned downtime' }]);
    expect(((await rpc(ids.bob, `select public.my_context() as c`))[0] as { c: { announcements: unknown[] } }).c.announcements).toHaveLength(1);
    expect((await rpc(ids.owner, `select * from public.admin_list_announcements()`)).length).toBe(2);
    await rpc(ids.owner, `select public.admin_delete_announcement('${id}')`);
    expect(await rpc(ids.bob, `select title from public.announcements`)).toEqual([]);
  });

  it('reports usage as counts, lists administrators read-only and deletes cohorts cleanly', async () => {
    expect((await rpc(ids.owner, `select day, kind, n from public.admin_usage_series(7)`)).length).toBeGreaterThan(0);
    expect(await rpc(ids.owner, `select email from public.admin_list_admins()`)).toEqual([{ email: 'owner@example.com' }]);
    await fails(as(ids.owner, () => db.query(`insert into public.admin_users (user_id) values ('${ids.bob}')`)), /permission denied/);
    await rpc(ids.owner, `select public.admin_save_cohort('beta', 'Beta testers')`);
    await rpc(ids.owner, `select public.admin_bulk_cohort('beta', array['${ids.bob}']::uuid[], true)`);
    await rpc(ids.owner, `select public.admin_set_flag('agent_mode', true, false, 0, 'beta', 'beta only')`);
    await rpc(ids.owner, `select public.admin_delete_cohort('beta')`);
    expect((await q(`select cohort_id from public.feature_flags where key = 'agent_mode'`))[0]).toEqual({ cohort_id: null });
    await rpc(ids.owner, `select public.admin_set_flag('agent_mode', true, false, 100, null, 'restore')`);
  });
});
