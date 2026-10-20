// The account key vault: AI provider keys stored in the user's Genius account, encrypted on the device with a
// key derived from a vault passphrase that never leaves it. The server (Supabase) holds ciphertext only.
//
//   passphrase --PBKDF2-SHA256 (600,000 rounds, random salt)--> AES-GCM-256 key (non-extractable)
//   each provider key --AES-GCM (fresh random IV, provider name as authenticated data)--> ciphertext
//
// Wrong passphrase: the "verifier" (a known phrase encrypted at creation) fails to decrypt. There is no
// recovery path by design: the owner of the database cannot read the keys, so they cannot restore them.
import type { ProviderId } from '../lib/types';

export const PROVIDER_IDS: ProviderId[] = ['nvidia', 'openai', 'anthropic', 'gemini', 'custom'];
export const KDF_ITERATIONS = 600_000;
export const MIN_PASSPHRASE = 8;
const VERIFIER_TEXT = 'genius-vault-v1';

export interface VaultEntry { iv: string; ct: string; at?: string }
export interface VaultRow {
  kdf_salt: string;
  kdf_iterations: number;
  verifier_iv: string;
  verifier_ct: string;
  entries: Partial<Record<ProviderId, VaultEntry>>;
  updated_at?: string;
}
/** Where the encrypted row lives (Supabase in the app; a map in tests). */
export interface VaultRemote {
  get(): Promise<VaultRow | null>;
  create(row: Omit<VaultRow, 'entries' | 'updated_at'>): Promise<void>;
  setEntries(entries: VaultRow['entries']): Promise<void>;
  remove(): Promise<void>;
}

export class WrongPassphrase extends Error {
  constructor() {
    super('That passphrase does not open your key vault.');
  }
}

