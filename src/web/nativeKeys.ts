// Android and iOS: provider keys (and the key-vault passphrase, when "do not ask again" is chosen) live in the
// platform's secure storage — the Android Keystore (through encrypted preferences) and the iOS Keychain —
// instead of the WebView's IndexedDB. Nothing here runs in the browser or the desktop app.
import { Capacitor } from '@capacitor/core';
import type { ProviderId } from '../lib/types';
import type { PersistentKeyVault } from './keyvault';

export function isNativeApp(): boolean {
  try {
    return Capacitor.isNativePlatform();
  } catch {
    return false;
  }
}

type Secure = { set(key: string, value: string): Promise<void>; get(key: string): Promise<unknown>; remove(key: string): Promise<boolean | void> };
let plugin: Promise<Secure> | null = null;
/** The native secure-storage plugin (loaded only inside the apps). */
export function secureStorage(): Promise<Secure> {
  plugin ??= import('@aparajita/capacitor-secure-storage').then((m) => {
    void m.SecureStorage.setKeyPrefix?.('genius_');
    return m.SecureStorage as unknown as Secure;
  });
  return plugin;
}

const PROVIDERS: ProviderId[] = ['nvidia', 'openai', 'anthropic', 'gemini', 'custom'];

export function createNativeKeyVault(): PersistentKeyVault {
  const mem = new Map<ProviderId, string>();
  let chain: Promise<void> = Promise.resolve();
  const enqueue = (task: () => Promise<void>) => {
    chain = chain.then(task, task);
  };
  const ready = (async () => {
    try {
      const s = await secureStorage();
      for (const p of PROVIDERS) {
        const v = await s.get(`key.${p}`).catch(() => null);
        if (typeof v === 'string' && v) mem.set(p, v);
      }
    } catch {
      /* secure storage unavailable: keys last for this session only */
    }
  })();
  return {
    ready,
    canRemember: () => true,
    remembered: (p) => mem.has(p),
    idle: () => chain,
    get: (p) => mem.get(p) ?? null,
    set: (p, key) => {
      mem.set(p, key);
      enqueue(async () => void (await (await secureStorage()).set(`key.${p}`, key).catch(() => undefined)));
    },
    remove: (p) => {
      mem.delete(p);
      enqueue(async () => void (await (await secureStorage()).remove(`key.${p}`).catch(() => undefined)));
    },
  };
}
