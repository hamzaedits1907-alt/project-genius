// Subscribes once to every backend channel and routes payloads into the stores.
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { ActivityEvent, AgentStep, AiDelta, AuthStatus, FileStreamEvent, PreviewEvent, ProcessOutput, ProcessState } from '../lib/types';
import { api } from '../lib/api';
import { noticeFor, notificationsEnabled, notifyIfAway, windowFocused } from '../lib/notify';
import { useApp } from './app';
import { useWorkspace } from './workspace';

/** Long AI work finished or needs the user while Genius is in the background: tell them. */
async function announce(ev: ActivityEvent) {
  if (!notificationsEnabled() || windowFocused() || !noticeFor(ev, '')) return;
  const open = useWorkspace.getState().project;
  const pid = ev.projectId;
  const name = !pid ? '' : open?.id === pid ? open.name : await api.getProject(pid).then((p) => p.name).catch(() => '');
  await notifyIfAway(ev, name, pid ? () => useApp.getState().navigate({ name: 'project', id: pid }) : undefined);
}

export async function subscribeAll(): Promise<UnlistenFn> {
  const ws = () => useWorkspace.getState();
  const app = () => useApp.getState();
  let pendingLines: ProcessOutput[] = [];
  let flushScheduled = false;
  const flush = () => {
    flushScheduled = false;
    const batch = pendingLines;
    pendingLines = [];
    for (const p of batch) ws().onProcessOutput(p.runId, p.stream, p.line);
  };

  const unlisteners = await Promise.all([
    listen<ActivityEvent>('genius://activity', ({ payload }) => {
      ws().onActivity(payload);
      if (payload.kind === 'approval') void app().refreshApprovals();
      void announce(payload);
    }),
    listen<AiDelta>('genius://ai-delta', ({ payload }) => {
      if (payload.projectId === ws().projectId) ws().onAiDelta(payload.taskId, payload.text);
    }),
    listen<FileStreamEvent>('genius://file-stream', ({ payload }) => ws().onFileStream(payload)),
    listen<ProcessOutput>('genius://process-output', ({ payload }) => {
      // batch output lines into animation frames so long installs keep the UI responsive
      pendingLines.push(payload);
      if (!flushScheduled) {
        flushScheduled = true;
        requestAnimationFrame(flush);
      }
    }),
    listen<ProcessState>('genius://process-state', ({ payload }) => {
      flush();
      ws().onProcessState(payload.run, payload.state, payload.outcome);
    }),
    listen<{ projectId: string; paths: string[] }>('genius://fs-changed', ({ payload }) => {
      if (payload.projectId !== ws().projectId) return;
      void ws().refreshFiles();
      const open = Object.keys(ws().buffers);
      const targets = payload.paths.length ? payload.paths.filter((p) => open.includes(p)) : open;
      for (const p of targets) void ws().reloadFile(p);
      // Applied changes supersede drafts: drop them and turn open draft tabs into real file tabs.
      const drafts = { ...ws().drafts };
      let tabs = ws().tabs;
      let active = ws().active;
      for (const p of payload.paths) {
        delete drafts[p];
        const draftTab = `draft:${p}`;
        if (tabs.includes(draftTab)) {
          const fileTab = `file:${p}`;
          tabs = tabs.includes(fileTab) ? tabs.filter((t) => t !== draftTab) : tabs.map((t) => (t === draftTab ? fileTab : t));
          if (active === draftTab) active = fileTab;
          void ws().reloadFile(p, true);
        }
      }
      ws().set({ drafts, tabs, active, gitVersion: ws().gitVersion + 1 });
    }),
    listen<PreviewEvent>('genius://preview', ({ payload }) => {
      if (payload.projectId !== ws().projectId) return;
      const nonce = ws().preview.nonce + 1;
      ws().setPreview({ status: payload.status, url: payload.url ?? (payload.status === 'ready' ? ws().preview.url : undefined), error: payload.error, nonce });
      if (payload.status === 'ready') ws().set({ rightTab: 'preview' });
    }),
    listen<{ code: string; url: string }>('genius://github-device-code', ({ payload }) => app().setDeviceCode(payload)),
    listen<AuthStatus>('genius://github-status', () => app().setDeviceCode(null)),
    listen<AgentStep>('genius://agent-step', ({ payload }) => ws().onAgentStep(payload)),
  ]);
  return () => unlisteners.forEach((u) => u());
}
