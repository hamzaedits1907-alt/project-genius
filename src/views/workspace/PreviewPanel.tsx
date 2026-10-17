import { useEffect, useRef, useState } from 'react';
import { AlertTriangle, Check, ChevronDown, ChevronRight, Crosshair, ExternalLink, Laptop, Lock, Monitor, Play, RefreshCcw, RotateCw, Smartphone, Square, Tablet, Wand2 } from 'lucide-react';
import { useWorkspace, type PickedElement } from '../../store/workspace';
import { canShowFrame, PREVIEW_STEPS, previewBusy } from '../../lib/logic';
import type { PreviewProblem } from '../../lib/types';
import { Button, Callout, Empty, IconButton, Spinner } from '../../components/ui';
import { usePreviewActions } from './usePreviewActions';

const DEVICES = { full: '100%', laptop: '1280px', tablet: '820px', phone: '390px' } as const;

const STAGE_TITLE: Record<string, string> = {
  config: 'Your project needs a fix before it can start',
  install: 'The dependencies could not be installed',
  start: 'Your website could not start',
  compile: 'Your website has an error',
  identity: 'The preview could not be verified',
  runtime: 'Your page reported an error',
};

/**
 * Shows the project's real dev server in an isolated iframe. The iframe origin
 * (http://127.0.0.1:<port>) differs from the Genius UI origin, and no Genius command is
 * granted to remote origins, so previewed code cannot reach credentials or IPC.
 * Everything shown here comes from the backend's verified preview state.
 */
