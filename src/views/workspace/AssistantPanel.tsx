import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, ChevronRight, Crosshair, FileDiff, Paperclip, Send, Square, Eye, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { bytes, clockTime, STATUS_LABEL } from '../../lib/logic';
import type { ContextPlan, MessageRow, TaskInfo } from '../../lib/types';
import { useApp } from '../../store/app';
import { describePicked, useWorkspace } from '../../store/workspace';
import { Badge, Button, Dot, Empty, Kbd, Spinner } from '../../components/ui';

export default function AssistantPanel() {
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const activity = useWorkspace((s) => s.activity);
  const active = useWorkspace((s) => s.active);
  const picked = useWorkspace((s) => s.picked);
  const setWs = useWorkspace((s) => s.set);
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [request, setRequest] = useState('');
  const [ctx, setCtx] = useState<ContextPlan | null>(null);
  const [showCtx, setShowCtx] = useState(false);
  const [pinned, setPinned] = useState<string[]>([]);
  const [excluded, setExcluded] = useState<string[]>([]);
  const [sending, setSending] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);
  const taskEvents = activity.filter((e) => e.kind === 'task').length;
  const finishedTasks = activity.filter((e) => e.kind === 'task' && e.status !== 'running').length;

  useEffect(() => {
    if (project) api.listMessages(project.id).then(setMessages).catch(() => undefined);
  }, [project, finishedTasks]);
  useEffect(() => bottom.current?.scrollIntoView({ block: 'end' }), [messages.length, taskEvents]);

  // Recompute the context preview (no AI call) while typing, so the user sees what will be shared.
  useEffect(() => {
    if (!project || !request.trim()) {
      setCtx(null);
      return;
    }
    const t = setTimeout(() => {
      const full = picked ? `${request}\n\n${describePicked(picked)}` : request;
      api.previewContext(project.id, full, pinned, excluded).then(setCtx).catch(() => setCtx(null));
    }, 350);
    return () => clearTimeout(t);
  }, [project, request, pinned, excluded, picked]);

  const recentProposals = useMemo(
    () => activity.filter((e) => e.kind === 'changeset' && e.status === 'awaiting_approval' && typeof e.detail.changesetId === 'string').slice(-3),
    [activity],
  );

  if (!project) return null;
  const busyTask = tasks.find((t) => t.kind !== 'verify');
  const openPath = active?.startsWith('file:') ? active.slice(5) : null;

  const send = async () => {
    if (!request.trim() || busyTask) return;
    setSending(true);
    try {
      await api.startChange(project.id, picked ? `${request}\n\n${describePicked(picked)}` : request, pinned, excluded);
      setWs({ picked: null });
      setRequest('');
      setPinned([]);
      setExcluded([]);
      setShowCtx(false);
      setMessages(await api.listMessages(project.id));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setSending(false);
    }
  };

  const included = ctx?.files.filter((f) => f.included) ?? [];
  const sentChars = included.reduce((a, f) => a + f.chars, 0) + request.length;

  return (
    <>
      <div className="scroll grow chat" aria-live="polite">
        {messages.length === 0 && tasks.length === 0 && (
          <Empty title="Ask Genius to change your app">
            Describe a change — “add a dark mode toggle”, “make the task list sortable by due date”. Genius reads only the relevant files, proposes a diff for you to review, then verifies it with a real build.
          </Empty>
        )}
        {messages.map((m) => (
          <div key={m.id} className={`msg ${m.role}`}>
            <div className="who">
              {m.role === 'user' ? 'You' : 'Genius'} · {clockTime(m.createdAt)}
            </div>
            {m.content}
          </div>
        ))}
        {tasks.map((t) => (
          <TaskCard key={t.id} task={t} />
        ))}
        {recentProposals.map((p) => (
          <ProposalCard key={p.id} changesetId={p.detail.changesetId as string} title={p.title} />
        ))}
        <div ref={bottom} />
      </div>
      <div className="composer">
        {picked && (
          <div className="callout info" style={{ alignItems: 'center' }}>
            <Crosshair size={14} color="var(--accent-2)" />
            <div className="grow" style={{ minWidth: 0 }}>
              <div className="small" style={{ fontWeight: 600 }}>
                Selected in preview: <span className="mono">&lt;{picked.tag}&gt;</span> {picked.text && `“${picked.text.slice(0, 50)}”`}
              </div>
              <div className="mono tiny faint ellipsis" title={picked.selector}>
                {picked.selector}
              </div>
            </div>
            <button className="icon-btn" style={{ width: 22, height: 22 }} aria-label="Clear selected element" onClick={() => setWs({ picked: null })}>
              <X size={13} />
            </button>
          </div>
        )}
        <textarea
          className="textarea"
          placeholder={busyTask ? `Waiting for “${busyTask.title}”…` : picked ? 'What should change about the selected element?' : 'Describe a change…'}
          value={request}
          onChange={(e) => setRequest(e.target.value)}
          onKeyDown={(e) => {
            if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
              e.preventDefault();
              void send();
            }
          }}
          aria-label="Change request"
        />
        <div className="row">
          <button className="btn ghost sm" onClick={() => setShowCtx(!showCtx)} disabled={!ctx} aria-expanded={showCtx} title="See exactly what will be sent to NVIDIA">
            {showCtx ? <ChevronDown size={13} /> : <ChevronRight size={13} />}
            <Eye size={13} />
            {ctx ? `${included.length} files · ${bytes(sentChars)} shared` : 'Context'}
          </button>
          {openPath && !pinned.includes(openPath) && (
            <button className="btn ghost sm" onClick={() => setPinned([...pinned, openPath])} title="Always include the open file">
              <Paperclip size={13} /> {openPath.split('/').pop()}
            </button>
          )}
          <span className="spacer" />
          <span className="faint tiny">
            <Kbd>Ctrl</Kbd>+<Kbd>Enter</Kbd>
          </span>
          <Button size="sm" variant="primary" icon={<Send size={13} />} loading={sending} disabled={!request.trim() || !!busyTask || !settings?.model} onClick={() => void send()}>
            Send
          </Button>
        </div>
        {showCtx && ctx && (
          <div className="card" style={{ maxHeight: 220, overflow: 'auto', padding: 8 }}>
            <div className="faint tiny" style={{ marginBottom: 6 }}>
              Sent to NVIDIA ({settings?.model}): your request, project memory, the file list, and the checked files below. Never sent: {ctx.excludedSensitive.length ? ctx.excludedSensitive.join(', ') : 'secret files, lockfiles, binaries'}.
            </div>
            {ctx.files
              .filter((f) => f.included || pinned.includes(f.path) || excluded.includes(f.path) || f.reason !== 'project source')
              .map((f) => (
                <label key={f.path} className="checkbox small" style={{ display: 'flex', padding: '2px 0' }}>
                  <input
                    type="checkbox"
                    checked={f.included && !excluded.includes(f.path)}
                    onChange={(e) => {
                      if (e.target.checked) {
                        setExcluded(excluded.filter((x) => x !== f.path));
                        if (!f.included) setPinned([...pinned, f.path]);
                      } else {
                        setExcluded([...excluded, f.path]);
                        setPinned(pinned.filter((x) => x !== f.path));
                      }
                    }}
                  />
                  <span className="mono tiny grow ellipsis">{f.path}</span>
                  <span className="faint tiny">{f.reason}</span>
                  <span className="faint tiny">{bytes(f.chars)}</span>
                </label>
              ))}
          </div>
        )}
      </div>
    </>
  );
}

