// Keeps the provider keys of this device in step with the account's encrypted key vault (accountVault.ts).
//
// What the user sees: sign in on a new phone or browser, type the vault passphrase once, and every provider
// key is there; paste a key on any device and it appears on the others. What happens underneath: keys are
// encrypted on the device before they leave it, the server stores ciphertext only, and a key that is only on
// this device is uploaded when the vault is unlocked. The vault is authoritative: a key changed on another
// device replaces the local one at the next unlock or refresh.
import { create } from 'zustand';
import type { SupabaseClient } from '@supabase/supabase-js';
import type { ProviderId } from '../lib/types';
import { AccountVault, PROVIDER_IDS, WrongPassphrase, type VaultRemote, type VaultRow } from './accountVault';
import { isNativeApp, secureStorage, within } from './nativeKeys';

/** Where this device keeps its own keys (the OS credential store on the desktop, the encrypted browser store on the web). */
export interface LocalKeys {
  get(p: ProviderId): Promise<string | null>;
  set(p: ProviderId, key: string): Promise<void>;
  remove(p: ProviderId): Promise<void>;
  /** Tell the interface that keys changed (refresh the "key saved" state). */
  changed(): void;
}

export type VaultState = 'signed-out' | 'loading' | 'none' | 'locked' | 'unlocked' | 'unavailable';
interface KeyVaultState {
  state: VaultState;
  providers: ProviderId[];
  /** This device opens the vault without asking again. */
  remembered: boolean;
  busy: boolean;
  error: string | null;
  notice: string | null;
}
export const useKeyVault = create<KeyVaultState>(() => ({ state: 'signed-out', providers: [], remembered: false, busy: false, error: null, notice: null }));

// ---------------------------------------------------------------------------------------------- device key
const DB = 'genius-vaultkey';
function idb<T>(mode: IDBTransactionMode, fn: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => open.result.createObjectStore('k');
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const db = open.result;
      const req = fn(db.transaction('k', mode).objectStore('k'));
      req.onsuccess = () => {
        resolve(req.result);
        db.close();
      };
      req.onerror = () => {
        reject(req.error);
        db.close();
      };
    };
  });
}
/** What "do not ask again on this device" remembers: the derived key in the browser, the passphrase in the Keystore/Keychain inside the mobile apps. */
const deviceMemory = {
  async get(uid: string): Promise<{ key?: CryptoKey; passphrase?: string } | null> {
    if (isNativeApp()) {
      const v = await within(secureStorage().then((s) => s.get(`vaultpass.${uid}`)), 3000, null);
      return typeof v === 'string' && v ? { passphrase: v } : null;
    }
    const k = await deviceKey.get(uid);
    return k ? { key: k } : null;
  },
  async put(uid: string, m: { key: CryptoKey; passphrase?: string }): Promise<boolean> {
    if (isNativeApp()) {
      if (!m.passphrase) return false;
      return within(secureStorage().then((s) => s.set(`vaultpass.${uid}`, m.passphrase!)).then(() => true), 4000, false);
    }
    return deviceKey.put(uid, m.key);
  },
  async del(uid: string): Promise<void> {
    if (isNativeApp()) await within(secureStorage().then((s) => s.remove(`vaultpass.${uid}`)), 3000, undefined);
    else await deviceKey.del(uid);
  },
};
const deviceKey = {
  get: (uid: string) => idb<CryptoKey | undefined>('readonly', (s) => s.get(`vault:${uid}`)).catch(() => undefined),
  put: (uid: string, k: CryptoKey) => idb('readwrite', (s) => s.put(k, `vault:${uid}`)).then(() => true).catch(() => false),
  del: (uid: string) => idb('readwrite', (s) => s.delete(`vault:${uid}`)).catch(() => undefined),
};

// ---------------------------------------------------------------------------------------------- remote
export function supabaseRemote(sb: SupabaseClient, uid: string): VaultRemote {
  const fail = (e: { message: string; code?: string } | null): never => {
    const err = new Error(e?.message ?? 'unknown error') as Error & { code?: string };
    err.code = e?.code;
    throw err;
  };
  return {
    get: async () => {
      const { data, error } = await sb.from('key_vault').select('kdf_salt, kdf_iterations, verifier_iv, verifier_ct, entries, updated_at').maybeSingle();
      if (error) fail(error);
      return (data as VaultRow | null) ?? null;
    },
    create: async (row) => {
      const { error } = await sb.from('key_vault').insert(row);
      if (error) fail(error);
    },
    setEntries: async (entries) => {
      const { error } = await sb.from('key_vault').update({ entries }).eq('user_id', uid);
      if (error) fail(error);
    },
    remove: async () => {
      const { error } = await sb.from('key_vault').delete().eq('user_id', uid);
      if (error) fail(error);
    },
  };
}

// ---------------------------------------------------------------------------------------------- controller
let vault: AccountVault | null = null;
let local: LocalKeys | null = null;
let userId: string | null = null;
let key: CryptoKey | null = null;
let timer: ReturnType<typeof setInterval> | null = null;
let onFocus: (() => void) | null = null;

const msg = (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));
const missingTable = (e: unknown) => /key_vault|PGRST205|42P01|does not exist|schema cache/i.test(msg(e) + String((e as { code?: string })?.code ?? ''));
const set = (p: Partial<KeyVaultState>) => useKeyVault.setState(p);

