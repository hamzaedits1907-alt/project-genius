import { useEffect, useRef, useState } from 'react';
import { AlertTriangle, ChevronDown, ChevronRight, Crosshair, ExternalLink, Laptop, Lock, Monitor, Play, RotateCw, Smartphone, Tablet, Wand2, X } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { api, errorMessage } from '../../lib/api';
import { registerPreviewTarget } from '../../lib/inspection';
import { errorLocation, fixMessage, kindLabel, parseRuntimeReport, problemSummary } from '../../lib/runtime';
import type { PreviewRuntimeError } from '../../lib/types';
import { useApp } from '../../store/app';
import { askGenius, useWorkspace, type PickedElement } from '../../store/workspace';
import { Badge, Button, Callout, Empty, IconButton, Spinner } from '../../components/ui';

const DEVICES = { full: '100%', laptop: '1280px', tablet: '820px', phone: '390px' } as const;

/**
 * Shows the project's real dev server in an isolated iframe. The iframe origin
 * (http://127.0.0.1:<port>) differs from the Genius UI origin, and no Genius command is
 * granted to remote origins, so previewed code cannot reach credentials or IPC.
 */
export default function PreviewPanel({ onStart }: { onStart: () => void }) {
  const preview = useWorkspace((s) => s.preview);
  const runs = useWorkspace((s) => s.runs);
  const order = useWorkspace((s) => s.runOrder);
  const setPreview = useWorkspace((s) => s.setPreview);
  const set = useWorkspace((s) => s.set);
  const [device, setDevice] = useState<keyof typeof DEVICES>('full');
  const picking = useWorkspace((s) => s.picking);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const deviceRef = useRef<HTMLDivElement>(null);
  const devRun = order.map((id) => runs[id]).find((r) => r?.info.op === 'dev');
  const origin = preview.url ? new URL(preview.url).origin : null;

  const pending = useRef<PreviewRuntimeError[]>([]);
  const flushTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Messages from the preview are accepted only from the preview's own origin and only
  // as data (an element description the user can see before anything is sent; runtime
  // errors that are validated, then forwarded to the backend in batches).
  useEffect(() => {
    const flush = () => {
      flushTimer.current = null;
      const batch = pending.current;
      pending.current = [];
      const pid = useWorkspace.getState().projectId;
      if (!pid || !batch.length) return;
      api
        .reportPreviewErrors(pid, batch)
        .then((list) => useWorkspace.getState().projectId === pid && set({ previewErrors: list }))
        .catch(() => undefined);
    };
    const onMsg = (e: MessageEvent) => {
      if (!origin || e.origin !== origin || e.source !== frameRef.current?.contentWindow) return;
      const data = e.data as { type?: string; element?: PickedElement };
      if (data?.type === 'genius:picked' && data.element && typeof data.element.tag === 'string') {
        set({ picked: data.element, picking: false, rightTab: 'assistant' });
      } else if (data?.type === 'genius:pick-cancelled') {
        set({ picking: false });
      } else if (data?.type === 'genius:runtime') {
        pending.current.push(...parseRuntimeReport(data));
        if (!flushTimer.current) flushTimer.current = setTimeout(flush, 300);
      } else if (data?.type === 'genius:runtime-healthy') {
        // Reloaded / hot-updated without errors and the page shows content: problems are gone.
        pending.current = [];
        const pid = useWorkspace.getState().projectId;
        if (pid && useWorkspace.getState().previewErrors.length) {
          set({ previewErrors: [] });
          api.clearPreviewErrors(pid).catch(() => undefined);
        }
      }
    };
    window.addEventListener('message', onMsg);
    return () => {
      window.removeEventListener('message', onMsg);
      if (flushTimer.current) clearTimeout(flushTimer.current);
      flushTimer.current = null;
    };
  }, [origin, set]);

  // Genius can look at the running app (visual verification, `genius://inspect-request`).
  const ready = preview.status === 'ready';
  useEffect(() => {
    return registerPreviewTarget(() => {
      const frame = frameRef.current;
      const container = deviceRef.current;
      if (!ready || !origin || !frame || !container) return null;
      return { frame, container, origin, projectId: useWorkspace.getState().projectId };
    });
  }, [ready, origin]);

  const togglePick = () => {
    const win = frameRef.current?.contentWindow;
    if (!win || !origin) return;
    win.postMessage({ type: 'genius:pick', on: !picking }, origin);
    set({ picking: !picking });
  };

  if (preview.status === 'stopped' || (preview.status === 'failed' && !preview.url)) {
    return (
      <div className="col" style={{ height: '100%' }}>
        {preview.status === 'failed' && (
          <div style={{ padding: 10 }}>
            <Callout tone="bad" title="The preview could not start">
              {preview.error}
            </Callout>
            {devRun && (
              <Button size="sm" style={{ marginTop: 8 }} onClick={() => set({ bottomTab: 'terminal', selectedRun: devRun.info.id })}>
                Show dev server output
              </Button>
            )}
          </div>
        )}
        <Empty icon={<Monitor size={20} />} title="Preview is not running" action={<Button variant="ok" icon={<Play size={14} />} onClick={onStart}>Run the app</Button>}>
          Starts the project's real development server (npm run dev) on 127.0.0.1 and shows the running application here.
        </Empty>
      </div>
    );
  }

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }}>
      <div className="preview-bar">
        <IconButton label="Reload preview" onClick={() => setPreview({ nonce: preview.nonce + 1 })} disabled={preview.status !== 'ready'}>
          <RotateCw size={14} />
        </IconButton>
        <div className="preview-url" title="Isolated local preview origin">
          <Lock size={11} />
          <span className="mono ellipsis">{preview.url ?? 'starting…'}</span>
          {preview.status === 'starting' && <Spinner size={10} />}
        </div>
        <IconButton label={picking ? 'Cancel element selection (Esc)' : 'Select an element to change'} active={picking} disabled={preview.status !== 'ready'} onClick={togglePick}>
          <Crosshair size={14} />
        </IconButton>
        <IconButton label="Full width" active={device === 'full'} onClick={() => setDevice('full')}>
          <Monitor size={14} />
        </IconButton>
        <IconButton label="Laptop width" active={device === 'laptop'} onClick={() => setDevice('laptop')}>
          <Laptop size={14} />
        </IconButton>
        <IconButton label="Tablet width" active={device === 'tablet'} onClick={() => setDevice('tablet')}>
          <Tablet size={14} />
        </IconButton>
        <IconButton label="Phone width" active={device === 'phone'} onClick={() => setDevice('phone')}>
          <Smartphone size={14} />
        </IconButton>
        <IconButton label="Open in browser" disabled={!preview.url} onClick={() => preview.url && void openUrl(preview.url)}>
          <ExternalLink size={14} />
        </IconButton>
      </div>
      {preview.status === 'ready' && <ProblemBar />}
      <div style={{ flex: 1, minHeight: 0, background: 'var(--bg-0)', overflow: 'auto' }}>
        {preview.status === 'starting' || !preview.url ? (
          <Empty icon={<Spinner size={18} />} title="Starting the development server…">
            Waiting for the server to accept connections. Output is in the Terminal tab.
          </Empty>
        ) : (
          <div ref={deviceRef} className="device-frame" style={{ width: DEVICES[device], maxWidth: '100%' }}>
            <iframe
              ref={frameRef}
              key={`${preview.url}#${preview.nonce}`}
              className="preview-frame"
              title="Application preview"
              src={preview.url}
              sandbox="allow-scripts allow-same-origin allow-forms allow-modals allow-popups"
              referrerPolicy="no-referrer"
            />
          </div>
        )}
      </div>
      <div className="faint tiny" style={{ padding: '4px 10px', borderTop: '1px solid var(--border)' }}>
        {picking ? 'Click an element in the app to ask Genius to change it. Esc cancels.' : 'Vite reloads the page automatically after files change. Compile errors appear inside the preview and in the Terminal.'}
      </div>
    </div>
  );
}

