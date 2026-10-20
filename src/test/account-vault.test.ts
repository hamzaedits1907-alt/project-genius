import { webcrypto } from 'node:crypto';
import { describe, expect, it } from 'vitest';
import { AccountVault, WrongPassphrase, checkPassphrase, type VaultRemote, type VaultRow } from '../web/accountVault';

const subtle = webcrypto.subtle as SubtleCrypto;

/** The server as the vault sees it: it only ever holds what it is given (here, a plain object). */
function fakeServer() {
  let row: VaultRow | null = null;
  const remote: VaultRemote = {
    get: async () => (row ? structuredClone(row) : null),
    create: async (r) => {
      if (row) throw new Error('duplicate');
      row = { ...r, entries: {} };
    },
    setEntries: async (entries) => {
      if (!row) throw new Error('no vault');
      row = { ...row, entries: structuredClone(entries) };
    },
    remove: async () => void (row = null),
  };
  return { remote, dump: () => JSON.stringify(row) };
}
const make = () => {
  const s = fakeServer();
  return { s, vault: new AccountVault(s.remote, subtle) };
};

describe('account key vault (client-side encryption)', () => {
  it('stores provider keys so that the server holds no readable key, and another device can read them back', async () => {
    const { s, vault } = make();
    const key = await vault.create('correct horse battery staple');
    await vault.put(key, 'nvidia', 'nvapi-SECRET-VALUE-1234567890');
    await vault.put(key, 'openai', 'sk-proj-ANOTHER-SECRET-0987654321');
    const stored = s.dump();
    expect(stored).not.toContain('SECRET');
    expect(stored).not.toContain('nvapi-');
    expect(stored).not.toContain('sk-proj');
    // a second device: only the passphrase
    const phone = new AccountVault(s.remote, subtle);
    const k2 = await phone.unlock('correct horse battery staple');
    const { keys, unreadable } = await phone.readAll(k2);
    expect(keys).toEqual({ nvidia: 'nvapi-SECRET-VALUE-1234567890', openai: 'sk-proj-ANOTHER-SECRET-0987654321' });
    expect(unreadable).toEqual([]);
  });

  it('refuses a wrong passphrase and does not reveal anything', async () => {
    const { vault } = make();
    await vault.create('correct horse battery staple');
    await expect(vault.unlock('wrong passphrase 123')).rejects.toBeInstanceOf(WrongPassphrase);
  });

  it('every save uses a fresh IV, so equal keys do not give equal ciphertext', async () => {
    const { s, vault } = make();
    const key = await vault.create('a long enough passphrase');
    await vault.put(key, 'nvidia', 'same-key-value-123456');
    const a = JSON.parse(s.dump()).entries.nvidia.ct;
    await vault.put(key, 'nvidia', 'same-key-value-123456');
    expect(JSON.parse(s.dump()).entries.nvidia.ct).not.toBe(a);
  });

  it('a ciphertext copied into another provider slot cannot be read (authenticated data)', async () => {
    const { s, vault } = make();
    const key = await vault.create('a long enough passphrase');
    await vault.put(key, 'nvidia', 'nvidia-key-value-123456');
    const row = JSON.parse(s.dump());
    await s.remote.setEntries({ ...row.entries, openai: row.entries.nvidia });
    const { keys, unreadable } = await vault.readAll(key);
    expect(keys.nvidia).toBe('nvidia-key-value-123456');
    expect(keys.openai).toBeUndefined();
    expect(unreadable).toEqual(['openai']);
  });

  it("saving one device's key does not erase another device's keys, and delete removes just one", async () => {
    const { s, vault } = make();
    const key = await vault.create('a long enough passphrase');
    const other = new AccountVault(s.remote, subtle);
    const k2 = await other.unlock('a long enough passphrase');
    await vault.put(key, 'nvidia', 'nvidia-key-value-123456');
    await other.put(k2, 'gemini', 'gemini-key-value-123456');
    expect(Object.keys((await vault.readAll(key)).keys).sort()).toEqual(['gemini', 'nvidia']);
    await vault.delete(key, 'nvidia');
    const after = await other.readAll(k2);
    expect(Object.keys(after.keys)).toEqual(['gemini']);
    expect(after.removed).toEqual(['nvidia']); // a tombstone: other devices drop their copy instead of re-uploading it
  });

  it('changing the passphrase re-encrypts everything; the old passphrase stops working', async () => {
    const { vault } = make();
    const key = await vault.create('the first passphrase');
    await vault.put(key, 'nvidia', 'nvidia-key-value-123456');
    const k2 = await vault.changePassphrase('the first passphrase', 'the second passphrase');
    expect((await vault.readAll(k2)).keys.nvidia).toBe('nvidia-key-value-123456');
    await expect(vault.unlock('the first passphrase')).rejects.toBeInstanceOf(WrongPassphrase);
    expect((await vault.readAll(await vault.unlock('the second passphrase'))).keys.nvidia).toBe('nvidia-key-value-123456');
  });

  it('a key remembered from an earlier vault no longer opens a reset one; reset removes the keys', async () => {
    const { vault } = make();
    const old = await vault.create('the first passphrase');
    await vault.put(old, 'nvidia', 'nvidia-key-value-123456');
    await vault.reset();
    expect(await vault.exists()).toBe(false);
    await vault.create('the second passphrase');
    await expect(vault.verify(old)).rejects.toBeInstanceOf(WrongPassphrase);
    expect((await vault.readAll(await vault.unlock('the second passphrase'))).keys).toEqual({});
  });

  it('rejects weak passphrases and a second vault', async () => {
    expect(checkPassphrase('short')).toMatch(/at least/);
    expect(checkPassphrase('aaaaaaaaaaaa')).toMatch(/simple/);
    expect(checkPassphrase('a perfectly fine passphrase')).toBeNull();
    const { vault } = make();
    await expect(vault.create('short')).rejects.toThrow(/at least/);
    await vault.create('a perfectly fine passphrase');
    await expect(vault.create('another fine passphrase')).rejects.toThrow(/already exists/);
  });
});
