import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Bot, CheckCircle2, CircleDashed, FileDiff, FileInput, FilePen, GitBranch, Hammer, HelpCircle, ListChecks, Package, Play, RefreshCw,
  ShieldQuestion, Square, Stethoscope, TerminalSquare, TestTube2, XCircle, AlertTriangle, Download, ClipboardList, Activity as ActivityIcon,
} from 'lucide-react';
import { Github } from '../../components/icons';
import { api, errorMessage } from '../../lib/api';
import { clockTime, duration, matchesFilter, STATUS_LABEL, type ActivityFilter } from '../../lib/logic';
import type { ActivityEvent, EventKind, ReadinessCheck } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Dot, Empty, Spinner, StatusBadge } from '../../components/ui';

export const KIND_ICON: Record<EventKind, React.ReactNode> = {
  task: <ListChecks size={14} />,
  ai: <Bot size={14} />,
  plan: <ClipboardList size={14} />,
  file_read: <FileInput size={14} />,
  file_write: <FilePen size={14} />,
  changeset: <FileDiff size={14} />,
  command: <TerminalSquare size={14} />,
  install: <Package size={14} />,
  build: <Hammer size={14} />,
  test: <TestTube2 size={14} />,
  preview: <Play size={14} />,
  git: <GitBranch size={14} />,
  github: <Github size={14} />,
  approval: <ShieldQuestion size={14} />,
  export: <Download size={14} />,
  diagnosis: <Stethoscope size={14} />,
};

export default function BottomPanel() {
  const tab = useWorkspace((s) => s.bottomTab);
  const set = useWorkspace((s) => s.set);
  const runs = useWorkspace((s) => s.runs);
  const failures = useWorkspace((s) => s.activity.filter((e) => e.status === 'failed').length);
  const runningCount = Object.values(runs).filter((r) => r.state === 'running').length;
  return (
    <div className="panel">
      <div className="panel-head">
        <div className="subtabs" role="tablist">
          <button role="tab" aria-selected={tab === 'activity'} className={tab === 'activity' ? 'on' : ''} onClick={() => set({ bottomTab: 'activity' })}>
            <ActivityIcon size={13} /> Activity {failures > 0 && <Badge tone="bad">{failures}</Badge>}
          </button>
          <button role="tab" aria-selected={tab === 'terminal'} className={tab === 'terminal' ? 'on' : ''} onClick={() => set({ bottomTab: 'terminal' })}>
            <TerminalSquare size={13} /> Terminal {runningCount > 0 && <span className="dot run" />}
          </button>
          <button role="tab" aria-selected={tab === 'readiness'} className={tab === 'readiness' ? 'on' : ''} onClick={() => set({ bottomTab: 'readiness' })}>
            <CheckCircle2 size={13} /> Readiness
          </button>
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        {tab === 'activity' && <ActivityFeed />}
        {tab === 'terminal' && <Terminal />}
        {tab === 'readiness' && <Readiness />}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------- activity

const FILTERS: { id: ActivityFilter; label: string }[] = [
  { id: 'all', label: 'All' },
  { id: 'ai', label: 'AI' },
  { id: 'files', label: 'Files' },
  { id: 'commands', label: 'Commands' },
  { id: 'builds', label: 'Builds & tests' },
  { id: 'git', label: 'Git' },
  { id: 'errors', label: 'Errors' },
  { id: 'approvals', label: 'Approvals' },
];

export function ActivityFeed() {
  const activity = useWorkspace((s) => s.activity);
  const [filter, setFilter] = useState<ActivityFilter>('all');
  const [selected, setSelected] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const listRef = useRef<HTMLDivElement>(null);
  const shown = useMemo(() => activity.filter((e) => matchesFilter(e, filter)), [activity, filter]);
  const sel = shown.find((e) => e.id === selected) ?? activity.find((e) => e.id === selected);

  useEffect(() => {
    if (follow && listRef.current) listRef.current.scrollTop = listRef.current.scrollHeight;
  }, [shown.length, follow]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div className="row" style={{ padding: '6px 10px', borderBottom: '1px solid var(--border)', flexWrap: 'wrap', gap: 4 }}>
        <div className="subtabs" role="radiogroup" aria-label="Filter activity">
          {FILTERS.map((f) => (
            <button key={f.id} role="radio" aria-checked={filter === f.id} className={filter === f.id ? 'on' : ''} onClick={() => setFilter(f.id)}>
              {f.label}
            </button>
          ))}
        </div>
        <span className="spacer" />
        <label className="checkbox tiny muted">
          <input type="checkbox" checked={follow} onChange={(e) => setFollow(e.target.checked)} /> Follow
        </label>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'grid', gridTemplateColumns: sel ? 'minmax(0, 1fr) minmax(0, 1fr)' : '1fr' }}>
        <div className="scroll feed" ref={listRef} onScroll={(e) => {
          const el = e.currentTarget;
          const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
          if (atBottom !== follow) setFollow(atBottom);
        }}>
          {shown.length === 0 && <Empty icon={<CircleDashed size={18} />} title="No activity yet">Everything Genius does — AI requests, file writes, commands, builds and Git operations — appears here as it happens.</Empty>}
          {shown.map((e) => (
            <div key={e.id} className={`ev ${selected === e.id ? 'on' : ''}`} onClick={() => setSelected(selected === e.id ? null : e.id)} role="button" tabIndex={0} onKeyDown={(k) => k.key === 'Enter' && setSelected(e.id)}>
              <span className="ic" style={{ color: e.status === 'failed' ? 'var(--bad)' : e.status === 'awaiting_approval' ? 'var(--warn)' : undefined }}>
                {KIND_ICON[e.kind] ?? <HelpCircle size={14} />}
              </span>
              <div style={{ minWidth: 0 }}>
                <div className="t">{e.title}</div>
                <div className="m row" style={{ gap: 6 }}>
                  <Dot status={e.status} />
                  {STATUS_LABEL[e.status] ?? e.status}
                  {typeof e.detail.durationMs === 'number' && <span>· {duration(e.detail.durationMs)}</span>}
                  {typeof e.detail.exitCode === 'number' && <span>· exit {e.detail.exitCode}</span>}
                </div>
              </div>
              <span className="time">{clockTime(e.ts)}</span>
            </div>
          ))}
        </div>
        {sel && <EventDetail ev={sel} />}
      </div>
    </div>
  );
}

