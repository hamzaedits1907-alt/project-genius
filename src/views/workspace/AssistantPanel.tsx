import { useCallback, useEffect, useRef, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { Bot, ChevronDown, ChevronRight, Crosshair, FileDiff, FileText, ImageIcon, Info, MessageSquarePlus, Paperclip, Send, ShieldCheck, Square, Trash2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { bytes, clockTime } from '../../lib/logic';
import { fileToBase64, reportSummary, stepLabel } from '../../lib/agent';
import type { AgentMode, AgentReport, AgentStep, Attachment, Changeset, Conversation, MessageRow, TaskInfo } from '../../lib/types';
import { useApp } from '../../store/app';
import { describePicked, useWorkspace } from '../../store/workspace';
import { Badge, Button, Empty, Kbd, Segmented, Spinner } from '../../components/ui';
import CapabilitiesDialog from './CapabilitiesDialog';

const KIND_LABEL: Record<string, { label: string; tone: 'ok' | 'bad' | 'warn' | 'accent' | 'idle' }> = {
  answer: { label: 'Answer', tone: 'idle' },
  plan: { label: 'Plan / suggestion', tone: 'accent' },
  proposal: { label: 'Proposed changes', tone: 'warn' },
  error: { label: 'Stopped with an error', tone: 'bad' },
};

/** A persistent, per-project conversation with the Genius agent. */
export default function AssistantPanel() {
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const active = useWorkspace((s) => s.active);
  const picked = useWorkspace((s) => s.picked);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const setWs = useWorkspace((s) => s.set);
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [convId, setConvId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [request, setRequest] = useState('');
  const [mode, setMode] = useState<AgentMode>('build');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [sending, setSending] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [showCaps, setShowCaps] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);
  const projectId = project?.id;
  const agentTask = tasks.find((t) => t.kind === 'agent');
  const busyTask = tasks.find((t) => t.kind !== 'verify');

  const loadConversations = useCallback(async () => {
    if (!projectId) return;
    let list = await api.listConversations(projectId);
    if (list.length === 0) list = [await api.createConversation(projectId, 'Project conversation')];
    setConversations(list);
    setConvId((cur) => (cur && list.some((c) => c.id === cur) ? cur : list[0].id));
  }, [projectId]);

  useEffect(() => {
    loadConversations().catch((e) => toast('bad', errorMessage(e)));
  }, [loadConversations, toast]);

  // Reload messages when the conversation changes and whenever an agent task finishes.
  const finished = useWorkspace((s) => s.activity.filter((e) => e.kind === 'task' && e.status !== 'running').length);
  useEffect(() => {
    if (!convId) return;
    api.conversationMessages(convId).then(setMessages).catch((e) => toast('bad', errorMessage(e)));
  }, [convId, finished, historyVersion, toast]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' });
  }, [messages.length, agentTask?.id]);

  if (!project) return null;
  const openPath = active?.startsWith('file:') ? active.slice(5) : null;

  const newConversation = async () => {
    try {
      const c = await api.createConversation(project.id);
      setConversations([c, ...conversations]);
      setConvId(c.id);
      setMessages([]);
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const deleteConversation = async () => {
    if (!convId || !window.confirm('Delete this conversation? Project files and change history are not affected.')) return;
    try {
      await api.deleteConversation(convId);
      setConvId(null);
      await loadConversations();
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const addFiles = async (files: { name: string; path?: string; file?: File }[]) => {
    for (const f of files) {
      try {
        const a = f.path
          ? await api.importAttachment(project.id, f.path, 'message', convId ?? undefined)
          : await api.importAttachmentData(project.id, f.name, await fileToBase64(f.file!), 'message', convId ?? undefined);
        setAttachments((cur) => [...cur, a]);
        if (a.status === 'failed') toast('bad', `${a.name}: ${a.error}`);
      } catch (e) {
        toast('bad', `${f.name}: ${errorMessage(e)}`);
      }
    }
  };

  const pickFiles = async () => {
    const types = await api.supportedImportTypes();
    const chosen = await openDialog({ multiple: true, title: 'Attach files', filters: [{ name: 'Supported files', extensions: types }] });
    const paths = Array.isArray(chosen) ? chosen : typeof chosen === 'string' ? [chosen] : [];
    await addFiles(paths.map((p) => ({ name: p.split(/[\\/]/).pop() ?? p, path: p })));
  };

  const send = async () => {
    if (!request.trim() || busyTask || !convId) return;
    setSending(true);
    try {
      const message = picked ? `${request}\n\n${describePicked(picked)}` : request;
      await api.startAgent(project.id, { conversationId: convId, message, mode, attachmentIds: attachments.filter((a) => a.status === 'ready').map((a) => a.id), pinned });
      setWs({ picked: null });
      setRequest('');
      setAttachments([]);
      setPinned([]);
      setMessages(await api.conversationMessages(convId));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setSending(false);
    }
  };

  const conv = conversations.find((c) => c.id === convId);
  return (
    <>
      <div className="row conv-bar">
        <select className="input sm grow" aria-label="Conversation" value={convId ?? ''} onChange={(e) => setConvId(e.target.value)} disabled={!!agentTask}>
          {conversations.map((c) => (
            <option key={c.id} value={c.id}>
              {c.title} ({c.messageCount})
            </option>
          ))}
        </select>
        <button className="icon-btn" aria-label="New conversation" title="New conversation (project history is kept)" onClick={() => void newConversation()} disabled={!!agentTask}>
          <MessageSquarePlus size={15} />
        </button>
        <button className="icon-btn" aria-label="Delete conversation" title="Delete this conversation" onClick={() => void deleteConversation()} disabled={!!agentTask || !conv}>
          <Trash2 size={14} />
        </button>
        <button className="icon-btn" aria-label="What can Genius do?" title="What can Genius do right now?" onClick={() => setShowCaps(true)}>
          <ShieldCheck size={15} />
        </button>
      </div>
      <div className="scroll grow chat" aria-live="polite">
        {messages.length === 0 && !agentTask && (
          <Empty icon={<Bot size={20} />} title="Talk to Genius about this project">
            Ask for a feature (“add a sidebar”), a fix (“the sidebar is broken on mobile”), or a question (“how is routing set up?”). Genius inspects the real code with tools, proposes a reviewable diff, and reports exactly what it read, changed and verified.
          </Empty>
        )}
        {messages.map((m) => (
          <Message key={m.id} m={m} />
        ))}
        {tasks
          .filter((t) => t.kind === 'agent' || t.kind === 'change' || t.kind === 'diagnose' || t.kind === 'verify')
          .map((t) => (
            <TaskCard key={t.id} task={t} />
          ))}
        <div ref={bottom} />
      </div>
      <div
        className={`composer ${dragging ? 'dropping' : ''}`}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          void addFiles(Array.from(e.dataTransfer.files).map((file) => ({ name: file.name, file })));
        }}
      >
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
        {(attachments.length > 0 || pinned.length > 0) && (
          <div className="chips" aria-label="Attached to this message">
            {attachments.map((a) => (
              <span key={a.id} className={`chip ${a.status === 'failed' ? 'bad' : ''}`} title={a.error ?? `${a.kind} · ${bytes(a.size)} · ${a.chars} characters extracted`}>
                {a.kind === 'image' ? <ImageIcon size={11} /> : <FileText size={11} />} {a.name} {a.status === 'failed' ? '(failed)' : `· ${bytes(a.size)}`}
                <button className="x" aria-label={`Remove ${a.name}`} onClick={() => setAttachments(attachments.filter((x) => x.id !== a.id))}>
                  <X size={10} />
                </button>
              </span>
            ))}
            {pinned.map((p) => (
              <span key={p} className="chip" title="Pinned project file">
                <Paperclip size={11} /> {p}
                <button className="x" aria-label={`Unpin ${p}`} onClick={() => setPinned(pinned.filter((x) => x !== p))}>
                  <X size={10} />
                </button>
              </span>
            ))}
          </div>
        )}
        <textarea
          className="textarea"
          placeholder={busyTask ? `Waiting for “${busyTask.title}”…` : picked ? 'What should change about the selected element?' : 'Message Genius about this project…'}
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
        <div className="row" style={{ flexWrap: 'wrap' }}>
          <Segmented<AgentMode>
            label="Mode"
            value={mode}
            onChange={setMode}
            options={[
              { value: 'build', label: 'Build' },
              { value: 'plan', label: 'Plan only' },
            ]}
          />
          <button className="btn ghost sm" onClick={() => void pickFiles()} title="Attach files (or drop them here)" aria-label="Attach files">
            <Paperclip size={13} />
          </button>
          {openPath && !pinned.includes(openPath) && (
            <button className="btn ghost sm" onClick={() => setPinned([...pinned, openPath])} title="Include the open file with this message">
              + {openPath.split('/').pop()}
            </button>
          )}
          <span className="spacer" />
          <span className="faint tiny">
            <Kbd>Ctrl</Kbd>+<Kbd>Enter</Kbd>
          </span>
          {agentTask ? (
            <Button size="sm" variant="danger" icon={<Square size={12} />} onClick={() => void api.cancelTask(agentTask.id).catch((e) => toast('bad', errorMessage(e)))}>
              Stop
            </Button>
          ) : (
            <Button size="sm" variant="primary" icon={<Send size={13} />} loading={sending} disabled={!request.trim() || !!busyTask || !settings?.model || !convId} onClick={() => void send()}>
              Send
            </Button>
          )}
        </div>
        <div className="faint tiny row" style={{ gap: 4 }}>
          <Info size={11} /> Sent to {settings?.model ?? 'the selected NVIDIA model'}: your message, attached files, recent conversation, project memory and facts, and files Genius chooses to read (listed live). Secret files are never sent.
        </div>
      </div>
      {showCaps && <CapabilitiesDialog onClose={() => setShowCaps(false)} />}
    </>
  );
}

function Message({ m }: { m: MessageRow }) {
  const meta = (m.meta ?? {}) as Record<string, unknown>;
  const kind = KIND_LABEL[m.kind ?? ''];
  const atts = (meta.attachments as { id: string; name: string; kind: string }[] | undefined) ?? [];
  const report = meta.report as AgentReport | undefined;
  const review = meta.review as { findings?: string; blockers?: number; error?: string } | undefined;
  return (
    <div className={`msg ${m.role}`}>
      <div className="who row" style={{ gap: 6 }}>
        <span>
          {m.role === 'user' ? 'You' : 'Genius'} · {clockTime(m.createdAt)}
        </span>
        {m.role === 'assistant' && kind && <Badge tone={kind.tone}>{kind.label}</Badge>}
        {m.role === 'user' && meta.mode === 'plan' && <Badge tone="accent">plan only</Badge>}
      </div>
      {m.content}
      {atts.length > 0 && (
        <div className="chips" style={{ marginTop: 6 }}>
          {atts.map((a) => (
            <span key={a.id} className="chip">
              {a.kind === 'image' ? <ImageIcon size={11} /> : <FileText size={11} />} {a.name}
            </span>
          ))}
        </div>
      )}
      {typeof meta.changesetId === 'string' && <ProposalStatus changesetId={meta.changesetId} />}
      {review && (review.findings || review.error) && (
        <details className="report">
          <summary>
            Reviewer subagent {review.error ? '— could not run' : review.blockers ? `— ${review.blockers} blocking issue(s)` : '— no blocking issues'}
          </summary>
          <div className="small">{review.error ?? review.findings}</div>
        </details>
      )}
      {report && <Report report={report} />}
    </div>
  );
}

/** Live status of a proposal from the changeset record (not from the message text). */
function ProposalStatus({ changesetId }: { changesetId: string }) {
  const openTab = useWorkspace((s) => s.openTab);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const project = useWorkspace((s) => s.project);
  const [cs, setCs] = useState<Changeset | null>(null);
  useEffect(() => {
    api
      .getChangeset(changesetId)
      .then((d) => setCs(d.changeset))
      .catch(() => setCs(null));
  }, [changesetId, historyVersion]);
  if (!cs) return null;
  const build = project?.status.lastBuild;
  const builtAfter = cs.appliedAt && build && Date.parse(build.at) >= Date.parse(cs.appliedAt);
  const tone = cs.status === 'applied' ? 'ok' : cs.status === 'proposed' ? 'warn' : 'idle';
  return (
    <div className="task-card" style={{ marginTop: 8 }}>
      <div className="row">
        <FileDiff size={15} />
        <span className="grow small">
          {cs.files.length} file(s): {cs.files.map((f) => f.path.split('/').pop()).join(', ')}
        </span>
        <Badge tone={tone}>{cs.status === 'proposed' ? 'awaiting your review' : cs.status}</Badge>
      </div>
      {cs.status === 'applied' && (
        <div className="small">
          Verification:{' '}
          {builtAfter ? (
            <Badge tone={build!.status === 'succeeded' ? 'ok' : 'bad'}>
              build {build!.status} {build!.exitCode != null ? `(exit ${build!.exitCode})` : ''}
            </Badge>
          ) : (
            <span className="faint">no build has run since these changes were applied</span>
          )}
        </div>
      )}
      <Button size="sm" onClick={() => openTab(`review:${changesetId}`)}>
        {cs.status === 'proposed' ? 'Review diff' : 'View diff'}
      </Button>
    </div>
  );
}

function Report({ report }: { report: AgentReport }) {
  const s = reportSummary(report);
  return (
    <details className="report">
      <summary>
        What Genius did · {s}
        {report.warnings.length > 0 && ' · ⚠'}
      </summary>
      <ul className="small">
        <li>Files read: {report.filesRead.length ? report.filesRead.join(', ') : 'none'}</li>
        <li>Searches: {report.searches}</li>
        <li>
          Checks run:{' '}
          {report.checks.length
            ? report.checks.map((c) => `${c.check}: ${c.status ?? 'error'}${c.exitCode != null ? ` (exit ${c.exitCode})` : ''}`).join(', ')
            : 'none'}
        </li>
        {report.subagents.length > 0 && <li>Subagents: {report.subagents.map((x) => `${x.role}${x.ok ? '' : ' (failed)'}`).join(', ')}</li>}
        {report.failedSteps.length > 0 && <li>Failed or refused steps: {report.failedSteps.join('; ')}</li>}
        {report.notVerified.map((n) => (
          <li key={n} className="warn-text">
            Not verified: {n}
          </li>
        ))}
        {report.warnings.map((w) => (
          <li key={w} className="bad-text">
            {w}
          </li>
        ))}
        {report.error && <li className="bad-text">Error: {report.error}</li>}
      </ul>
    </details>
  );
}

/** Live view of a running task: real agent tool calls (with the agent that ran them) and model output. */
function TaskCard({ task }: { task: TaskInfo }) {
  const steps = useWorkspace((s) => s.agentSteps[task.id]) as AgentStep[] | undefined;
  const activity = useWorkspace((s) => s.activity);
  const stream = useWorkspace((s) => s.aiStream[task.id]);
  const drafts = useWorkspace((s) => s.drafts);
  const openTab = useWorkspace((s) => s.openTab);
  const toast = useApp((s) => s.toast);
  const [showRaw, setShowRaw] = useState(false);
  const events = activity.filter((e) => e.taskId === task.id && (e.kind === 'ai' || e.kind === 'approval' || e.kind === 'build' || e.kind === 'test' || e.kind === 'install' || (e.kind === 'task' && e.title.includes('subagent'))));
  const writing = Object.values(drafts).filter((d) => d.taskId === task.id);
  const current = writing.find((d) => d.status === 'streaming');
  const lastAi = events.filter((e) => e.kind === 'ai').slice(-1)[0];
  return (
    <div className="task-card fade-in" aria-label={`Running: ${task.title}`}>
      <div className="row">
        <Spinner />
        <strong className="grow ellipsis">{task.title}</strong>
        <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))}>
          Stop
        </Button>
      </div>
      {lastAi && lastAi.status === 'running' && <div className="small faint">Thinking: {lastAi.title}</div>}
      {steps && steps.length > 0 && (
        <ol className="steps">
          {steps.map((s) => (
            <li key={s.id} className={s.ok ? '' : 'bad-text'} title={JSON.stringify(s.args)}>
              <Badge tone={s.agent === 'main' ? 'idle' : 'accent'}>{s.agent}</Badge> {stepLabel(s)}
            </li>
          ))}
        </ol>
      )}
      {events
        .filter((e) => e.kind !== 'ai')
        .slice(-4)
        .map((e) => (
          <div key={e.id} className="small row" style={{ gap: 6 }}>
            <Badge tone={e.status === 'completed' ? 'ok' : e.status === 'failed' ? 'bad' : 'run'}>{e.status.replace('_', ' ')}</Badge>
            <span className="ellipsis">{e.title}</span>
          </div>
        ))}
      {current && (
        <button className="btn sm" onClick={() => openTab(`draft:${current.path}`)} style={{ justifyContent: 'flex-start' }}>
          <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} /> Drafting <span className="mono">{current.path}</span> (not saved)
        </button>
      )}
      {stream && (
        <>
          <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowRaw(!showRaw)}>
            {showRaw ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Raw model output ({bytes(stream.length)})
          </button>
          {showRaw && <pre className="stream">{stream.slice(-20000)}</pre>}
        </>
      )}
    </div>
  );
}
