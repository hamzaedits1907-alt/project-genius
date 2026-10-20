// Wires the sync algorithm (syncEngine.ts) to this computer's database (Tauri commands) and the account's
// Supabase functions, and runs it while a Genius account is signed in on the desktop app.
import { invoke } from '@tauri-apps/api/core';
import { create } from 'zustand';
import { refreshChatFromStore, startLiveRefresh } from './liveSync';
import { keyHooks } from '../lib/api';
import { sb, useAuth } from './auth';
import { onKeyRemoved, onKeySaved, startKeySync, stopKeySync } from './keySync';
import { desktopLocalKeys } from './keyPorts';
import { relink, syncOnce, type SyncPorts } from './syncEngine';

export interface SyncStatus {
  state: 'off' | 'idle' | 'syncing' | 'error' | 'paused';
  lastSyncAt: string | null;
  pending: number;
  error: string | null;
  warnings: string[];
}
export const useSync = create<SyncStatus>(() => ({ state: 'off', lastSyncAt: null, pending: 0, error: null, warnings: [] }));

const errText = (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));

function ports(): SyncPorts {
  const rpc = async (fn: string, args: Record<string, unknown>) => {
    const { data, error } = await sb().rpc(fn, args);
    if (error) throw error;
    return data;
  };
  return {
    get: (key) => invoke<string | null>('sync_get', { key }),
    set: (key, value) => invoke<void>('sync_set', { key, value }),
    enqueueAll: () => invoke<number>('sync_enqueue_all'),
    collect: (limit) => invoke('sync_collect', { limit }),
    ack: (keys) => invoke<void>('sync_ack', { keys }),
    apply: (batch) => invoke('sync_apply', { batch }),
    pullConversations: async (after, limit) => ((await rpc('sync_pull_conversations', { p_after_ts: after?.ts ?? null, p_after_id: after?.id ?? null, p_limit: limit })) ?? []) as Record<string, unknown>[],
    pullMessages: async (after, limit) => ((await rpc('sync_pull_messages', { p_after_ts: after?.ts ?? null, p_after_id: after?.id ?? null, p_limit: limit })) ?? []) as Record<string, unknown>[],
    push: async (conversations, messages) => void (await rpc('sync_push', { p_conversations: conversations, p_messages: messages })),
  };
}

let running: Promise<void> | null = null;
let userIdNow: string | null = null;

/** Run one sync now (single-flight). Never throws: problems are shown in the status. */
export function syncNow(): Promise<void> {
  if (running) return running;
  const uid = userIdNow;
  if (!uid) return Promise.resolve();
  useSync.setState({ state: 'syncing', error: null });
  running = (async () => {
    try {
      const r = await syncOnce(ports(), uid);
      const pending = await invoke<number>('sync_pending').catch(() => 0);
      useSync.setState({ state: r.paused ? 'paused' : 'idle', lastSyncAt: r.paused ? useSync.getState().lastSyncAt : new Date().toISOString(), pending, error: null, warnings: r.warnings.slice(0, 5) });
      if (r.changed) await refreshChatFromStore();
    } catch (e) {
      const msg = errText(e);
      const missing = /sync_pull|sync_push|PGRST202|42883|does not exist/i.test(msg);
      useSync.setState({ state: 'error', error: missing ? 'The sync update has not been applied to your Genius account database yet (supabase/migrations/20261021000000_sync.sql). Conversations stay on this computer until it is.' : msg });
    } finally {
      running = null;
    }
  })();
  return running;
}

/** The user chose to move this computer's conversations to the signed-in account. */
export async function relinkToAccount() {
  const uid = userIdNow;
  if (!uid) return;
  await relink(ports(), uid);
  await syncNow();
}

let stop: (() => void) | null = null;

/** Start (or restart) syncing for the signed-in user; stops when the user signs out. */
export function startDesktopSync(): void {
  const unsub = useAuth.subscribe((s, prev) => {
    const id = s.session?.user.id ?? null;
    if (id === (prev.session?.user.id ?? null)) return;
    begin(id);
  });
  begin(useAuth.getState().session?.user.id ?? null);
  void unsub;
}

function begin(id: string | null) {
  stop?.();
  stop = null;
  userIdNow = id;
  if (!id) {
    useSync.setState({ state: 'off', pending: 0, error: null });
    stopKeySync();
    keyHooks.saved = undefined;
    keyHooks.removed = undefined;
    return;
  }
  // The account key vault: provider keys follow the account (encrypted on this computer before they leave it)
  keyHooks.saved = (p, k) => void onKeySaved(p, k);
  keyHooks.removed = (p) => void onKeyRemoved(p);
  void startKeySync(sb(), id, desktopLocalKeys());
  const timers: ReturnType<typeof setInterval>[] = [];
  let lastRun = 0;
  const tick = async () => {
    if (document.visibilityState === 'hidden' && Date.now() - lastRun < 120_000) return;
    const pending = await invoke<number>('sync_pending').catch(() => 0);
    useSync.setState({ pending });
    // local changes go up within seconds; remote changes are looked for every ~25 s
    if (pending > 0 || Date.now() - lastRun > 25_000) {
      lastRun = Date.now();
      await syncNow();
    }
  };
  timers.push(setInterval(() => void tick(), 5000));
  void tick();
  // Realtime: another device just changed something
  const live = startLiveRefresh(sb(), id, { refresh: async () => void (await syncNow()), busy: () => false, pollMs: 3_600_000 });
  stop = () => {
    timers.forEach(clearInterval);
    live();
  };
}