function EventDetail({ ev }: { ev: ActivityEvent }) {
  const openTab = useWorkspace((s) => s.openTab);
  const openFile = useWorkspace((s) => s.openFile);
  const set = useWorkspace((s) => s.set);
  const files = useWorkspace((s) => s.files);
  const d = ev.detail as Record<string, unknown>;
  const err = d.error as { kind?: string; message?: string } | undefined;
  const changesetId = (typeof d.changesetId === 'string' && d.changesetId) || (typeof (d.result as Record<string, unknown>)?.changesetId === 'string' && ((d.result as Record<string, unknown>).changesetId as string)) || null;
  const runId = typeof d.runId === 'string' ? d.runId : null;
  const path = typeof d.path === 'string' ? d.path : null;
  const known = new Set(['error', 'changesetId', 'runId', 'path', 'command', 'cwd', 'reason', 'model', 'rootCause']);
  const rest = Object.fromEntries(Object.entries(d).filter(([k]) => !known.has(k)));
  return (
    <div className="detail" style={{ borderTop: 0, borderLeft: '1px solid var(--border)' }}>
      <div className="row" style={{ marginBottom: 8 }}>
        <strong className="grow" style={{ overflowWrap: 'anywhere' }}>{ev.title}</strong>
        <StatusBadge status={ev.status} label={STATUS_LABEL[ev.status]} />
      </div>
      <dl className="kv">
        <dt>Type</dt>
        <dd>{ev.kind.replace('_', ' ')}</dd>
        <dt>Started</dt>
        <dd>{new Date(ev.ts).toLocaleString()}</dd>
        {ev.updatedTs !== ev.ts && (
          <>
            <dt>Updated</dt>
            <dd>{new Date(ev.updatedTs).toLocaleString()}</dd>
          </>
        )}
        {typeof d.command === 'string' && (
          <>
            <dt>Command</dt>
            <dd className="mono">{d.command}</dd>
          </>
        )}
        {typeof d.cwd === 'string' && (
          <>
            <dt>Directory</dt>
            <dd className="mono">{d.cwd}</dd>
          </>
        )}
        {typeof d.reason === 'string' && (
          <>
            <dt>Why</dt>
            <dd>{d.reason}</dd>
          </>
        )}
        {typeof d.model === 'string' && (
          <>
            <dt>Model</dt>
            <dd className="mono">{d.model}</dd>
          </>
        )}
        {typeof d.rootCause === 'string' && (
          <>
            <dt>Root cause</dt>
            <dd>{d.rootCause}</dd>
          </>
        )}
      </dl>
      {err && (
        <div className="callout bad" style={{ marginTop: 10 }}>
          <XCircle size={16} color="var(--bad)" />
          <div style={{ whiteSpace: 'pre-wrap' }}>{err.message}</div>
        </div>
      )}
      <div className="row wrap" style={{ marginTop: 10 }}>
        {changesetId && (
          <Button size="sm" icon={<FileDiff size={13} />} onClick={() => openTab(`review:${changesetId}`)}>
            View changes
          </Button>
        )}
        {path && files.some((f) => f.path === path) && (
          <Button size="sm" onClick={() => void openFile(path)}>
            Open {path.split('/').pop()}
          </Button>
        )}
        {runId && (
          <Button size="sm" icon={<TerminalSquare size={13} />} onClick={() => set({ bottomTab: 'terminal', selectedRun: runId })}>
            Show output
          </Button>
        )}
      </div>
      {Object.keys(rest).length > 0 && (
        <details style={{ marginTop: 10 }} open={ev.kind === 'diagnosis' || ev.kind === 'file_read'}>
          <summary className="small muted" style={{ cursor: 'pointer' }}>
            Details
          </summary>
          <pre className="block" style={{ marginTop: 6 }}>{JSON.stringify(rest, null, 2)}</pre>
        </details>
      )}
    </div>
  );
}

