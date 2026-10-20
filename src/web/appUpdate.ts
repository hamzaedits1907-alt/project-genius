// Update check for the Android app (the Windows app has its own updater). The app asks GitHub for the newest
// published release; when it carries a newer APK a banner offers the download. Android installs a downloaded APK
// itself after the user confirms: an app cannot silently replace itself unless it comes from a store.
import { create } from 'zustand';

declare const __APP_VERSION__: string | undefined;
export const APP_VERSION: string = typeof __APP_VERSION__ === 'string' ? __APP_VERSION__ : '0.0.0';
const RELEASES = 'https://api.github.com/repos/hamzaedits1907-alt/project-genius/releases/latest';
const DISMISSED = 'genius.update.dismissed';

/** 1 when a is newer than b, -1 when older, 0 when equal ("0.10.0" > "0.9.2"; a "v" prefix and a "-suffix" are ignored). */
export function compareVersions(a: string, b: string): number {
  const parts = (v: string) => v.replace(/^v/i, '').split('-')[0].split('.').map((x) => parseInt(x, 10) || 0);
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length, 3); i++) {
    const d = (x[i] ?? 0) - (y[i] ?? 0);
    if (d !== 0) return d > 0 ? 1 : -1;
  }
  return 0;
}

export interface UpdateInfo {
  version: string;
  url: string;
  name: string;
  notes: string;
  /** The APK is signed with a throw-away debug key: Android refuses to install it over an app signed with another key. */
  debugSigned: boolean;
}

interface Release {
  tag_name?: string;
  body?: string;
  assets?: { name: string; browser_download_url: string }[];
}

/** The newest release's APK when it is newer than `current`; null when up to date or nothing could be read. */
export async function newerRelease(current: string, fetchImpl: typeof fetch = fetch): Promise<UpdateInfo | null> {
  const res = await fetchImpl(RELEASES, { headers: { Accept: 'application/vnd.github+json' } });
  if (!res.ok) throw new Error(`GitHub answered ${res.status}`);
  const rel = (await res.json()) as Release;
  const version = (rel.tag_name ?? '').replace(/^v/i, '');
  const apk = rel.assets?.find((a) => /\.apk$/i.test(a.name));
  if (!version || !apk || compareVersions(version, current) <= 0) return null;
  return { version, url: apk.browser_download_url, name: apk.name, notes: (rel.body ?? '').split('\n---')[0].trim().slice(0, 1200), debugSigned: /-debug\.apk$/i.test(apk.name) };
}

interface UpdateState {
  checking: boolean;
  info: UpdateInfo | null;
  dismissed: string | null;
  error: string | null;
  lastChecked: string | null;
  check: (manual?: boolean) => Promise<void>;
  dismiss: () => void;
}

const readDismissed = () => {
  try {
    return localStorage.getItem(DISMISSED);
  } catch {
    return null;
  }
};

export const useAppUpdate = create<UpdateState>((set, get) => ({
  checking: false,
  info: null,
  dismissed: readDismissed(),
  error: null,
  lastChecked: null,
  check: async (manual = false) => {
    if (get().checking) return;
    set({ checking: true, error: null });
    try {
      const info = await newerRelease(APP_VERSION);
      set({ info, lastChecked: new Date().toISOString(), dismissed: manual ? null : get().dismissed });
    } catch (e) {
      set({ error: manual ? `Could not check for updates: ${e instanceof Error ? e.message : String(e)}` : null });
    } finally {
      set({ checking: false });
    }
  },
  dismiss: () => {
    const v = get().info?.version ?? null;
    try {
      if (v) localStorage.setItem(DISMISSED, v);
    } catch {
      /* the banner returns next start */
    }
    set({ dismissed: v });
  },
}));

/** Open the APK download in the system browser (the app itself cannot install it). */
export function openUpdate(url: string) {
  const w = window.open(url, '_system');
  if (!w) location.href = url;
}

let timer: ReturnType<typeof setInterval> | null = null;
/** Check now, every 6 hours, and whenever the app comes back to the foreground. */
export function startUpdateChecks(): () => void {
  void useAppUpdate.getState().check();
  timer ??= setInterval(() => void useAppUpdate.getState().check(), 6 * 3600_000);
  const onBack = () => {
    if (document.visibilityState === 'visible') void useAppUpdate.getState().check();
  };
  document.addEventListener('visibilitychange', onBack);
  return () => {
    document.removeEventListener('visibilitychange', onBack);
    if (timer) clearInterval(timer);
    timer = null;
  };
}