/** Live view of one running task: its real events, the file being written, and the raw model stream. */
function TaskCard({ task }: { task: TaskInfo }) {
  const activity = useWorkspace((s) => s.activity);
  const stream = useWorkspace((s) => s.aiStream[task.id]);
  const drafts = useWorkspace((s) => s.drafts);
  const openTab = useWorkspace((s) => s.openTab);
  const [showRaw, setShowRaw] = useState(false);
  const steps = activity.filter((e) => e.taskId === task.id && e.kind !== 'task' && e.kind !== 'file_write');
  const writes = activity.filter((e) => e.taskId === task.id && e.kind === 'file_write');
  const writing = Object.values(drafts).filter((d) => d.taskId === task.id);
  const current = writing.find((d) => d.status === 'streaming');
  return (
    <div className="task-card fade-in">
      <div className="row">
        <Spinner />
        <strong className="grow ellipsis">{task.title}</strong>
        <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={() => void api.cancelTask(task.id)}>
          Cancel
        </Button>
      </div>
      <div className="col" style={{ gap: 3 }}>
        {steps.slice(-8).map((e) => (
          <div key={e.id} className="row small" style={{ gap: 6 }}>
            <Dot status={e.status} />
            <span className="ellipsis grow">{e.title}</span>
            <span className="faint tiny">{STATUS_LABEL[e.status]}</span>
          </div>
        ))}
      </div>
      {current && (
        <button className="btn sm" onClick={() => openTab(`draft:${current.path}`)} style={{ justifyContent: 'flex-start' }}>
          <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} /> Writing <span className="mono">{current.path}</span> ({current.content.split('\n').length} lines)
        </button>
      )}
      {writing.length > 0 && (
        <div className="chips">
          {writing.map((d) => (
            <button key={d.path} className="chip" style={{ border: 0, cursor: 'pointer' }} onClick={() => openTab(`draft:${d.path}`)} title={d.status}>
              {d.status === 'streaming' ? '✎ ' : d.status === 'incomplete' ? '⚠ ' : '◌ '}
              {d.path.split('/').pop()}
            </button>
          ))}
        </div>
      )}
      {writes.length > 0 && <div className="faint tiny">{writes.length} file(s) written to disk by this task.</div>}
      {stream && (
        <>
          <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowRaw(!showRaw)}>
            {showRaw ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Model output ({bytes(stream.length)})
          </button>
          {showRaw && <pre className="stream">{stream.slice(-20000)}</pre>}
        </>
      )}
    </div>
  );
}

function ProposalCard({ changesetId, title }: { changesetId: string; title: string }) {
  const openTab = useWorkspace((s) => s.openTab);
  return (
    <div className="task-card" style={{ borderColor: 'var(--warn)' }}>
      <div className="row">
        <FileDiff size={15} color="var(--warn)" />
        <span className="grow small">{title}</span>
        <Badge tone="warn">Needs review</Badge>
      </div>
      <Button size="sm" onClick={() => openTab(`review:${changesetId}`)}>
        Review diff
      </Button>
    </div>
  );
}