/** Friendly summary of runtime problems in the running app, with a one-click fix. */
export function ProblemBar() {
  const project = useWorkspace((s) => s.project);
  const errors = useWorkspace((s) => s.previewErrors);
  const busy = useWorkspace((s) => s.tasks.some((t) => t.kind !== 'verify'));
  const set = useWorkspace((s) => s.set);
  const toast = useApp((s) => s.toast);
  const [open, setOpen] = useState(false);
  const [sending, setSending] = useState(false);
  const sum = problemSummary(errors);
  if (!project || !sum) return null;

  const fix = async () => {
    setSending(true);
    try {
      await askGenius(project.id, fixMessage(errors));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setSending(false);
    }
  };
  const dismiss = () => {
    set({ previewErrors: [] });
    api.clearPreviewErrors(project.id).catch(() => undefined);
  };

  return (
    <div className={`problem-bar ${sum.severity === 'error' ? 'soft' : ''}`} role="alert">
      <div className="problem-row">
        <AlertTriangle size={15} className="problem-icon" />
        <div className="grow problem-text" title={sum.detail}>
          <strong>{sum.title}</strong>
          {sum.detail && <span className="mono">: {sum.detail}</span>}
          {sum.location && <span className="faint"> — {sum.location}</span>}
        </div>
        {sum.count > 1 && <Badge tone="bad">×{sum.count}</Badge>}
        <Button size="sm" variant="ghost" icon={open ? <ChevronDown size={13} /> : <ChevronRight size={13} />} aria-expanded={open} onClick={() => setOpen(!open)}>
          Details
        </Button>
        <Button size="sm" variant="primary" icon={<Wand2 size={13} />} loading={sending} disabled={busy} title={busy ? 'Genius is working on something else' : 'Ask Genius to find the cause and fix it'} onClick={() => void fix()}>
          Fix it
        </Button>
        <IconButton label="Dismiss" onClick={dismiss}>
          <X size={13} />
        </IconButton>
      </div>
      {open && (
        <ul className="problem-details">
          {errors.map((e, i) => (
            <li key={`${e.kind}:${e.message}:${e.source ?? ''}:${e.line ?? ''}:${i}`}>
              <div className="small">
                <strong>{kindLabel(e)}</strong>
                {e.count > 1 && <span className="faint"> ×{e.count}</span>}
                {errorLocation(e) && <span className="faint"> · {errorLocation(e)}</span>}
                <span className="faint"> · page {e.page}</span>
              </div>
              <div className="mono small problem-msg">{e.message}</div>
              {e.stack && <pre className="block problem-stack">{e.stack}</pre>}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