/** Copy the vault to this device, and this device's own keys to the vault. */
export async function reconcile(): Promise<void> {
  if (!vault || !key || !local) return;
  const { keys, unreadable, removed } = await vault.readAll(key);
  let changed = false;
  const uploaded: ProviderId[] = [];
  for (const p of PROVIDER_IDS) {
    const have = await local.get(p);
    const inVault = keys[p];
    if (inVault && inVault !== have) {
      await local.set(p, inVault);
      changed = true;
    } else if (removed.includes(p)) {
      if (have) {
        await local.remove(p); // removed on another device
        changed = true;
      }
    } else if (!inVault && have && !unreadable.includes(p)) {
      await vault.put(key, p, have); // a key that exists only on this device joins the account
      uploaded.push(p);
    }
  }
  set({ providers: PROVIDER_IDS.filter((p) => keys[p] || uploaded.includes(p)), error: unreadable.length ? `Could not read the saved key for: ${unreadable.join(', ')}. Save it again.` : null });
  if (changed) local.changed();
}

async function openWith(k: CryptoKey, remember: boolean, passphrase?: string) {
  key = k;
  if (remember && userId) set({ remembered: await deviceMemory.put(userId, { key: k, passphrase }) });
  set({ state: 'unlocked', error: null });
  await reconcile();
  if (!timer) {
    timer = setInterval(() => void reconcile().catch(() => undefined), 5 * 60_000);
    onFocus = () => void reconcile().catch(() => undefined);
    if (typeof window !== 'undefined') window.addEventListener('focus', onFocus);
  }
}

/** Start for a signed-in user: find out whether a vault exists and open it with this device's remembered key. */
export async function startKeySync(sb: SupabaseClient, uid: string, keys: LocalKeys): Promise<void> {
  stopKeySync();
  userId = uid;
  local = keys;
  vault = new AccountVault(supabaseRemote(sb, uid));
  set({ state: 'loading', error: null, notice: null });
  try {
    const row = await supabaseRemote(sb, uid).get();
    if (!row) return set({ state: 'none', providers: [], remembered: false });
    const saved = await deviceMemory.get(uid);
    if (saved) {
      try {
        const k = saved.key ?? (await vault.unlock(saved.passphrase ?? ''));
        await vault.verify(k, row);
        set({ remembered: true });
        return await openWith(k, false);
      } catch {
        await deviceMemory.del(uid); // the passphrase was changed or the vault reset elsewhere
      }
    }
    set({ state: 'locked', providers: Object.keys(row.entries ?? {}) as ProviderId[], remembered: false });
  } catch (e) {
    set(missingTable(e) ? { state: 'unavailable', error: 'The key vault is not set up in your Genius account database yet (supabase/migrations/20261022000000_key_vault.sql).' } : { state: 'unavailable', error: msg(e) });
  }
}

export function stopKeySync() {
  if (timer) clearInterval(timer);
  timer = null;
  if (onFocus && typeof window !== 'undefined') window.removeEventListener('focus', onFocus);
  onFocus = null;
  key = null;
  vault = null;
  local = null;
  userId = null;
  set({ state: 'signed-out', providers: [], remembered: false, busy: false, error: null, notice: null });
}

async function guarded(fn: () => Promise<void>) {
  set({ busy: true, error: null, notice: null });
  try {
    await fn();
  } catch (e) {
    set({ error: e instanceof WrongPassphrase ? e.message : msg(e) });
  } finally {
    set({ busy: false });
  }
}

export const keyVaultActions = {
  create: (passphrase: string, remember: boolean) =>
    guarded(async () => {
      if (!vault) throw new Error('Sign in first.');
      await openWith(await vault.create(passphrase), remember, passphrase);
      set({ notice: 'Your key vault is ready. Keys you save now are available on every device after you unlock the vault there.' });
    }),
  unlock: (passphrase: string, remember: boolean) =>
    guarded(async () => {
      if (!vault) throw new Error('Sign in first.');
      await openWith(await vault.unlock(passphrase), remember, passphrase);
      set({ notice: 'Unlocked. Your saved keys are on this device now.' });
    }),
  /** Lock this device: forget the remembered key. Keys already copied to this device stay (they are in its own secure store). */
  lock: () =>
    guarded(async () => {
      if (userId) await deviceMemory.del(userId);
      key = null;
      set({ state: 'locked', remembered: false });
    }),
  change: (oldPassphrase: string, newPassphrase: string) =>
    guarded(async () => {
      if (!vault) throw new Error('Sign in first.');
      const k = await vault.changePassphrase(oldPassphrase, newPassphrase);
      await openWith(k, useKeyVault.getState().remembered, newPassphrase);
      set({ notice: 'Passphrase changed. Unlock the vault again on your other devices with the new passphrase.' });
    }),
  /** Forgotten passphrase: remove the vault and every key in it. */
  reset: () =>
    guarded(async () => {
      if (!vault) throw new Error('Sign in first.');
      await vault.reset();
      if (userId) await deviceMemory.del(userId);
      key = null;
      set({ state: 'none', providers: [], remembered: false, notice: 'The vault was deleted. Create a new one and save your keys again.' });
    }),
  refresh: () => guarded(reconcile),
};

/** A key was saved or replaced on this device (the user typed it): put it in the account vault too. */
export async function onKeySaved(provider: ProviderId, secret: string): Promise<void> {
  if (!vault || !key) return;
  try {
    await vault.put(key, provider, secret);
    set({ providers: [...new Set([...useKeyVault.getState().providers, provider])] });
  } catch (e) {
    set({ error: `The key is saved on this device but could not be saved to your account: ${msg(e)}` });
  }
}

export async function onKeyRemoved(provider: ProviderId): Promise<void> {
  if (!vault || !key) return;
  try {
    await vault.delete(key, provider);
    set({ providers: useKeyVault.getState().providers.filter((p) => p !== provider) });
  } catch (e) {
    set({ error: `The key was removed here but not from your account: ${msg(e)}` });
  }
}
