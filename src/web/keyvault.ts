// Where the user's AI provider key lives in the browser.
//
// Two honest choices, made by the user:
//   * "This session only": the key is kept in sessionStorage and memory; closing the tab forgets it.
//   * "Remember on this device": the key is encrypted with AES-GCM using a NON-EXPORTABLE Web Crypto key
//     that sits in IndexedDB next to it, so the saved text is not readable by looking at the browser's
//     storage files and the master key cannot be copied out of the browser.
//
// What this does NOT do, and the UI says so: any script running on this page can still ask the browser to
// decrypt the key, so a cross-site-scripting bug would expose it. The defences are the strict content
// security policy of the app (scripts only from this site), no third-party scripts, and keeping the key
// out of every server: it is never sent to Supabase or to a Genius backend and is never synced between
// devices. It goes only to the provider (or, for providers that block browsers, through the relay that
// forwards it and stores nothing).
import type { ProviderId } from '../lib/types';
import type { KeyVault } from './runtime';

export interface SecretDb {
  getMaster(): Promise<CryptoKey | null>;
  putMaster(k: CryptoKey): Promise<void>;
  getRecord(id: string): Promise<{ iv: ArrayBuffer; data: ArrayBuffer } | null>;
  putRecord(id: string, r: { iv: ArrayBuffer; data: ArrayBuffer }): Promise<void>;
  deleteRecord(id: string): Promise<void>;
  listRecordIds(): Promise<string[]>;
}

const DB_NAME = 'genius-keyvault';

/** IndexedDB-backed storage. Rejects when IndexedDB is unavailable (private windows, blocked storage). */
export function indexedDbSecretDb(factory: IDBFactory = indexedDB): SecretDb {
  const open = () =>
    new Promise<IDBDatabase>((resolve, reject) => {
      const req = factory.open(DB_NAME, 1);
      req.onupgradeneeded = () => {
        req.result.createObjectStore('meta');
        req.result.createObjectStore('keys');
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error ?? new Error('IndexedDB is not available'));
    });
  const run = async <T>(store: 'meta' | 'keys', mode: IDBTransactionMode, fn: (s: IDBObjectStore) => IDBRequest<T>): Promise<T> => {
    const db = await open();
    try {
      return await new Promise<T>((resolve, reject) => {
        const tx = db.transaction(store, mode);
        const req = fn(tx.objectStore(store));
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
      });
    } finally {
      db.close();
    }
  };
  return {
    getMaster: async () => ((await run<CryptoKey | undefined>('meta', 'readonly', (s) => s.get('master'))) ?? null),
    putMaster: async (k) => void (await run('meta', 'readwrite', (s) => s.put(k, 'master'))),
    getRecord: async (id) => ((await run<{ iv: ArrayBuffer; data: ArrayBuffer } | undefined>('keys', 'readonly', (s) => s.get(id))) ?? null),
    putRecord: async (id, r) => void (await run('keys', 'readwrite', (s) => s.put(r, id))),
    deleteRecord: async (id) => void (await run('keys', 'readwrite', (s) => s.delete(id))),
    listRecordIds: async () => (await run<IDBValidKey[]>('keys', 'readonly', (s) => s.getAllKeys())).map(String),
  };
}

/** In-memory storage (tests, and the fallback when IndexedDB cannot be used). */
export function memorySecretDb(): SecretDb {
  let master: CryptoKey | null = null;
  const recs = new Map<string, { iv: ArrayBuffer; data: ArrayBuffer }>();
  return {
    getMaster: async () => master,
    putMaster: async (k) => void (master = k),
    getRecord: async (id) => recs.get(id) ?? null,
    putRecord: async (id, r) => void recs.set(id, r),
    deleteRecord: async (id) => void recs.delete(id),
    listRecordIds: async () => [...recs.keys()],
  };
}

export interface PersistentKeyVault extends KeyVault {
  /** Resolves once remembered keys have been decrypted into memory. Await before the first request. */
  ready: Promise<void>;
  /** Is this provider's key stored (encrypted) on this device? */
  remembered(p: ProviderId): boolean;
  /** Can keys be remembered here at all (IndexedDB + Web Crypto available)? */
  canRemember(): boolean;
  /** Resolves when pending writes to the encrypted store have finished. */
  idle(): Promise<void>;
}

