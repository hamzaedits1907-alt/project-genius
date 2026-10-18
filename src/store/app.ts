import { create } from 'zustand';
import { api, errorMessage } from '../lib/api';
import type { ApprovalRequest, AppInfo, KeyStatus, Settings, UpdateInfo } from '../lib/types';

export type Route =
  | { name: 'home' }
  | { name: 'create' }
  | { name: 'chat' }
  | { name: 'research' }
  | { name: 'settings'; tab?: SettingsTab }
  | { name: 'project'; id: string }
  | { name: 'agents'; tab?: 'agents' | 'missions' | 'teams' };

export type SettingsTab = 'ai' | 'github' | 'integrations' | 'approvals' | 'workspace' | 'appearance' | 'updates';

export interface Toast {
  id: number;
  tone: 'ok' | 'bad' | 'info';
  text: string;
}

interface AppState {
  route: Route;
  settings: Settings | null;
  keyStatus: KeyStatus | null;
  appInfo: AppInfo | null;
  approvals: ApprovalRequest[];
  toasts: Toast[];
  deviceCode: { code: string; url: string } | null;
  /** Latest result of a GitHub Releases update check (null = not checked yet). */
  update: UpdateInfo | null;
  showUpdate: boolean;
  /** Set to the new version on the first start after an update (shows "What's new"). */
  justUpdated: string | null;
  /** A version the user chose to skip: no announcement or sidebar badge for it. */
  skippedVersion: string | null;
  skipVersion: (v: string | null) => void;
  navigate: (r: Route) => void;
  toast: (tone: Toast['tone'], text: string) => void;
  dismissToast: (id: number) => void;
  loadSettings: () => Promise<void>;
  setSettings: (s: Settings) => void;
  setKeyStatus: (k: KeyStatus) => void;
  refreshApprovals: () => Promise<void>;
  setDeviceCode: (c: AppState['deviceCode']) => void;
  /** Simple mode hides the IDE; advanced shows files, terminal, git and every tool. */
  setUiMode: (mode: Settings['uiMode']) => Promise<void>;
  /** Saves a colour theme (applied immediately). */
  setTheme: (theme: Settings['theme']) => Promise<void>;
  checkUpdate: (manual?: boolean) => Promise<UpdateInfo | null>;
  setShowUpdate: (v: boolean) => void;
}

let toastId = 0;

export const useApp = create<AppState>((set, get) => ({
  route: { name: 'home' },
  settings: null,
  keyStatus: null,
  appInfo: null,
  approvals: [],
  toasts: [],
  deviceCode: null,
  update: null,
  showUpdate: false,
  justUpdated: null,
  skippedVersion: storage('genius.skipVersion'),
  skipVersion: (v) => {
    storage('genius.skipVersion', v ?? '');
    set({ skippedVersion: v, showUpdate: false });
  },
  navigate: (route) => set({ route }),
  toast: (tone, text) => {
    const id = ++toastId;
    set({ toasts: [...get().toasts.slice(-4), { id, tone, text }] });
    setTimeout(() => get().dismissToast(id), tone === 'bad' ? 9000 : 4500);
  },
  dismissToast: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
  loadSettings: async () => {
    try {
      const [settings, keyStatus, appInfo] = await Promise.all([api.getSettings(), api.keyStatus(), api.appInfo()]);
      set({ settings, keyStatus, appInfo });
      applyTheme(settings.theme);
      const last = storage('genius.lastVersion');
      storage('genius.lastVersion', appInfo.version);
      if (last && last !== appInfo.version) {
        set({ justUpdated: appInfo.version, showUpdate: true });
        get().toast('ok', `Genius was updated to ${appInfo.version}. Enjoy!`);
      }
      if (settings.autoCheckUpdates !== false) void get().checkUpdate(false);
    } catch (e) {
      get().toast('bad', `Could not load settings: ${errorMessage(e)}`);
    }
  },
  setSettings: (settings) => {
    set({ settings });
    applyTheme(settings.theme);
  },
  setKeyStatus: (keyStatus) => set({ keyStatus }),
  refreshApprovals: async () => {
    try {
      set({ approvals: await api.pendingApprovals() });
    } catch {
      /* approvals list is refreshed again on the next event */
    }
  },
  setDeviceCode: (deviceCode) => set({ deviceCode }),
  setUiMode: async (uiMode) => {
    const cur = get().settings;
    if (!cur || cur.uiMode === uiMode) return;
    set({ settings: { ...cur, uiMode } });
    try {
      set({ settings: await api.saveSettings({ ...cur, uiMode }) });
    } catch (e) {
      get().toast('bad', `Could not save the interface mode: ${errorMessage(e)}`);
    }
  },
  setTheme: async (theme) => {
    const cur = get().settings;
    if (!cur || cur.theme === theme) return;
    get().setSettings({ ...cur, theme });
    try {
      get().setSettings(await api.saveSettings({ ...cur, theme }));
    } catch (e) {
      get().toast('bad', `Could not save the theme: ${errorMessage(e)}`);
    }
  },
  checkUpdate: async (manual = false) => {
    try {
      const update = await api.checkForUpdate();
      set({ update });
      // Announce each new version once (the green arrow in the sidebar stays until installed).
      if (!manual && update.available && update.latestVersion && storage('genius.updateAnnounced') !== update.latestVersion && get().skippedVersion !== update.latestVersion) {
        storage('genius.updateAnnounced', update.latestVersion);
        set({ showUpdate: true });
      }
      if (manual && !update.available) get().toast('ok', update.detail ?? `You're on the latest version (${update.currentVersion}).`);
      return update;
    } catch (e) {
      if (manual) get().toast('bad', `Could not check for updates: ${errorMessage(e)}`);
      return null;
    }
  },
  setShowUpdate: (showUpdate) => set({ showUpdate }),
}));

export function applyTheme(theme: Settings['theme']) {
  const resolved = theme === 'system' ? (window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : theme;
  document.documentElement.dataset.theme = resolved;
}

/** Tiny try/catch wrapper around localStorage (unavailable in some contexts). */
function storage(key: string, value?: string): string | null {
  try {
    if (value !== undefined) localStorage.setItem(key, value);
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
