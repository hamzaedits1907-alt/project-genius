import { useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { api, errorMessage } from '../../lib/api';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';

/** Preview and repair actions shared by the toolbar, the preview panel and the header. */
export function usePreviewActions() {
  const projectId = useWorkspace((s) => s.projectId);
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState<string | null>(null);

  const run = async (what: string, fn: () => Promise<unknown>) => {
    if (!projectId) return;
    setBusy(what);
    try {
      await fn();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const ws = () => useWorkspace.getState();

  return {
    busy,
    start: () =>
      run('start', async () => {
        ws().set({ rightTab: 'preview' });
        ws().onPreviewState(await api.startPreview(projectId!));
      }),
    restart: () =>
      run('restart', async () => {
        ws().set({ runtimeError: null });
        ws().onPreviewState(await api.restartPreview(projectId!));
      }),
    stop: () => run('stop', () => api.stopPreview(projectId!)),
    refresh: () => {
      ws().set({ runtimeError: null });
      ws().setPreview({ nonce: ws().preview.nonce + 1 });
    },
    openExternal: () => {
      const url = ws().preview.url;
      if (url) void openUrl(url).catch((e) => toast('bad', errorMessage(e)));
    },
    autoFix: () =>
      run('fix', async () => {
        const rt = ws().runtimeError;
        await api.startAutoFix(projectId!, rt ? `${rt.message}\n${rt.stack}`.trim() : null);
        ws().set({ rightTab: 'assistant', runtimeError: null });
        toast('info', 'Genius is checking and repairing the project. Progress is shown in the conversation.');
      }),
  };
}