const PROVIDERS: ProviderId[] = ['nvidia', 'openai', 'anthropic', 'gemini', 'custom'];
const enc = new TextEncoder();
const dec = new TextDecoder();

export function createKeyVault(opts: { db?: SecretDb; subtle?: SubtleCrypto; session?: Storage | null } = {}): PersistentKeyVault {
  const subtle = opts.subtle ?? (typeof crypto !== 'undefined' ? crypto.subtle : undefined);
  let db: SecretDb | null = opts.db ?? null;
  if (!db) {
    try {
      db = typeof indexedDB !== 'undefined' ? indexedDbSecretDb() : null;
    } catch {
      db = null;
    }
  }
  let session: Storage | null = null;
  try {
    session = opts.session === undefined ? (typeof sessionStorage !== 'undefined' ? sessionStorage : null) : opts.session;
  } catch {
    session = null;
  }
  const mem = new Map<ProviderId, string>();
  const stored = new Set<ProviderId>();
  const skey = (p: ProviderId) => `genius.key.${p}`;
  const usable = !!db && !!subtle;

  async function master(create: boolean): Promise<CryptoKey | null> {
    if (!db || !subtle) return null;
    const existing = await db.getMaster();
    if (existing || !create) return existing;
    // extractable = false: the key material can be used by this browser profile but never read out.
    const k = await subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, ['encrypt', 'decrypt']);
    await db.putMaster(k);
    return k;
  }

  async function load(): Promise<void> {
    if (!db || !subtle) return;
    try {
      const k = await master(false);
      if (!k) return;
      for (const id of await db.listRecordIds()) {
        const p = id as ProviderId;
        if (!PROVIDERS.includes(p)) continue;
        const r = await db.getRecord(id);
        if (!r) continue;
        try {
          mem.set(p, dec.decode(await subtle.decrypt({ name: 'AES-GCM', iv: r.iv }, k, r.data)));
          stored.add(p);
        } catch {
          await db.deleteRecord(id); // unreadable (master key replaced): drop it rather than keep a dead secret
        }
      }
    } catch {
      /* storage blocked: behave as session-only */
    }
  }

  let chain: Promise<void> = Promise.resolve();
  const enqueue = (task: () => Promise<void>) => {
    chain = chain.then(task, task);
  };
  const ready = load();
  const vault: PersistentKeyVault = {
    ready,
    canRemember: () => usable,
    idle: () => chain,
    remembered: (p) => stored.has(p),
    get: (p) => {
      const m = mem.get(p);
      if (m) return m;
      try {
        return session?.getItem(skey(p)) ?? null;
      } catch {
        return null;
      }
    },
    set: (p, key, o) => {
      mem.set(p, key);
      const remember = !!o?.remember && usable;
      if (remember) {
        // The plaintext never touches sessionStorage when the user chose to remember it encrypted.
        try {
          session?.removeItem(skey(p));
        } catch {
          /* nothing stored */
        }
        enqueue(async () => {
          try {
            const k = await master(true);
            const iv = crypto.getRandomValues(new Uint8Array(12));
            const data = await subtle!.encrypt({ name: 'AES-GCM', iv }, k!, enc.encode(key));
            await db!.putRecord(p, { iv: iv.buffer as ArrayBuffer, data });
            stored.add(p);
          } catch {
            stored.delete(p); // could not persist: the key still works for this session
          }
        });
      } else {
        try {
          session?.setItem(skey(p), key);
        } catch {
          /* memory only */
        }
        if (stored.has(p)) {
          stored.delete(p);
          enqueue(async () => void (await db?.deleteRecord(p).catch(() => undefined)));
        }
      }
    },
    remove: (p) => {
      mem.delete(p);
      stored.delete(p);
      try {
        session?.removeItem(skey(p));
      } catch {
        /* nothing stored */
      }
      enqueue(async () => void (await db?.deleteRecord(p).catch(() => undefined)));
    },
  };
  return vault;
}
