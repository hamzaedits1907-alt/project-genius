import { useCallback, useEffect, useRef, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { ArrowUp, Check, ChevronDown, ChevronRight, Circle, CircleDot, Crosshair, FileDiff, FileText, Globe, ImageIcon, Info, MessageSquarePlus, Paperclip, ShieldCheck, Square, Trash2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { bytes, clockTime } from '../../lib/logic';
import { AGENTS, askOf, fileToBase64, latestNote, latestTodo, reportSummary, stepLabel, type Ask } from '../../lib/agent';
import type { AgentMode, AgentReport, AgentStep, Attachment, Changeset, Conversation, MessageRow, TaskInfo } from '../../lib/types';
import { useApp } from '../../store/app';
import { describePicked, useWorkspace } from '../../store/workspace';
import { Badge, Button, Segmented } from '../../components/ui';
import { GeniusAvatar, moodFor } from '../../components/GeniusAvatar';
import { Markdown } from '../../components/Markdown';
import CapabilitiesDialog from './CapabilitiesDialog';

const KIND_LABEL: Record<string, { label: string; tone: 'ok' | 'bad' | 'warn' | 'accent' | 'idle' }> = {
  answer: { label: 'Answer', tone: 'idle' },
  plan: { label: 'Plan / suggestion', tone: 'accent' },
  question: { label: 'Question', tone: 'accent' },
  proposal: { label: 'Proposed changes', tone: 'warn' },
  error: { label: 'Stopped with an error', tone: 'bad' },
};

/** Tasks shown live in the conversation. */
const CHAT_TASKS = ['agent', 'change', 'diagnose', 'verify', 'plan', 'generate'];

function suggestions(hasCode: boolean): string[] {
  return hasCode
    ? ['Make the design look more modern and polished', 'Add a dark mode toggle', 'Make it work great on mobile', 'What can you do?']
    : ['Build me a portfolio website', 'What can you do?', 'Plan a landing page for a coffee shop'];
}

/** A persistent, per-project conversation with the Genius agent. */
export default function AssistantPanel({ variant = 'advanced' }: { variant?: 'simple' | 'advanced' }) {
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const active = useWorkspace((s) => s.active);
  const picked = useWorkspace((s) => s.picked);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const hasCode = useWorkspace((s) => s.files.some((f) => f.path === 'src/App.tsx'));
  const drafting = useWorkspace((s) => Object.values(s.drafts).some((d) => d.status === 'streaming'));
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
  const [happyUntil, setHappyUntil] = useState(0);
  const bottom = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const projectId = project?.id;
  const agentTask = tasks.find((t) => t.kind === 'agent');
  const busyTask = tasks.find((t) => t.kind !== 'verify');
  const name = settings?.assistantName?.trim() || 'Genius';

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
    api
      .conversationMessages(convId)
      .then((m) => {
        setMessages((prev) => {
          const last = m[m.length - 1];
          if (prev.length && m.length > prev.length && last?.role === 'assistant' && last.kind !== 'error') setHappyUntil(Date.now() + 5000);
          return m;
        });
      })
      .catch((e) => toast('bad', errorMessage(e)));
  }, [convId, finished, historyVersion, toast]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: 'end' });
  }, [messages.length, agentTask?.id, tasks.length]);

  useEffect(() => {
    if (happyUntil <= Date.now()) return;
    const t = setTimeout(() => setHappyUntil(0), happyUntil - Date.now());
    return () => clearTimeout(t);
  }, [happyUntil]);

  // Grow the composer with its content.
  useEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
  }, [request]);

  if (!project) return null;
  const openPath = active?.startsWith('file:') ? active.slice(5) : null;
  const lastMsg = messages[messages.length - 1];
  const mood = moodFor({
    busy: busyTask ? (drafting ? 'working' : 'thinking') : null,
    failed: lastMsg?.role === 'assistant' && lastMsg.kind === 'error',
    typing: request.trim().length > 0,
    justFinished: happyUntil > Date.now(),
  });

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

  const send = async (text = request) => {
    if (!text.trim() || busyTask || !convId) return;
    setSending(true);
    try {
      const message = picked ? `${text}\n\n${describePicked(picked)}` : text;
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
  const chatTasks = tasks.filter((t) => CHAT_TASKS.includes(t.kind));
  const status = busyTask ? busyTask.title.replace(/^Genius: /, 'Working on: ') : lastMsg?.kind === 'error' ? 'Something went wrong — tell me what to try next' : settings?.model ? 'Ready when you are' : 'Choose a model in Settings to start';
  const canSend = !!request.trim() && !busyTask && !!settings?.model && !!convId;

  return (
    <>
      <div className="chat-head">
        <GeniusAvatar size={34} mood={mood} />
        <div className="grow" style={{ minWidth: 0 }}>
          <div className="chat-name">{name}</div>
          <div className="chat-status ellipsis" title={status}>
            {busyTask && <span className="dot run" />} {status}
          </div>
        </div>
        <select className="input sm conv-select" aria-label="Conversation" value={convId ?? ''} onChange={(e) => setConvId(e.target.value)} disabled={!!agentTask}>
          {conversations.map((c) => (
            <option key={c.id} value={c.id}>
              {c.title} ({c.messageCount})
            </option>
          ))}
        </select>
        <button className="icon-btn" aria-label="New conversation" title="New conversation (project history is kept)" onClick={() => void newConversation()} disabled={!!agentTask}>
          <MessageSquarePlus size={15} />
        </button>
        {variant === 'advanced' && (
          <button className="icon-btn" aria-label="Delete conversation" title="Delete this conversation" onClick={() => void deleteConversation()} disabled={!!agentTask || !conv}>
            <Trash2 size={14} />
          </button>
        )}
        <button className="icon-btn" aria-label="What can Genius do?" title={`What can ${name} do right now?`} onClick={() => setShowCaps(true)}>
          <ShieldCheck size={15} />
        </button>
      </div>
      <div className="scroll grow chat" aria-live="polite">
        {messages.length === 0 && chatTasks.length === 0 && (
          <div className="chat-hello fade-in">
            <GeniusAvatar size={84} mood={mood} />
            <h3>Hi, I'm {name}!</h3>
            <p>
              Tell me what to build or change — “add a pricing page”, “the menu is broken on mobile”, or just ask a question. I'll look at the real code, explain my plan, and show you every change before it lands.
            </p>
            <div className="suggestions">
              {suggestions(hasCode).map((s) => (
                <button key={s} onClick={() => (settings?.model ? void send(s) : setRequest(s))} disabled={!!busyTask}>
                  {s}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((m, i) => (
          <Message key={m.id} m={m} name={name} isLast={i === messages.length - 1} busy={!!busyTask} onAnswer={(t) => void send(t)} />
        ))}
        {chatTasks.map((t) => (
          <TaskCard key={t.id} task={t} expanded={variant === 'advanced'} name={name} />
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
        <div className="composer-box">
          <textarea
            ref={input}
            rows={1}
            placeholder={busyTask ? `${name} is working on “${busyTask.title.replace(/^Genius: /, '')}”…` : picked ? 'What should change about the selected element?' : `Message ${name}…`}
            value={request}
            onChange={(e) => setRequest(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                void send();
              }
            }}
            aria-label="Change request"
          />
          <div className="row composer-tools">
            <button className="icon-btn" onClick={() => void pickFiles()} title="Attach files, images or docs (or drop them here)" aria-label="Attach files">
              <Paperclip size={15} />
            </button>
            {openPath && !pinned.includes(openPath) && (
              <button className="btn ghost sm" onClick={() => setPinned([...pinned, openPath])} title="Include the open file with this message">
                + {openPath.split('/').pop()}
              </button>
            )}
            <Segmented<AgentMode>
              label="Mode"
              value={mode}
              onChange={setMode}
              options={[
                { value: 'build', label: 'Build' },
                { value: 'plan', label: 'Plan only' },
              ]}
            />
            {settings?.internetAccess && (
              <span className="faint" title={`${name} can search the web and read pages (turn off in Settings → AI provider)`}>
                <Globe size={13} />
              </span>
            )}
            <span className="spacer" />
            {agentTask ? (
              <Button size="sm" variant="danger" icon={<Square size={12} />} onClick={() => void api.cancelTask(agentTask.id).catch((e) => toast('bad', errorMessage(e)))}>
                Stop
              </Button>
            ) : (
              <button className="send-btn" aria-label="Send" title="Send (Enter)" disabled={!canSend || sending} onClick={() => void send()}>
                {sending ? <span className="spinner" /> : <ArrowUp size={16} />}
              </button>
            )}
          </div>
        </div>
        {variant === 'advanced' && (
          <div className="faint tiny row" style={{ gap: 4 }}>
            <Info size={11} /> Sent to {settings?.model ?? 'the selected NVIDIA model'}: your message, attached files, recent conversation, project memory and facts, and files {name} chooses to read (listed live). Secret files are never sent.
          </div>
        )}
      </div>
      {showCaps && <CapabilitiesDialog onClose={() => setShowCaps(false)} />}
    </>
  );
}

function Message({ m, name, isLast, busy, onAnswer }: { m: MessageRow; name: string; isLast: boolean; busy: boolean; onAnswer: (text: string) => void }) {
  const meta = (m.meta ?? {}) as Record<string, unknown>;
  const kind = KIND_LABEL[m.kind ?? ''];
  const atts = (meta.attachments as { id: string; name: string; kind: string }[] | undefined) ?? [];
  const report = meta.report as AgentReport | undefined;
  const review = meta.review as { findings?: string; blockers?: number; error?: string } | undefined;
  const ask = askOf(meta);
  if (m.role === 'user') {
    return (
      <div className="msg user fade-in">
        <div className="who">
          You · {clockTime(m.createdAt)} {meta.mode === 'plan' && <Badge tone="accent">plan only</Badge>}
        </div>
        <Markdown text={m.content} />
        {atts.length > 0 && (
          <div className="chips" style={{ marginTop: 6 }}>
            {atts.map((a) => (
              <span key={a.id} className="chip">
                {a.kind === 'image' ? <ImageIcon size={11} /> : <FileText size={11} />} {a.name}
              </span>
            ))}
          </div>
        )}
      </div>
    );
  }
  return (
    <div className="msg-row fade-in">
      <GeniusAvatar size={26} mood={m.kind === 'error' ? 'sad' : 'idle'} className="msg-avatar" />
      <div className={`msg assistant ${m.kind === 'error' ? 'error' : ''}`}>
        <div className="who row" style={{ gap: 6 }}>
          <span>
            {name} · {clockTime(m.createdAt)}
          </span>
          {kind && m.kind !== 'answer' && <Badge tone={kind.tone}>{kind.label}</Badge>}
        </div>
        <Markdown text={m.content} />
        {ask && <AskCard ask={ask} active={isLast && !busy} onAnswer={onAnswer} />}
        {typeof meta.changesetId === 'string' && <ProposalStatus changesetId={meta.changesetId} />}
        {review && (review.findings || review.error) && (
          <details className="report">
            <summary>Reviewer subagent {review.error ? '— could not run' : review.blockers ? `— ${review.blockers} blocking issue(s)` : '— no blocking issues'}</summary>
            <div className="small">
              <Markdown text={review.error ?? review.findings ?? ''} />
            </div>
          </details>
        )}
        {report && <Report report={report} />}
      </div>
    </div>
  );
}

/** A question Genius asked, with one-click answers (the user can also just type). */
function AskCard({ ask, active, onAnswer }: { ask: Ask; active: boolean; onAnswer: (text: string) => void }) {
  const [chosen, setChosen] = useState<string[]>([]);
  return (
    <div className={`ask ${active ? '' : 'done'}`}>
      <div className="ask-q">{ask.question}</div>
      <div className="ask-options">
        {ask.options.map((o) =>
          ask.multi ? (
            <label key={o} className={`ask-opt ${chosen.includes(o) ? 'on' : ''}`}>
              <input type="checkbox" disabled={!active} checked={chosen.includes(o)} onChange={(e) => setChosen(e.target.checked ? [...chosen, o] : chosen.filter((x) => x !== o))} />
              {o}
            </label>
          ) : (
            <button key={o} className="ask-opt" disabled={!active} onClick={() => onAnswer(o)}>
              {o}
            </button>
          ),
        )}
      </div>
      {ask.multi && active && (
        <Button size="sm" variant="primary" disabled={!chosen.length} onClick={() => onAnswer(chosen.join(', '))}>
          Send answer
        </Button>
      )}
      {active && <div className="faint tiny">…or type your own answer below.</div>}
    </div>
  );
}

/** Live status of a proposal from the changeset record (not from the message text). */
function ProposalStatus({ changesetId }: { changesetId: string }) {
  const openTab = useWorkspace((s) => s.openTab);
  const setWs = useWorkspace((s) => s.set);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const project = useWorkspace((s) => s.project);
  const toast = useApp((s) => s.toast);
  const [cs, setCs] = useState<Changeset | null>(null);
  const [busy, setBusy] = useState<'apply' | 'reject' | null>(null);
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
  const act = async (what: 'apply' | 'reject') => {
    setBusy(what);
    try {
      const next = what === 'apply' ? await api.applyChangeset(cs.id) : await api.rejectChangeset(cs.id);
      setCs(next);
      if (what === 'apply') toast('ok', `Applied ${cs.files.length} file change(s).`);
      useWorkspace.setState((s) => ({ historyVersion: s.historyVersion + 1, gitVersion: s.gitVersion + 1 }));
      void useWorkspace.getState().refreshFiles();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="task-card proposal">
      <div className="row">
        <FileDiff size={15} />
        <span className="grow small ellipsis" title={cs.files.map((f) => f.path).join('\n')}>
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
      <div className="row wrap">
        {cs.status === 'proposed' && (
          <>
            <Button size="sm" variant="primary" icon={<Check size={13} />} loading={busy === 'apply'} disabled={!!busy} onClick={() => void act('apply')}>
              Apply
            </Button>
            <Button size="sm" variant="ghost" icon={<X size={13} />} loading={busy === 'reject'} disabled={!!busy} onClick={() => void act('reject')}>
              Discard
            </Button>
          </>
        )}
        <Button
          size="sm"
          onClick={() => {
            openTab(`review:${changesetId}`);
            setWs({ stage: 'code' });
          }}
        >
          {cs.status === 'proposed' ? 'Review diff' : 'View diff'}
        </Button>
      </div>
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
          {report.checks.length ? report.checks.map((c) => `${c.check}: ${c.status ?? 'error'}${c.exitCode != null ? ` (exit ${c.exitCode})` : ''}`).join(', ') : 'none'}
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

function AgentBadge({ agent }: { agent: string }) {
  const a = AGENTS[agent] ?? { label: agent, hint: '' };
  return (
    <span className={`agent-badge agent-${agent}`} title={a.hint}>
      {a.label}
    </span>
  );
}

/** Live view of a running task: what Genius says it's doing, its checklist, real tool calls and model output. */
function TaskCard({ task, expanded, name }: { task: TaskInfo; expanded: boolean; name: string }) {
  const steps = useWorkspace((s) => s.agentSteps[task.id]) as AgentStep[] | undefined;
  const activity = useWorkspace((s) => s.activity);
  const stream = useWorkspace((s) => s.aiStream[task.id]);
  const drafts = useWorkspace((s) => s.drafts);
  const openTab = useWorkspace((s) => s.openTab);
  const setWs = useWorkspace((s) => s.set);
  const toast = useApp((s) => s.toast);
  const [showRaw, setShowRaw] = useState(false);
  const [showSteps, setShowSteps] = useState(expanded);
  const events = activity.filter((e) => e.taskId === task.id && (e.kind === 'ai' || e.kind === 'approval' || e.kind === 'build' || e.kind === 'test' || e.kind === 'install' || (e.kind === 'task' && e.title.includes('subagent'))));
  const writing = Object.values(drafts).filter((d) => d.taskId === task.id);
  const current = writing.find((d) => d.status === 'streaming');
  const lastAi = events.filter((e) => e.kind === 'ai').slice(-1)[0];
  const todo = latestTodo(steps);
  const note = latestNote(steps);
  const visibleSteps = (steps ?? []).filter((s) => s.tool !== 'todo');
  const title = task.kind === 'plan' ? `${name} is planning your app` : task.kind === 'generate' ? `${name} is writing your app` : task.title.replace(/^Genius: /, '');
  return (
    <div className="msg-row fade-in">
      <GeniusAvatar size={26} mood={current ? 'working' : 'thinking'} className="msg-avatar" />
      <div className="task-card live" aria-label={`Running: ${task.title}`}>
        <div className="row">
          <strong className="grow ellipsis">{title}</strong>
          <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))}>
            Stop
          </Button>
        </div>
        {note ? <div className="task-note">{note}</div> : lastAi && lastAi.status === 'running' && <div className="small faint thinking-dots">Thinking</div>}
        {todo.length > 0 && (
          <ul className="todo" aria-label="Genius's plan">
            {todo.map((t, i) => (
              <li key={i} className={t.status}>
                {t.status === 'done' ? <Check size={13} /> : t.status === 'active' ? <CircleDot size={13} /> : <Circle size={13} />}
                <span>{t.text}</span>
              </li>
            ))}
          </ul>
        )}
        {visibleSteps.length > 0 && (
          <>
            <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowSteps(!showSteps)} aria-expanded={showSteps}>
              {showSteps ? <ChevronDown size={12} /> : <ChevronRight size={12} />} {visibleSteps.length} step{visibleSteps.length === 1 ? '' : 's'} · last: {visibleSteps[visibleSteps.length - 1].tool.replace(/_/g, ' ')}
            </button>
            <ol className="steps" hidden={!showSteps}>
              {visibleSteps.map((s) => (
                <li key={s.id} className={s.ok ? '' : 'bad-text'} title={JSON.stringify(s.args)}>
                  <AgentBadge agent={s.agent} /> <span>{stepLabel(s)}</span>
                </li>
              ))}
            </ol>
          </>
        )}
        {events
          .filter((e) => e.kind !== 'ai')
          .slice(-3)
          .map((e) => (
            <div key={e.id} className="small row" style={{ gap: 6 }}>
              <Badge tone={e.status === 'completed' ? 'ok' : e.status === 'failed' ? 'bad' : 'run'}>{e.status.replace('_', ' ')}</Badge>
              <span className="ellipsis">{e.title}</span>
            </div>
          ))}
        {writing.length > 0 && (
          <div className="files-written">
            {writing.map((d) => (
              <button
                key={d.path}
                className={`file-pill ${d.status}`}
                onClick={() => {
                  openTab(`draft:${d.path}`);
                  setWs({ stage: 'code' });
                }}
                title={d.status === 'streaming' ? 'Being written now (not saved yet)' : d.path}
              >
                {d.status === 'streaming' ? <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} /> : <Check size={11} />}
                {d.path.split('/').pop()}
              </button>
            ))}
          </div>
        )}
        {stream && expanded && (
          <>
            <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowRaw(!showRaw)}>
              {showRaw ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Raw model output ({bytes(stream.length)})
            </button>
            {showRaw && <pre className="stream">{stream.slice(-20000)}</pre>}
          </>
        )}
      </div>
    </div>
  );
}