const enc = new TextEncoder();
const dec = new TextDecoder();
export const b64 = (buf: ArrayBuffer | Uint8Array) => {
  const bytes = buf instanceof Uint8Array ? buf : new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
export const unb64 = (s: string) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const aad = (p: string) => enc.encode(`genius-vault:${p}`);

export function checkPassphrase(p: string): string | null {
  if (p.normalize('NFKC').length < MIN_PASSPHRASE) return `Use at least ${MIN_PASSPHRASE} characters.`;
  if (/^(.)\1+$/.test(p)) return 'That passphrase is too simple.';
  return null;
}

export class AccountVault {
  constructor(private remote: VaultRemote, private subtle: SubtleCrypto = crypto.subtle, private random: (n: number) => Uint8Array = (n) => crypto.getRandomValues(new Uint8Array(n))) {}

  async exists(): Promise<boolean> {
    return (await this.remote.get()) !== null;
  }

  private async derive(passphrase: string, salt: Uint8Array, iterations: number): Promise<CryptoKey> {
    const base = await this.subtle.importKey('raw', enc.encode(passphrase.normalize('NFKC')), 'PBKDF2', false, ['deriveKey']);
    // extractable = false: the derived key can encrypt and decrypt in this browser but can never be read out
    return this.subtle.deriveKey({ name: 'PBKDF2', hash: 'SHA-256', salt: salt as BufferSource, iterations }, base, { name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
  }

  private async seal(key: CryptoKey, text: string, ad: Uint8Array): Promise<{ iv: string; ct: string }> {
    const iv = this.random(12);
    const ct = await this.subtle.encrypt({ name: 'AES-GCM', iv: iv as BufferSource, additionalData: ad as BufferSource }, key, enc.encode(text));
    return { iv: b64(iv), ct: b64(ct) };
  }

  private async open(key: CryptoKey, e: { iv: string; ct: string }, ad: Uint8Array): Promise<string> {
    return dec.decode(await this.subtle.decrypt({ name: 'AES-GCM', iv: unb64(e.iv) as BufferSource, additionalData: ad as BufferSource }, key, unb64(e.ct) as BufferSource));
  }

  /** Create the vault with this passphrase. Returns the key so the caller can stay unlocked. */
  async create(passphrase: string): Promise<CryptoKey> {
    const bad = checkPassphrase(passphrase);
    if (bad) throw new Error(bad);
    if (await this.exists()) throw new Error('A key vault already exists for this account. Unlock it, or reset it first.');
    const salt = this.random(16);
    const key = await this.derive(passphrase, salt, KDF_ITERATIONS);
    const v = await this.seal(key, VERIFIER_TEXT, aad('verifier'));
    await this.remote.create({ kdf_salt: b64(salt), kdf_iterations: KDF_ITERATIONS, verifier_iv: v.iv, verifier_ct: v.ct });
    return key;
  }

  /** Open the vault. Throws WrongPassphrase when the passphrase is not the right one. */
  async unlock(passphrase: string): Promise<CryptoKey> {
    const row = await this.remote.get();
    if (!row) throw new Error('There is no key vault for this account yet.');
    const key = await this.derive(passphrase, unb64(row.kdf_salt), row.kdf_iterations);
    await this.verify(key, row);
    return key;
  }

  /** Check a key that was remembered on this device (it may belong to an older vault that was since reset). */
  async verify(key: CryptoKey, row?: VaultRow | null): Promise<void> {
    const r = row ?? (await this.remote.get());
    if (!r) throw new Error('There is no key vault for this account yet.');
    try {
      if ((await this.open(key, { iv: r.verifier_iv, ct: r.verifier_ct }, aad('verifier'))) !== VERIFIER_TEXT) throw new WrongPassphrase();
    } catch {
      throw new WrongPassphrase();
    }
  }

  /**
   * Every provider key in the vault, decrypted. A removed key is a tombstone (an encrypted empty value), so
   * other devices learn about the removal instead of uploading their old copy again. An entry that cannot be
   * read is left out and reported.
   */
  async readAll(key: CryptoKey): Promise<{ keys: Partial<Record<ProviderId, string>>; unreadable: ProviderId[]; removed: ProviderId[] }> {
    const row = await this.remote.get();
    const keys: Partial<Record<ProviderId, string>> = {};
    const unreadable: ProviderId[] = [];
    const removed: ProviderId[] = [];
    for (const p of PROVIDER_IDS) {
      const e = row?.entries?.[p];
      if (!e) continue;
      try {
        const v = await this.open(key, e, aad(p));
        if (v === '') removed.push(p);
        else keys[p] = v;
      } catch {
        unreadable.push(p);
      }
    }
    return { keys, unreadable, removed };
  }

  /** Store (or replace) one provider's key. Re-reads the row first so another device's keys are not lost. */
  async put(key: CryptoKey, provider: ProviderId, secret: string, at = new Date().toISOString()): Promise<void> {
    const row = await this.remote.get();
    if (!row) throw new Error('There is no key vault for this account yet.');
    await this.verify(key, row);
    const sealed = await this.seal(key, secret, aad(provider));
    await this.remote.setEntries({ ...row.entries, [provider]: { ...sealed, at } });
  }

  async delete(key: CryptoKey, provider: ProviderId): Promise<void> {
    const row = await this.remote.get();
    if (!row?.entries?.[provider]) return;
    await this.verify(key, row);
    await this.remote.setEntries({ ...row.entries, [provider]: { ...(await this.seal(key, '', aad(provider))), at: new Date().toISOString() } });
  }

  /** Re-encrypt everything under a new passphrase. Other devices must unlock again with the new one. */
  async changePassphrase(oldPassphrase: string, newPassphrase: string): Promise<CryptoKey> {
    const bad = checkPassphrase(newPassphrase);
    if (bad) throw new Error(bad);
    const oldKey = await this.unlock(oldPassphrase);
    const { keys } = await this.readAll(oldKey);
    const salt = this.random(16);
    const key = await this.derive(newPassphrase, salt, KDF_ITERATIONS);
    const v = await this.seal(key, VERIFIER_TEXT, aad('verifier'));
    const entries: VaultRow['entries'] = {};
    for (const p of PROVIDER_IDS) if (keys[p] !== undefined) entries[p] = { ...(await this.seal(key, keys[p]!, aad(p))), at: new Date().toISOString() };
    await this.remote.remove();
    await this.remote.create({ kdf_salt: b64(salt), kdf_iterations: KDF_ITERATIONS, verifier_iv: v.iv, verifier_ct: v.ct });
    await this.remote.setEntries(entries);
    return key;
  }

  /** Forgotten passphrase: delete the vault (and so every saved key). The keys must be entered again. */
  async reset(): Promise<void> {
    await this.remote.remove();
  }
}
