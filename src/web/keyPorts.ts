// How the key vault reaches this device's own key storage.
import { invoke } from '@tauri-apps/api/core';
import { api } from '../lib/api';
import type { ProviderId } from '../lib/types';
import { useApp } from '../store/app';
import type { PersistentKeyVault } from './keyvault';
import type { LocalKeys } from './keySync';

const refreshStatus = () => void api.keyStatus().then((k) => useApp.getState().setKeyStatus(k)).catch(() => undefined);

/** Browser (web and the mobile apps): the encrypted per-device store, remembered across sessions. */
export function webLocalKeys(vault: PersistentKeyVault): LocalKeys {
  return {
    get: async (p) => {
      await vault.ready;
      return vault.get(p);
    },
    set: async (p, key) => {
      vault.set(p, key, { remember: vault.canRemember() });
      await vault.idle();
    },
    remove: async (p) => {
      vault.remove(p);
      await vault.idle();
    },
    changed: refreshStatus,
  };
}

/** Desktop: the operating system's credential store. */
export function desktopLocalKeys(): LocalKeys {
  return {
    get: (p: ProviderId) => invoke<string | null>('api_key_for', { provider: p }),
    set: (p: ProviderId, key: string) => invoke<void>('save_api_key_for', { provider: p, key }),
    remove: (p: ProviderId) => invoke<void>('remove_api_key_for', { provider: p }),
    changed: refreshStatus,
  };
}
