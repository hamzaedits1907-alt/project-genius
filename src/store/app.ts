import { create } from 'zustand';
import { api, errorMessage } from '../lib/api';
import type { ApprovalRequest, AppInfo, KeyStatus, Settings } from '../lib/types';

export type Route =
  | { name: 'home' }
  | { name: 'create' }
  | { name: 'settings'; tab?: SettingsTab }
  | { name: 'project'; id: string };

export type SettingsTab = 'ai' | 'github' | 'approvals' | 'workspace' | 'appearance';

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
  navigate: (r: Route) => void;
  toast: (tone: Toast['tone'], text: string) => void;
  dismissToast: (id: number) => void;
  loadSettings: () => Promise<void>;
  setSettings: (s: Settings) => void;
  setKeyStatus: (k: KeyStatus) => void;
  refreshApprovals: () => Promise<void>;
  setDeviceCode: (c: AppState['deviceCode']) => void;
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
}));

export function applyTheme(theme: Settings['theme']) {
  const resolved = theme === 'system' ? (window.matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark') : theme;
  document.documentElement.dataset.theme = resolved;
}
