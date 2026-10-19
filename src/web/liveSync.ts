// Keeps a signed-in client current when the same account changes on another device: Supabase Realtime
// (when the database publishes changes) plus a visibility/online/interval refresh as the reliable fallback.
// A refresh only READS; it is skipped while this client is itself in the middle of an answer.
import type { SupabaseClient } from '@supabase/supabase-js';
import { useChat } from '../store/chat';

export interface LiveRefreshOptions {
  refresh?: () => Promise<void>;
  busy?: () => boolean;
  pollMs?: number;
  debounceMs?: number;
}

/** Re-read the conversation list and the open conversation from the store. */
export async function refreshChatFromStore(): Promise<void> {
  const st = useChat.getState();
  await st.refresh('chat').catch(() => undefined);
  await useChat.getState().refresh('research').catch(() => undefined);
}

const localBusy = () => useChat.getState().tasks.some((t) => t.kind === 'chat' || t.kind === 'research');

/** Starts watching; returns the function that stops everything. */
export function startLiveRefresh(sb: SupabaseClient | null, userId: string, opts: LiveRefreshOptions = {}): () => void {
  const refresh = opts.refresh ?? refreshChatFromStore;
  const busy = opts.busy ?? localBusy;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let running = false;
  let again = false;
  const run = async () => {
    if (busy()) return;
    if (running) {
      again = true;
      return;
    }
    running = true;
    try {
      await refresh();
    } catch {
      /* a failed refresh is retried by the next trigger */
    } finally {
      running = false;
      if (again) {
        again = false;
        trigger();
      }
    }
  };
  const trigger = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => void run(), opts.debounceMs ?? 400);
  };
  const onVisible = () => {
    if (typeof document === 'undefined' || document.visibilityState === 'visible') trigger();
  };
  let channel: ReturnType<SupabaseClient['channel']> | null = null;
  try {
    channel = sb
      ?.channel(`genius-sync-${userId}`)
      .on('postgres_changes', { event: '*', schema: 'public', table: 'conversations', filter: `user_id=eq.${userId}` }, trigger)
      .on('postgres_changes', { event: '*', schema: 'public', table: 'messages', filter: `user_id=eq.${userId}` }, trigger)
      .subscribe() ?? null;
  } catch {
    channel = null; // realtime unavailable: polling still covers it
  }
  const poll = setInterval(() => {
    if (typeof document === 'undefined' || document.visibilityState === 'visible') trigger();
  }, opts.pollMs ?? 30_000);
  if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisible);
  if (typeof window !== 'undefined') window.addEventListener('online', trigger);
  return () => {
    if (timer) clearTimeout(timer);
    clearInterval(poll);
    if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisible);
    if (typeof window !== 'undefined') window.removeEventListener('online', trigger);
    if (channel) void sb?.removeChannel(channel);
  };
}