// ---------------------------------------------------------------- terminal

function Elapsed({ since, until }: { since: string; until?: number }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (until) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [until]);
  return <>{duration((until ?? now) - Date.parse(since))}</>;
}

function Terminal() {
  const runs = useWorkspace((s) => s.runs);
  const order = useWorkspace((s) => s.runOrder);
  const selectedRun = useWorkspace((s) => s.selectedRun);
  const set = useWorkspace((s) => s.set);
  const projectId = useWorkspace((s) => s.projectId);
  const toast = useApp((s) => s.toast);
  const outRef = useRef<HTMLDivElement>(null);
  const sel = (selectedRun && runs[selectedRun]) || (order[0] ? runs[order[0]] : undefined);

  useEffect(() => {
    const el = outRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 80) el.scrollTop = el.scrollHeight;
  }, [sel?.lines.length]);
  useEffect(() => {
    if (outRef.current) outRef.current.scrollTop = outRef.current.scrollHeight;
  }, [sel?.info.id]);

  if (order.length === 0) return <Empty icon={<TerminalSquare size={18} />} title="No commands have run yet">Installs, builds, tests and the dev server run here with their real output and exit codes.</Empty>;

  const diagnose = async (runId: string) => {
    if (!projectId) return;
    try {
      await api.startDiagnose(projectId, runId);
      set({ rightTab: 'assistant' });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  return (
    <div style={{ display: 'grid', gridTemplateColumns: '230px 1fr', height: '100%', minHeight: 0 }}>
      <div className="run-list">
        {order.map((id) => {
          const r = runs[id];
          if (!r) return null;
          return (
            <button key={id} className={`run-item ${sel?.info.id === id ? 'on' : ''}`} onClick={() => set({ selectedRun: id })}>
              <div className="row" style={{ gap: 6 }}>
                <Dot status={r.state === 'running' ? 'running' : r.state === 'succeeded' ? 'completed' : r.state === 'cancelled' ? 'cancelled' : 'failed'} />
                <span className="small" style={{ fontWeight: 600 }}>
                  {r.info.op}
                </span>
                <span className="spacer" />
                <span className="faint tiny">{clockTime(r.info.startedAt)}</span>
              </div>
              <span className="mono tiny faint ellipsis">{r.info.command}</span>
            </button>
          );
        })}
      </div>
      {sel ? (
        <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
          <div className="banner" style={{ background: 'var(--bg-1)', flexWrap: 'wrap' }}>
            <span className="mono small">$ {sel.info.command}</span>
            <span className="faint tiny mono ellipsis grow" title={sel.info.cwd}>
              in {sel.info.cwd}
            </span>
            {sel.state === 'running' ? (
              <>
                <Badge tone="run">
                  <Spinner size={9} /> running · <Elapsed since={sel.info.startedAt} />
                </Badge>
                <Button size="sm" variant="danger" icon={<Square size={12} />} onClick={() => void api.stopRun(sel.info.id).catch((e) => toast('bad', errorMessage(e)))}>
                  Stop
                </Button>
              </>
            ) : (
              <>
                <StatusBadge status={sel.state} label={`${sel.state}${sel.outcome?.exitCode != null ? ` · exit ${sel.outcome.exitCode}` : ''}`} />
                {sel.outcome && <span className="faint tiny">{duration(sel.outcome.durationMs)}</span>}
                {sel.state === 'failed' && sel.info.op !== 'dev' && sel.info.op !== 'install' && (
                  <Button size="sm" icon={<Stethoscope size={12} />} onClick={() => void diagnose(sel.info.id)}>
                    Diagnose with Build Doctor
                  </Button>
                )}
              </>
            )}
          </div>
          {sel.info.reason && <div className="faint tiny" style={{ padding: '4px 12px' }}>Why: {sel.info.reason}</div>}
          <div className="scroll term" ref={outRef} style={{ flex: 1 }} role="log" aria-label="Command output">
            {sel.lines.length === 0 && <span className="faint">{sel.state === 'running' ? 'Waiting for output…' : 'No output.'}</span>}
            {sel.lines.map((l, i) => (
              <div key={i} className={l.stream === 'stderr' ? 'err' : undefined}>
                {l.line || ' '}
              </div>
            ))}
          </div>
        </div>
      ) : (
        <Empty title="Select a run" />
      )}
    </div>
  );
}

// ---------------------------------------------------------------- readiness

function Readiness() {
  const projectId = useWorkspace((s) => s.projectId);
  const version = useWorkspace((s) => s.historyVersion + s.gitVersion);
  const [checks, setChecks] = useState<ReadinessCheck[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const load = async () => {
    if (!projectId) return;
    setBusy(true);
    setErr(null);
    try {
      setChecks(await api.readiness(projectId));
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId, version]);
  const icon = (s: ReadinessCheck['status']) =>
    s === 'pass' ? <CheckCircle2 size={16} color="var(--ok)" /> : s === 'fail' ? <XCircle size={16} color="var(--bad)" /> : s === 'warn' ? <AlertTriangle size={16} color="var(--warn)" /> : <HelpCircle size={16} color="var(--text-3)" />;
  return (
    <div className="scroll" style={{ height: '100%' }}>
      <div className="row" style={{ padding: '8px 14px', borderBottom: '1px solid var(--border)' }}>
        <span className="small muted grow">Every check below comes from a real result (process exit codes, Git state, a fresh secret scan). “Unknown” means it has not been run.</span>
        <Button size="sm" icon={<RefreshCw size={12} />} loading={busy} onClick={() => void load()}>
          Re-check
        </Button>
      </div>
      {err && <div style={{ padding: 12, color: 'var(--bad)' }}>{err}</div>}
      <div className="checks">
        {checks?.map((c) => (
          <div key={c.id} className="check">
            {icon(c.status)}
            <strong>{c.label}</strong>
            <span className="muted small" style={{ overflowWrap: 'anywhere' }}>
              {c.detail}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
