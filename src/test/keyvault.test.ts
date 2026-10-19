// The browser key vault: session-only vs remembered-on-this-device, with real Web Crypto (Node's
// implementation of the same API) and an in-memory stand-in for IndexedDB.
import { describe, expect, it } from 'vitest';
import { createKeyVault, memorySecretDb } from '../web/keyvault';

const KEY = 'nvapi-secret-0123456789abcdef';

function fakeSession(): Storage {
  const m = new Map<string, string>();
  return { getItem: (k) => m.get(k) ?? null, setItem: (k, v) => void m.set(k, v), removeItem: (k) => void m.delete(k), clear: () => m.clear(), key: () => null, get length() { return m.size; } } as Storage;
}

describe('key vault', () => {
  it('session-only keys live in the tab and never reach the encrypted store', async () => {
    const db = memorySecretDb();
    const session = fakeSession();
    const v = createKeyVault({ db, session });
    await v.ready;
    v.set('nvidia', KEY, { remember: false });
    await v.idle();
    expect(v.get('nvidia')).toBe(KEY);
    expect(v.remembered('nvidia')).toBe(false);
    expect(await db.listRecordIds()).toEqual([]);
    // a new "page load" in the same tab still has it; a new tab (fresh session) does not
    expect(createKeyVault({ db, session }).get('nvidia')).toBe(KEY);
    expect(createKeyVault({ db, session: fakeSession() }).get('nvidia')).toBeNull();
  });

  it('remembered keys survive a restart, are encrypted at rest and need the stored master key', async () => {
    const db = memorySecretDb();
    const v = createKeyVault({ db, session: fakeSession() });
    await v.ready;
    v.set('openai', KEY, { remember: true });
    await v.idle();
    expect(v.remembered('openai')).toBe(true);

    const rec = await db.getRecord('openai');
    expect(rec).not.toBeNull();
    const raw = new TextDecoder('latin1').decode(new Uint8Array(rec!.data));
    expect(raw).not.toContain('secret');
    expect(raw).not.toContain(KEY);
    const master = await db.getMaster();
    expect(master?.extractable).toBe(false);
    await expect(crypto.subtle.exportKey('raw', master!)).rejects.toThrow();

    // "restart": a new vault over the same storage, in a tab with an empty sessionStorage
    const again = createKeyVault({ db, session: fakeSession() });
    await again.ready;
    expect(again.get('openai')).toBe(KEY);
    expect(again.remembered('openai')).toBe(true);
  });

  it('replacing a remembered key with a session-only one deletes the stored copy', async () => {
    const db = memorySecretDb();
    const v = createKeyVault({ db, session: fakeSession() });
    v.set('gemini', KEY, { remember: true });
    await v.idle();
    expect(await db.listRecordIds()).toEqual(['gemini']);
    v.set('gemini', 'AIza-another-key-123456', { remember: false });
    await v.idle();
    expect(await db.listRecordIds()).toEqual([]);
    expect(v.remembered('gemini')).toBe(false);
  });

  it('removing a key forgets it everywhere', async () => {
    const db = memorySecretDb();
    const session = fakeSession();
    const v = createKeyVault({ db, session });
    v.set('anthropic', KEY, { remember: true });
    await v.idle();
    v.remove('anthropic');
    await v.idle();
    expect(v.get('anthropic')).toBeNull();
    expect(await db.listRecordIds()).toEqual([]);
    const fresh = createKeyVault({ db, session });
    await fresh.ready;
    expect(fresh.get('anthropic')).toBeNull();
  });

  it('drops a stored key it can no longer decrypt instead of keeping a dead secret', async () => {
    const db = memorySecretDb();
    const v = createKeyVault({ db, session: fakeSession() });
    v.set('custom', KEY, { remember: true });
    await v.idle();
    await db.putMaster(await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt'])); // master replaced
    const again = createKeyVault({ db, session: fakeSession() });
    await again.ready;
    expect(again.get('custom')).toBeNull();
    expect(await db.listRecordIds()).toEqual([]);
  });

  it('falls back to session-only when no secure storage exists', async () => {
    const v = createKeyVault({ db: undefined, subtle: undefined, session: fakeSession() });
    // jsdom has neither IndexedDB nor (here) a usable store: remember is requested but cannot be honoured
    v.set('nvidia', KEY, { remember: true });
    await v.idle();
    expect(v.get('nvidia')).toBe(KEY);
    if (!v.canRemember()) expect(v.remembered('nvidia')).toBe(false);
  });
});
