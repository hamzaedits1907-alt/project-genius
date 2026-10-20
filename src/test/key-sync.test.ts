// The key vault glue: two "devices" share one account row (a fake Supabase), keys move between them
// encrypted, removals propagate, and a wrong passphrase installs nothing.
import { afterEach, describe, expect, it } from 'vitest';
import type { SupabaseClient } from '@supabase/supabase-js';
import type { ProviderId } from '../lib/types';
import { keyVaultActions, onKeyRemoved, onKeySaved, reconcile, startKeySync, stopKeySync, useKeyVault, type LocalKeys } from '../web/keySync';

/** Just enough of the Supabase client for the key_vault table. */
function fakeSupabase() {
  let row: Record<string, unknown> | null = null;
  const q = (result: () => unknown) => {
    const p: any = Promise.resolve().then(result); // eslint-disable-line @typescript-eslint/no-explicit-any
    p.maybeSingle = () => p;
    p.eq = () => p;
    return p;
  };
  const client = {
    from: (table: string) => {
      if (table !== 'key_vault') throw new Error('unexpected table ' + table);
      return {
        select: () => q(() => ({ data: row ? structuredClone(row) : null, error: null })),
        insert: (r: Record<string, unknown>) => q(() => ((row = { ...r, entries: {} }), { error: null })),
        update: (r: Record<string, unknown>) => q(() => ((row = { ...row, ...r }), { error: null })),
        delete: () => q(() => ((row = null), { error: null })),
      };
    },
  };
  return { sb: client as unknown as SupabaseClient, dump: () => JSON.stringify(row) };
}

function device() {
  const store = new Map<ProviderId, string>();
  let changed = 0;
  const keys: LocalKeys = {
    get: async (p) => store.get(p) ?? null,
    set: async (p, k) => void store.set(p, k),
    remove: async (p) => void store.delete(p),
    changed: () => void changed++,
  };
  return { store, keys, changes: () => changed };
}

afterEach(() => stopKeySync());

describe('key vault sync between devices', () => {
  it('a key saved on the desktop appears on the phone after unlocking, and is never stored in readable form', async () => {
    const server = fakeSupabase();
    const desktop = device();
    await startKeySync(server.sb, 'u1', desktop.keys);
    expect(useKeyVault.getState().state).toBe('none');
    await keyVaultActions.create('a long enough passphrase', false);
    expect(useKeyVault.getState().state).toBe('unlocked');
    desktop.store.set('nvidia', 'nvapi-DESKTOP-KEY-0123456789');
    await onKeySaved('nvidia', 'nvapi-DESKTOP-KEY-0123456789');
    expect(server.dump()).not.toContain('DESKTOP-KEY');
    stopKeySync();

    const phone = device();
    await startKeySync(server.sb, 'u1', phone.keys);
    expect(useKeyVault.getState().state).toBe('locked');
    await keyVaultActions.unlock('wrong passphrase here', false);
    expect(useKeyVault.getState().state).toBe('locked');
    expect(useKeyVault.getState().error).toMatch(/does not open/);
    expect(phone.store.size).toBe(0);
    await keyVaultActions.unlock('a long enough passphrase', false);
    expect(useKeyVault.getState().state).toBe('unlocked');
    expect(phone.store.get('nvidia')).toBe('nvapi-DESKTOP-KEY-0123456789');
    expect(phone.changes()).toBe(1);
  });

  it('a key that only exists on this device joins the account when the vault is unlocked', async () => {
    const server = fakeSupabase();
    const a = device();
    a.store.set('openai', 'sk-LOCAL-ONLY-KEY-0123456789');
    await startKeySync(server.sb, 'u1', a.keys);
    await keyVaultActions.create('a long enough passphrase', false);
    expect(useKeyVault.getState().providers).toEqual(['openai']);
    stopKeySync();
    const b = device();
    await startKeySync(server.sb, 'u1', b.keys);
    await keyVaultActions.unlock('a long enough passphrase', false);
    expect(b.store.get('openai')).toBe('sk-LOCAL-ONLY-KEY-0123456789');
  });

  it('removing a key on one device removes it from the others at their next refresh (and it is not uploaded again)', async () => {
    const server = fakeSupabase();
    const a = device();
    await startKeySync(server.sb, 'u1', a.keys);
    await keyVaultActions.create('a long enough passphrase', false);
    a.store.set('gemini', 'AIza-GEMINI-KEY-0123456789');
    await onKeySaved('gemini', 'AIza-GEMINI-KEY-0123456789');
    stopKeySync();

    const b = device();
    await startKeySync(server.sb, 'u1', b.keys);
    await keyVaultActions.unlock('a long enough passphrase', false);
    expect(b.store.get('gemini')).toBeDefined();
    b.store.delete('gemini');
    await onKeyRemoved('gemini');
    stopKeySync();

    // the first device still has its copy: unlocking drops it instead of re-uploading it
    await startKeySync(server.sb, 'u1', a.keys);
    await keyVaultActions.unlock('a long enough passphrase', false);
    expect(a.store.has('gemini')).toBe(false);
    await reconcile();
    expect(useKeyVault.getState().providers).not.toContain('gemini');
  });

  it('says clearly when the table has not been created yet', async () => {
    const sb = { from: () => ({ select: () => Object.assign(Promise.resolve({ data: null, error: { message: 'relation "public.key_vault" does not exist', code: '42P01' } }), { maybeSingle() { return this; } }) }) } as unknown as SupabaseClient;
    await startKeySync(sb, 'u1', device().keys);
    expect(useKeyVault.getState().state).toBe('unavailable');
    expect(useKeyVault.getState().error).toMatch(/20261022000000_key_vault/);
  });
});
