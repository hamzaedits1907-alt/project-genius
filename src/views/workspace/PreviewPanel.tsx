import { useEffect, useRef, useState } from 'react';
import { Crosshair, ExternalLink, Laptop, Lock, Monitor, Play, RotateCw, Smartphone, Tablet } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useWorkspace, type PickedElement } from '../../store/workspace';
import { Button, Callout, Empty, IconButton, Spinner } from '../../components/ui';

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
  const devRun = order.map((id) => runs[id]).find((r) => r?.info.op === 'dev');
  const origin = preview.url ? new URL(preview.url).origin : null;

  // Messages from the preview are accepted only from the preview's own origin and only
  // as data (an element description the user can see before anything is sent).
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      if (!origin || e.origin !== origin || e.source !== frameRef.current?.contentWindow) return;
      const data = e.data as { type?: string; element?: PickedElement };
      if (data?.type === 'genius:picked' && data.element && typeof data.element.tag === 'string') {
        set({ picked: data.element, picking: false, rightTab: 'assistant' });
      } else if (data?.type === 'genius:pick-cancelled') {
        set({ picking: false });
      }
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, [origin, set]);

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
      <div style={{ flex: 1, minHeight: 0, background: 'var(--bg-0)', overflow: 'auto' }}>
        {preview.status === 'starting' || !preview.url ? (
          <Empty icon={<Spinner size={18} />} title="Starting the development server…">
            Waiting for the server to accept connections. Output is in the Terminal tab.
          </Empty>
        ) : (
          <div className="device-frame" style={{ width: DEVICES[device], maxWidth: '100%' }}>
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
