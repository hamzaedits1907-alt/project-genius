// Opt-in LIVE test against the real Supabase project (skipped unless the variables are set). Run with:
//   GENIUS_LIVE_URL=… GENIUS_LIVE_ANON=… GENIUS_LIVE_EMAIL=… GENIUS_LIVE_PASSWORD=… npx vitest run src/test/live-account.test.ts
// It signs in as a test account and proves, on the real backend: the account key vault stores only
// ciphertext and a second "device" can open it, conversations pushed like the desktop does come back through
// the sync functions, and a second account cannot see them.
import { webcrypto } from 'node:crypto';
import { createClient } from '@supabase/supabase-js';
import { describe, expect, it } from 'vitest';
import { AccountVault } from '../web/accountVault';
import { supabaseRemote } from '../web/keySync';

const { GENIUS_LIVE_URL: url, GENIUS_LIVE_ANON: anon, GENIUS_LIVE_EMAIL: email, GENIUS_LIVE_PASSWORD: password } = process.env;
const live = !!(url && anon && email && password);

describe.skipIf(!live)('live account (real Supabase)', () => {
  const client = () => createClient(url!, anon!, { auth: { persistSession: false, autoRefreshToken: false } });

  it('key vault: ciphertext only on the server, readable from a second device with the passphrase', async () => {
    const a = client();
    const { data, error } = await a.auth.signInWithPassword({ email: email!, password: password! });
    expect(error).toBeNull();
    const uid = data.user!.id;
    const subtle = webcrypto.subtle as SubtleCrypto;
    const deviceA = new AccountVault(supabaseRemote(a, uid), subtle);
    await deviceA.reset(); // start clean (test account)
    const key = await deviceA.create('live test passphrase 123');
    await deviceA.put(key, 'nvidia', 'nvapi-LIVE-TEST-KEY-0123456789abcdef');
    const raw = await a.from('key_vault').select('*').single();
    expect(JSON.stringify(raw.data)).not.toContain('LIVE-TEST-KEY');
    expect(JSON.stringify(raw.data)).not.toContain('nvapi-');

    const b = client();
    await b.auth.signInWithPassword({ email: email!, password: password! });
    const deviceB = new AccountVault(supabaseRemote(b, uid), subtle);
    await expect(deviceB.unlock('wrong passphrase 999')).rejects.toThrow(/does not open/);
    const kb = await deviceB.unlock('live test passphrase 123');
    expect((await deviceB.readAll(kb)).keys.nvidia).toBe('nvapi-LIVE-TEST-KEY-0123456789abcdef');
    await deviceB.delete(kb, 'nvidia');
    expect((await deviceA.readAll(key)).removed).toEqual(['nvidia']);
    await deviceA.reset();
    expect(await deviceA.exists()).toBe(false);
  }, 60_000);

  it('the database refuses a plaintext key in the vault', async () => {
    const a = client();
    await a.auth.signInWithPassword({ email: email!, password: password! });
    const uid = (await a.auth.getUser()).data.user!.id;
    const r = supabaseRemote(a, uid);
    await r.remove();
    await r.create({ kdf_salt: 'AAAAAAAAAAAAAAAAAAAAAA==', kdf_iterations: 600000, verifier_iv: 'AAAAAAAAAAAAAAAA', verifier_ct: 'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' });
    await expect(r.setEntries({ openai: { iv: 'AAAAAAAAAAAAAAAA', ct: 'sk-proj-abcdefghijklmnopqrstuvwxyz0123' } })).rejects.toThrow();
    await r.remove();
  }, 60_000);

  it('conversations pushed like the desktop does are pulled back, deleted ones become tombstones, and ids of another account are untouchable', async () => {
    const a = client();
    await a.auth.signInWithPassword({ email: email!, password: password! });
    const cid = crypto.randomUUID();
    const mid = crypto.randomUUID();
    const now = new Date().toISOString();
    const conv = { id: cid, mode: 'chat', title: 'live sync test', origin: null, createdAt: now, updatedAt: now, metaUpdatedAt: now };
    const msg = { id: mid, conversationId: cid, role: 'user', kind: 'chat', content: 'pushed from the desktop path', meta: {}, taskId: null, createdAt: now, updatedAt: now };
    expect((await a.rpc('sync_push', { p_conversations: [conv], p_messages: [msg] })).error).toBeNull();
    expect((await a.rpc('sync_push', { p_conversations: [conv], p_messages: [msg] })).error).toBeNull(); // retry: idempotent
    const pulled = (await a.rpc('sync_pull_messages', { p_after_ts: new Date(Date.now() - 120_000).toISOString(), p_after_id: null, p_limit: 300 })).data as { id: string; content: string }[];
    expect(pulled.filter((m) => m.id === mid)).toHaveLength(1);
    const list = (await a.rpc('conversation_summaries', { p_limit: 300 })).data as { id: string; message_count: number; snippet: string }[];
    expect(list.find((c) => c.id === cid)).toMatchObject({ message_count: 1, snippet: 'pushed from the desktop path' });

    // another account: sees nothing of it and cannot overwrite it
    const other = client();
    const up = await other.auth.signUp({ email: `genius.qa.other.${Date.now()}@example.com`, password: `Other-${Date.now()}-Pw!` });
    expect(up.data.session).not.toBeNull();
    expect(((await other.rpc('conversation_summaries', { p_limit: 300 })).data as { id: string }[]).some((c) => c.id === cid)).toBe(false);
    await other.rpc('sync_push', { p_conversations: [{ ...conv, title: 'hijacked', metaUpdatedAt: new Date(Date.now() + 60_000).toISOString() }], p_messages: [] });
    const still = (await a.rpc('conversation_summaries', { p_limit: 300 })).data as { id: string; title: string }[];
    expect(still.find((c) => c.id === cid)?.title).toBe('live sync test');

    expect((await a.rpc('delete_conversation', { p_id: cid })).error).toBeNull();
    const after = (await a.rpc('sync_pull_conversations', { p_after_ts: new Date(Date.now() - 120_000).toISOString(), p_after_id: null, p_limit: 200 })).data as { id: string; deletedAt: string | null }[];
    expect(after.find((c) => c.id === cid)?.deletedAt).not.toBeNull();
    expect(((await a.rpc('conversation_summaries', { p_limit: 300 })).data as { id: string }[]).some((c) => c.id === cid)).toBe(false);
  }, 90_000);
});