export default function PreviewPanel() {
  const preview = useWorkspace((s) => s.preview);
  const runtimeError = useWorkspace((s) => s.runtimeError);
  const runs = useWorkspace((s) => s.runs);
  const set = useWorkspace((s) => s.set);
  const picking = useWorkspace((s) => s.picking);
  const [device, setDevice] = useState<keyof typeof DEVICES>('full');
  const [showAnyway, setShowAnyway] = useState(false);
  const frameRef = useRef<HTMLIFrameElement>(null);
  const actions = usePreviewActions();
  const origin = preview.url ? new URL(preview.url).origin : null;
  const busy = previewBusy(preview.status);
  const frame = canShowFrame(preview);
  const compileError = preview.status === 'failed' && preview.problem?.stage === 'compile';

  // Messages from the preview are accepted only from the preview's own origin and only
  // as data (an element description, or an error message, shown to the user).
  useEffect(() => {
    const onMsg = (e: MessageEvent) => {
      if (!origin || e.origin !== origin || e.source !== frameRef.current?.contentWindow) return;
      const data = e.data as { type?: string; element?: PickedElement; message?: unknown; stack?: unknown };
      if (data?.type === 'genius:picked' && data.element && typeof data.element.tag === 'string') {
        set({ picked: data.element, picking: false, rightTab: 'assistant' });
      } else if (data?.type === 'genius:pick-cancelled') {
        set({ picking: false });
      } else if (data?.type === 'genius:runtime-error' && typeof data.message === 'string') {
        set({ runtimeError: { message: data.message.slice(0, 2000), stack: typeof data.stack === 'string' ? data.stack.slice(0, 4000) : '', runId: useWorkspace.getState().preview.runId } });
      } else if (data?.type === 'genius:loaded') {
        set({ runtimeError: null });
      }
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, [origin, set]);

  useEffect(() => {
    setShowAnyway(false);
  }, [preview.runId, preview.status]);

  const togglePick = () => {
    const win = frameRef.current?.contentWindow;
    if (!win || !origin) return;
    win.postMessage({ type: 'genius:pick', on: !picking }, origin);
    set({ picking: !picking });
  };
  const running = !!preview.runId && (preview.status === 'ready' || preview.status === 'failed' || busy);
  const devRun = preview.runId ? runs[preview.runId] : undefined;
  const problemRun = preview.problem?.runId ? runs[preview.problem.runId] : undefined;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%' }} aria-label="Preview">
      <div className="preview-bar">
        {running || busy ? (
          <IconButton label="Stop the preview" onClick={() => void actions.stop()} disabled={actions.busy === 'stop' || preview.status === 'stopping'}>
            <Square size={13} />
          </IconButton>
        ) : (
          <IconButton label="Start the preview" onClick={() => void actions.start()} disabled={!!actions.busy}>
            <Play size={14} />
          </IconButton>
        )}
        <IconButton label={frame ? 'Refresh the page' : 'Refresh is available while the site is shown'} onClick={actions.refresh} disabled={!frame}>
          <RotateCw size={14} />
        </IconButton>
        <IconButton label={busy ? 'Restart is available once the server has started' : 'Restart the development server'} onClick={() => void actions.restart()} disabled={busy || !!actions.busy}>
          <RefreshCcw size={14} />
        </IconButton>
        <div className="preview-url" title="Isolated local preview address">
          <Lock size={11} />
          <span className="mono ellipsis">{preview.url ?? (busy ? 'starting…' : 'not running')}</span>
          {busy && <Spinner size={10} />}
        </div>
        <IconButton label={picking ? 'Cancel element selection (Esc)' : 'Select an element to change'} active={picking} disabled={preview.status !== 'ready' || !preview.inspector} onClick={togglePick}>
          <Crosshair size={14} />
        </IconButton>
        {(['full', 'laptop', 'tablet', 'phone'] as const).map((d) => (
          <IconButton key={d} label={`${d === 'full' ? 'Full' : d[0].toUpperCase() + d.slice(1)} width`} active={device === d} onClick={() => setDevice(d)}>
            {d === 'full' ? <Monitor size={14} /> : d === 'laptop' ? <Laptop size={14} /> : d === 'tablet' ? <Tablet size={14} /> : <Smartphone size={14} />}
          </IconButton>
        ))}
        <IconButton label={preview.url ? 'Open in your browser' : 'Open in browser is available while the server runs'} disabled={!preview.url || !running} onClick={actions.openExternal}>
          <ExternalLink size={14} />
        </IconButton>
      </div>

      {runtimeError && runtimeError.runId === preview.runId && frame && (
        <div style={{ padding: '8px 10px 0' }}>
          <Callout tone="warn" title="Your page reported an error while running">
            <div className="mono small ellipsis" title={runtimeError.message}>
              {runtimeError.message}
            </div>
            <div className="row" style={{ marginTop: 6 }}>
              <Button size="sm" variant="primary" icon={<Wand2 size={13} />} loading={actions.busy === 'fix'} onClick={() => void actions.autoFix()}>
                Fix automatically
              </Button>
              <Button size="sm" variant="ghost" onClick={() => set({ runtimeError: null })}>
                Dismiss
              </Button>
            </div>
          </Callout>
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, background: 'var(--bg-0)', overflow: 'auto', position: 'relative' }}>
        {busy ? (
          <Progress message={preview.message} status={preview.status} lines={devRun?.lines.slice(-14).map((l) => l.line) ?? []} />
        ) : preview.status === 'idle' ? (
          <Empty icon={<Monitor size={20} />} title="See your website here" action={<Button variant="ok" icon={<Play size={14} />} loading={actions.busy === 'start'} onClick={() => void actions.start()}>Preview</Button>}>
            Genius checks the project, installs what it needs, starts it on your computer and shows the running site in this panel. The first start can take a minute.
          </Empty>
        ) : frame && (!compileError || showAnyway) ? (
          <div className="device-frame" style={{ width: DEVICES[device], maxWidth: '100%' }}>
            <iframe
              ref={frameRef}
              key={`${preview.runId}#${preview.nonce}`}
              className="preview-frame"
              title="Application preview"
              src={preview.url!}
              sandbox="allow-scripts allow-same-origin allow-forms allow-modals allow-popups"
              referrerPolicy="no-referrer"
            />
          </div>
        ) : preview.problem ? (
          <ProblemPanel problem={preview.problem} message={preview.message} output={problemRun?.lines.slice(-60).map((l) => l.line) ?? []} canShowAnyway={compileError} onShowAnyway={() => setShowAnyway(true)} onTerminal={preview.problem.runId ? () => set({ bottomTab: 'terminal', selectedRun: preview.problem!.runId, layout: 'advanced' }) : undefined} />
        ) : (
          <Empty icon={<AlertTriangle size={20} />} title="The preview stopped" action={<Button variant="ok" icon={<Play size={14} />} onClick={() => void actions.start()}>Start again</Button>}>
            {preview.message}
          </Empty>
        )}
      </div>
      <div className="faint tiny" style={{ padding: '4px 10px', borderTop: '1px solid var(--border)' }} aria-live="polite">
        {picking ? 'Click an element in the app to ask Genius to change it. Esc cancels.' : preview.message || 'The page reloads automatically after files change.'}
      </div>
    </div>
  );
}

function Progress({ message, status, lines }: { message: string; status: string; lines: string[] }) {
  const [open, setOpen] = useState(false);
  const idx = PREVIEW_STEPS.findIndex((s) => s.status === status);
  return (
    <div className="empty" role="status" aria-label="Preview is starting">
      <Spinner size={18} />
      <h3>{message || 'Starting…'}</h3>
      {status !== 'stopping' && (
        <ol className="pv-steps">
          {PREVIEW_STEPS.map((s, i) => (
            <li key={s.status} className={i < idx ? 'done' : i === idx ? 'now' : ''}>
              {i < idx ? <Check size={12} /> : i === idx ? <Spinner size={10} /> : <span className="pv-dot" />} {s.label}
            </li>
          ))}
        </ol>
      )}
      {lines.length > 0 && (
        <>
          <button className="btn ghost sm" onClick={() => setOpen(!open)}>
            {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Technical details
          </button>
          {open && <pre className="stream" style={{ textAlign: 'left', maxWidth: '100%', width: 560 }}>{lines.join('\n')}</pre>}
        </>
      )}
    </div>
  );
}

function ProblemPanel({ problem, message, output, canShowAnyway, onShowAnyway, onTerminal }: { problem: PreviewProblem; message: string; output: string[]; canShowAnyway: boolean; onShowAnyway: () => void; onTerminal?: () => void }) {
  const [open, setOpen] = useState(false);
  const actions = usePreviewActions();
  const tasks = useWorkspace((s) => s.tasks);
  const fixing = tasks.find((t) => t.kind === 'repair');
  const blocking = tasks.find((t) => t.kind !== 'verify' && t.kind !== 'repair');
  return (
    <div className="pv-problem" role="alert">
      <div className="pv-problem-card">
        <h3>
          <AlertTriangle size={16} color="var(--bad)" /> {STAGE_TITLE[problem.stage] ?? message}
        </h3>
        <p>{problem.summary}</p>
        {!problem.fixable && <p className="small faint">Changing the code will not solve this one; the message above says what is needed. Then press “Try again”.</p>}
        <div className="row" style={{ flexWrap: 'wrap' }}>
          {problem.fixable && (
            <Button
              variant="primary"
              icon={<Wand2 size={14} />}
              loading={actions.busy === 'fix' || !!fixing}
              disabled={!!blocking}
              title={blocking ? `Wait for “${blocking.title}” to finish` : 'Genius reads the real error, makes the smallest fix, and checks it again (up to 3 attempts)'}
              onClick={() => void actions.autoFix()}
            >
              {fixing ? 'Fixing…' : 'Fix automatically'}
            </Button>
          )}
          <Button icon={<Play size={14} />} onClick={() => void actions.restart()} loading={actions.busy === 'restart'}>
            Try again
          </Button>
          <Button variant="ghost" onClick={() => setOpen(!open)} aria-expanded={open}>
            {open ? <ChevronDown size={13} /> : <ChevronRight size={13} />} View details
          </Button>
          {canShowAnyway && (
            <Button variant="ghost" onClick={onShowAnyway}>
              Show the page anyway
            </Button>
          )}
        </div>
        {open && (
          <div className="col" style={{ gap: 6 }}>
            {problem.file && <div className="small">File: <span className="mono">{problem.file}</span></div>}
            <pre className="stream">{problem.details || output.join('\n') || 'No further output was recorded.'}</pre>
            {onTerminal && (
              <Button size="sm" variant="ghost" onClick={onTerminal}>
                Open the full output in the terminal
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
