// The key vault table on a REAL PostgreSQL engine (PGlite): private per account, and it refuses anything
// that is not ciphertext. (The encryption itself is tested in src/test/account-vault.test.ts.)
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { PGlite } from '@electric-sql/pglite';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

const dir = join(__dirname, '..', '..', 'supabase', 'migrations');
let db: PGlite;
const ids = { alice: '', bob: '' };

async function as<T>(user: string, fn: () => Promise<T>): Promise<T> {
  await db.exec(`set role authenticated; select set_config('request.jwt.claim.sub', '${user}', false);`);
  try {
    return await fn();
  } finally {
    await db.exec(`reset role; select set_config('request.jwt.claim.sub', '', false);`);
  }
}
const ct = (n: number) => Buffer.from(`ciphertext-${n}-`.padEnd(40, 'x')).toString('base64');
const vaultRow = (entries: object = {}) => ({
  salt: Buffer.from('0123456789abcdef').toString('base64'),
  verifier: ct(0),
  entries: JSON.stringify(entries),
});
const insert = (user: string, e: object = {}) =>
  as(user, () => {
    const v = vaultRow(e);
    return db.query(`insert into public.key_vault (kdf_salt, kdf_iterations, verifier_iv, verifier_ct, entries) values ($1, 600000, 'AAAAAAAAAAAAAAAA', $2, $3::jsonb)`, [v.salt, v.verifier, v.entries]);
  });

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
  for (const f of ['20261019000000_genius_web.sql', '20261019000100_genius_web_storage.sql', '20261019000200_missions.sql', '20261020000000_admin_advanced.sql', '20261021000000_sync.sql', '20261022000000_key_vault.sql']) await db.exec(readFileSync(join(dir, f), 'utf8'));
  ids.alice = (await db.query<{ id: string }>(`insert into auth.users (email) values ('alice@example.com') returning id`)).rows[0].id;
  ids.bob = (await db.query<{ id: string }>(`insert into auth.users (email) values ('bob@example.com') returning id`)).rows[0].id;
});
afterAll(async () => {
  await db.close();
});

describe('key vault', () => {
  it('stores ciphertext for the owner and nobody else can read, replace or delete it', async () => {
    await insert(ids.alice, { nvidia: { iv: 'AAAAAAAAAAAAAAAA', ct: ct(1), at: '2026-10-20T10:00:00Z' } });
    expect((await as(ids.alice, () => db.query(`select entries from public.key_vault`))).rows).toHaveLength(1);
    expect((await as(ids.bob, () => db.query(`select entries from public.key_vault`))).rows).toHaveLength(0);
    await as(ids.bob, () => db.query(`update public.key_vault set entries = '{}'::jsonb`));
    await as(ids.bob, () => db.query(`delete from public.key_vault`));
    expect((await db.query(`select entries from public.key_vault where user_id = '${ids.alice}'`)).rows[0]).toMatchObject({ entries: { nvidia: { ct: ct(1) } } });
    await expect(as(ids.bob, () => db.query(`insert into public.key_vault (user_id, kdf_salt, kdf_iterations, verifier_iv, verifier_ct) values ('${ids.alice}', '${vaultRow().salt}', 600000, 'AAAAAAAAAAAAAAAA', '${ct(0)}')`))).rejects.toThrow();
  });

  it('a user has one vault and can update and reset it', async () => {
    await expect(insert(ids.alice)).rejects.toThrow(/duplicate key|unique/);
    await as(ids.alice, () => db.query(`update public.key_vault set entries = $1::jsonb`, [JSON.stringify({ openai: { iv: 'AAAAAAAAAAAAAAAA', ct: ct(2) } })]));
    expect((await db.query(`select entries from public.key_vault where user_id = '${ids.alice}'`)).rows[0]).toMatchObject({ entries: { openai: { ct: ct(2) } } });
    await as(ids.alice, () => db.query(`delete from public.key_vault`));
    expect((await db.query(`select count(*)::int as n from public.key_vault`)).rows[0]).toEqual({ n: 0 });
  });

  it('refuses a plaintext key, an unknown provider, malformed entries and oversized vaults', async () => {
    await expect(insert(ids.bob, { openai: { iv: 'AAAAAAAAAAAAAAAA', ct: 'sk-proj-abcdefghijklmnopqrstuvwxyz' } })).rejects.toThrow(/base64|encrypted/);
    await expect(insert(ids.bob, { nvidia: { iv: 'AAAAAAAAAAAAAAAA', ct: ct(1), note: 'nvapi-abcdefghijklmnop' } })).rejects.toThrow(/encrypted/);
    await expect(insert(ids.bob, { shady: { iv: 'AAAAAAAAAAAAAAAA', ct: ct(1) } })).rejects.toThrow(/unknown provider/);
    await expect(insert(ids.bob, { nvidia: { ct: ct(1) } })).rejects.toThrow(/iv and a ciphertext/);
    await expect(insert(ids.bob, { nvidia: { iv: 'AAAAAAAAAAAAAAAA', ct: 'A'.repeat(3000) } })).rejects.toThrow();
  });

  it('a suspended account cannot create a vault', async () => {
    await db.exec(`update public.profiles set status = 'suspended' where id = '${ids.bob}'`);
    await expect(insert(ids.bob)).rejects.toThrow();
    await db.exec(`update public.profiles set status = 'active' where id = '${ids.bob}'`);
    await insert(ids.bob);
  });
});
