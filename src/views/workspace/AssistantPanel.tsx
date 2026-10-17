import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { ArrowUp, AtSign, Crosshair, FileText, Globe, ImageIcon, Info, MessageSquarePlus, Paperclip, ShieldCheck, Slash, Square, Trash2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { bytes } from '../../lib/logic';
import { fileToBase64 } from '../../lib/agent';
import type { AgentMode, Attachment, Conversation, Effort, FileEntry, MessageRow, RewindResult, SlashCommand } from '../../lib/types';
import { useApp } from '../../store/app';
import { describePicked, useWorkspace } from '../../store/workspace';
import { Button, Segmented } from '../../components/ui';
import { GeniusAvatar, moodFor } from '../../components/GeniusAvatar';
import CapabilitiesDialog from './CapabilitiesDialog';
import { ACTION_NAMES, EFFORTS, filterCommands, filterFiles, isEffort, loadAutopilot, mentionQuery, parseCommand, saveAutopilot, slashQuery } from './assistant/commands';
import { useCommands } from './assistant/useCommands';
import { SuggestPopup, type SuggestItem } from './assistant/SuggestPopup';
import { Message, changedFiles, type MessageActions } from './assistant/Message';
import { TaskCard } from './assistant/TaskCard';
import { AutopilotToggle, ContextMeter, EffortSelect, HelpPanel, QueuedBubble } from './assistant/ComposerExtras';

/** Tasks shown live in the conversation. */
const CHAT_TASKS = ['agent', 'change', 'diagnose', 'verify', 'plan', 'generate'];

function suggestions(hasCode: boolean): string[] {
  return hasCode
    ? ['Make the design look more modern and polished', 'Add a dark mode toggle', 'Make it work great on mobile', 'What can you do?']
    : ['Build me a portfolio website', 'What can you do?', 'Plan a landing page for a coffee shop'];
}

const queueId = () => `q-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

/** A persistent, per-project conversation with the Genius agent. */
export default function AssistantPanel({ variant = 'advanced' }: { variant?: 'simple' | 'advanced' }) {
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const active = useWorkspace((s) => s.active);
  const picked = useWorkspace((s) => s.picked);
  const files = useWorkspace((s) => s.files);
  const queue = useWorkspace((s) => s.queue);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const drafting = useWorkspace((s) => Object.values(s.drafts).some((d) => d.status === 'streaming'));
  const setWs = useWorkspace((s) => s.set);
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const navigate = useApp((s) => s.navigate);
  const commands = useCommands();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [convId, setConvId] = useState<string | null>(null);
  const [messages, setMessages] = useState<MessageRow[]>([]);
  const [request, setRequest] = useState('');
  const [caret, setCaret] = useState(0);
  const [popupIndex, setPopupIndex] = useState(0);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [mode, setMode] = useState<AgentMode>('build');
  const [effort, setEffort] = useState<Effort>(() => (isEffort(settings?.effort) ? settings.effort : 'max'));
  const [autopilot, setAutopilotState] = useState(() => loadAutopilot(variant));
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [sending, setSending] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [showCaps, setShowCaps] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [happyUntil, setHappyUntil] = useState(0);
  const bottom = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const pendingCaret = useRef<number | null>(null);
  const projectId = project?.id;
  const agentTask = tasks.find((t) => t.kind === 'agent');
  const busyTask = tasks.find((t) => t.kind !== 'verify');
  const name = settings?.assistantName?.trim() || 'Genius';
  const settingsEffort = settings?.effort;

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

  // The saved default effort wins whenever settings change; Autopilot is remembered per mode.
  useEffect(() => {
    if (isEffort(settingsEffort)) setEffort(settingsEffort);
  }, [settingsEffort]);
  useEffect(() => {
    setAutopilotState(loadAutopilot(variant));
  }, [variant]);
  const setAutopilot = (on: boolean) => {
    setAutopilotState(on);
    saveAutopilot(variant, on);
  };

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
  }, [messages.length, agentTask?.id, tasks.length, queue.length]);

  useEffect(() => {
    if (happyUntil <= Date.now()) return;
    const t = setTimeout(() => setHappyUntil(0), happyUntil - Date.now());
    return () => clearTimeout(t);
  }, [happyUntil]);

  // Grow the composer with its content, and place the caret after programmatic edits.
  useLayoutEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
    if (pendingCaret.current != null) {
      el.focus();
      el.setSelectionRange(pendingCaret.current, pendingCaret.current);
      pendingCaret.current = null;
    }
  }, [request]);

  // Messages queued while Genius worked: once it is free, whatever it did not read is sent as a
  // normal message (and delivered ones are cleared — the conversation now holds them).
  const sendRef = useRef<(text?: string) => Promise<void>>(async () => {});
  useEffect(() => {
    if (busyTask || sending || !convId) return;
    const mine = queue.filter((q) => q.conversationId === convId);
    if (!mine.length) return;
    const pending = mine.filter((q) => q.status !== 'delivered');
    useWorkspace.getState().removeQueued(mine.map((q) => q.id));
    if (pending.length) void sendRef.current(pending.map((q) => q.text).join('\n\n'));
  }, [busyTask, sending, convId, queue]);

  if (!project) return null;
  const openPath = active?.startsWith('file:') ? active.slice(5) : null;
  const lastMsg = messages[messages.length - 1];
  const mood = moodFor({
    busy: busyTask ? (drafting ? 'working' : 'thinking') : null,
    failed: lastMsg?.role === 'assistant' && lastMsg.kind === 'error',
    typing: request.trim().length > 0,
    justFinished: happyUntil > Date.now(),
  });

  const setText = (text: string, at = text.length) => {
    setRequest(text);
    setCaret(at);
    setPopupIndex(0);
    pendingCaret.current = at;
  };

  const refreshAfterRewind = async () => {
    if (convId) setMessages(await api.conversationMessages(convId));
    useWorkspace.setState((s) => ({ historyVersion: s.historyVersion + 1, gitVersion: s.gitVersion + 1 }));
    const ws = useWorkspace.getState();
    void ws.refreshFiles();
    void ws.refreshProject();
    void loadConversations().catch(() => undefined);
  };

  const rewind = async (messageId: string): Promise<RewindResult | null> => {
    if (!convId) return null;
    try {
      const r = await api.rewindConversation(convId, messageId, true);
      await refreshAfterRewind();
      return r;
    } catch (e) {
      toast('bad', `Could not rewind: ${errorMessage(e)}`);
      return null;
    }
  };

  const rewound = (r: RewindResult) =>
    toast('ok', `Rewound ${r.removedMessages} message${r.removedMessages === 1 ? '' : 's'}${r.revertedChangesets ? ` and undid ${r.revertedChangesets} change${r.revertedChangesets === 1 ? '' : 's'}` : ''}. Everything stays revertible in History.`);

  const idleOrWarn = () => {
    if (!busyTask) return true;
    toast('info', `Wait until ${name} finishes (or press Stop).`);
    return false;
  };

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

  const compact = async () => {
    if (!convId || !idleOrWarn()) return;
    try {
      await api.compactConversation(convId);
      setMessages(await api.conversationMessages(convId));
      toast('ok', 'Conversation compacted: older messages were summarized.');
    } catch (e) {
      toast('bad', `Could not compact: ${errorMessage(e)}`);
    }
  };

  const undo = async () => {
    if (!idleOrWarn()) return;
    const last = [...messages].reverse().find((m) => m.role === 'user');
    if (!last) return toast('info', 'Nothing to undo yet.');
    if (!window.confirm(`Undo your last request (“${last.content.slice(0, 80)}”)?\n\n${name} forgets it and its answer, and the file changes it made are undone. They stay revertible in History.`)) return;
    const r = await rewind(last.id);
    if (!r) return;
    rewound(r);
    setText(last.content);
  };

  /** Client-side commands. Returns false for names the UI does not implement. */
  const runAction = async (cmd: string, arg: string): Promise<boolean> => {
    const a = arg.toLowerCase();
    switch (cmd) {
      case 'clear':
        if (idleOrWarn()) await newConversation();
        return true;
      case 'plan': {
        const next = mode === 'plan' ? 'build' : 'plan';
        setMode(next);
        toast('info', next === 'plan' ? `Plan only: ${name} will explain what it would do without changing files.` : `Build mode: ${name} can change files.`);
        return true;
      }
      case 'build':
        setMode('build');
        toast('info', `Build mode: ${name} can change files.`);
        return true;
      case 'effort':
        if (isEffort(a)) {
          setEffort(a);
          toast('ok', EFFORTS.find((e) => e.value === a)!.hint);
        } else toast('info', `Effort is ${effort}. Use /effort fast, /effort balanced or /effort max.`);
        return true;
      case 'autopilot': {
        const on = a === 'on' ? true : a === 'off' ? false : a ? null : !autopilot;
        if (on == null) toast('info', 'Use /autopilot on or /autopilot off.');
        else {
          setAutopilot(on);
          toast('ok', `Autopilot ${on ? 'on' : 'off'}.`);
        }
        return true;
      }
      case 'undo':
        await undo();
        return true;
      case 'compact':
        await compact();
        return true;
      case 'model':
        navigate({ name: 'settings', tab: 'ai' });
        return true;
      case 'help':
        setShowHelp(true);
        return true;
      default:
        return false;
    }
  };

  /** While Genius works, messages are handed to the running agent (or held until it is free). */
  const queueMessage = (text: string) => {
    if (!convId) return;
    const id = queueId();
    const taskId = agentTask?.id ?? null;
    useWorkspace.getState().enqueue({ id, taskId, conversationId: convId, text, status: 'queued' });
    setText('');
    if (taskId) api.steerAgent(taskId, text).catch(() => useWorkspace.getState().updateQueued(id, { status: 'undelivered' }));
  };

  const send = async (text = request) => {
    const trimmed = text.trim();
    if (!trimmed || !convId) return;
    const cmd = parseCommand(trimmed);
    if (cmd && ACTION_NAMES.has(cmd.name)) {
      setText('');
      await runAction(cmd.name, cmd.arg);
      return;
    }
    if (busyTask) return queueMessage(trimmed);
    if (sending) return;
    setSending(true);
    try {
      const message = picked ? `${trimmed}\n\n${describePicked(picked)}` : trimmed;
      await api.startAgent(project.id, { conversationId: convId, message, mode, attachmentIds: attachments.filter((a) => a.status === 'ready').map((a) => a.id), pinned, effort, autopilot });
      setWs({ picked: null });
      setText('');
      setAttachments([]);
      setPinned([]);
      setMessages(await api.conversationMessages(convId));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setSending(false);
    }
  };
  sendRef.current = send;

  const actions: MessageActions = {
    onEdit: async (m, text) => {
      if (!idleOrWarn()) return false;
      if (!window.confirm(`Edit and resend this message?\n\n${name} forgets this message and everything after it, and the file changes made since then are undone. They stay revertible in History.`)) return false;
      const r = await rewind(m.id);
      if (!r) return false;
      await send(text);
      return true;
    },
    onRewind: (m) => {
      if (!idleOrWarn()) return;
      if (!window.confirm(`Rewind to before this message?\n\n${name} forgets it and everything after it, and the file changes made since then are undone. They stay revertible in History.`)) return;
      void rewind(m.id).then((r) => {
        if (!r) return;
        rewound(r);
        setText(m.content);
      });
    },
    onRetry: (m) => {
      if (!idleOrWarn()) return;
      const i = messages.findIndex((x) => x.id === m.id);
      const prev = messages.slice(0, i).reverse().find((x) => x.role === 'user');
      if (!prev) return;
      if (changedFiles(m) && !window.confirm(`Retry this request?\n\nThe changes from this answer are undone first. They stay revertible in History.`)) return;
      void rewind(prev.id).then((r) => {
        if (r) void send(prev.content);
      });
    },
  };

  const addFiles = async (picks: { name: string; path?: string; file?: File }[]) => {
    for (const f of picks) {
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

  // ---- slash commands and @-mentions
  const slash = slashQuery(request, caret);
  const mention = slash == null ? mentionQuery(request, caret) : null;
  const cmdMatches: SlashCommand[] = slash != null ? filterCommands(commands, slash) : [];
  const fileMatches: FileEntry[] = mention ? filterFiles(files, mention.query) : [];
  const popup = dismissed === request ? null : slash != null ? 'slash' : mention ? 'mention' : null;
  const popupItems: SuggestItem[] =
    popup === 'slash'
      ? cmdMatches.map((c) => ({ key: c.name, icon: <Slash size={12} />, label: <span className="mono">/{c.name}{c.args ? <span className="faint"> {c.args}</span> : null}</span>, detail: c.description }))
      : popup === 'mention'
        ? fileMatches.map((f) => {
            const at = f.path.lastIndexOf('/');
            return { key: f.path, icon: <FileText size={12} />, label: <span className="mono">{f.path.slice(at + 1)}</span>, detail: at > 0 ? f.path.slice(0, at) : '' };
          })
        : [];
  const sel = Math.min(popupIndex, Math.max(0, popupItems.length - 1));

  const pickCommand = (c: SlashCommand) => {
    if (c.args) return setText(`/${c.name} `);
    if (c.kind === 'action' && ACTION_NAMES.has(c.name)) {
      setText('');
      void runAction(c.name, '');
      return;
    }
    void send(`/${c.name}`);
  };

  const pickFile = (f: FileEntry) => {
    if (!mention) return;
    const before = request.slice(0, mention.start);
    const after = request.slice(caret).replace(/^\s+/, '');
    const insert = `@${f.path} `;
    setText(before + insert + after, before.length + insert.length);
    if (!pinned.includes(f.path)) setPinned([...pinned, f.path]);
  };

  const pick = (i: number) => {
    if (popup === 'slash' && cmdMatches[i]) pickCommand(cmdMatches[i]);
    else if (popup === 'mention' && fileMatches[i]) pickFile(fileMatches[i]);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    const composing = e.nativeEvent.isComposing;
    if (popup) {
      const n = popupItems.length;
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        setDismissed(request);
        return;
      }
      if (n > 0 && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
        e.preventDefault();
        setPopupIndex((sel + (e.key === 'ArrowDown' ? 1 : n - 1)) % n);
        return;
      }
      if (n > 0 && !composing && (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey))) {
        e.preventDefault();
        pick(sel);
        return;
      }
    }
    if (e.key === 'Escape' && showHelp) {
      e.stopPropagation();
      setShowHelp(false);
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !composing) {
      e.preventDefault();
      void send();
    }
  };

  const conv = conversations.find((c) => c.id === convId);
  const chatTasks = tasks.filter((t) => CHAT_TASKS.includes(t.kind));
  const queued = queue.filter((q) => q.conversationId === convId);
  const anyTask = busyTask ?? tasks[0];
  const hasModel = !!settings?.model || settings?.modelMode === 'auto';
  const status = anyTask ? anyTask.title.replace(/^Genius: /, 'Working on: ') : lastMsg?.kind === 'error' ? 'Something went wrong — tell me what to try next' : hasModel ? 'Ready when you are' : 'Choose a model in Settings to start';
  const typedCmd = parseCommand(request);
  const isAction = !!typedCmd && ACTION_NAMES.has(typedCmd.name);
  const canSend = !!request.trim() && !!convId && (isAction || !!busyTask || hasModel);
  const used = messages.reduce((n, m) => n + m.content.length, 0) + request.length;
  const budget = settings?.contextBudgetChars || 60_000;

  return (
    <>
      <div className="chat-head">
        <GeniusAvatar size={34} mood={mood} />
        <div className="grow" style={{ minWidth: 0 }}>
          <div className="chat-name">{name}</div>
          <div className="chat-status ellipsis" title={status}>
            {anyTask && <span className="dot run" />} {status}
          </div>
        </div>
        <select className="input sm conv-select" aria-label="Conversation" value={convId ?? ''} onChange={(e) => setConvId(e.target.value)} disabled={!!agentTask}>
          {conversations.map((c) => (
            <option key={c.id} value={c.id}>
              {c.title} ({c.messageCount})
            </option>
          ))}
        </select>
        <button className="icon-btn" aria-label="New conversation" title="New conversation (project history is kept) — /clear" onClick={() => void newConversation()} disabled={!!agentTask}>
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
              {suggestions(files.some((f) => f.path === 'src/App.tsx')).map((s) => (
                <button key={s} onClick={() => (hasModel ? void send(s) : setText(s))} disabled={!!busyTask}>
                  {s}
                </button>
              ))}
            </div>
            <div className="faint tiny">
              Tip: type <span className="mono">/</span> for commands or <span className="mono">@</span> to include a file.
            </div>
          </div>
        )}
        {messages.map((m, i) => (
          <Message
            key={m.id}
            m={m}
            name={name}
            isLast={i === messages.length - 1}
            busy={!!busyTask}
            onAnswer={(t) => void send(t)}
            actions={actions}
            prevUser={m.role === 'assistant' ? messages.slice(0, i).reverse().find((x) => x.role === 'user') : undefined}
          />
        ))}
        {chatTasks.map((t) => (
          <TaskCard key={t.id} task={t} expanded={variant === 'advanced'} name={name} />
        ))}
        {queued.map((q) => (
          <QueuedBubble key={q.id} q={q} onRemove={() => useWorkspace.getState().removeQueued([q.id])} />
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
        {showHelp && (
          <HelpPanel
            commands={commands}
            onClose={() => setShowHelp(false)}
            onPick={(c) => {
              setShowHelp(false);
              setText(`/${c.name}${c.args ? ' ' : ''}`);
            }}
          />
        )}
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
              <span key={p} className="chip pinned" title={`${p} — its current content is sent with this message`}>
                <AtSign size={11} /> {p}
                <button className="x" aria-label={`Unpin ${p}`} onClick={() => setPinned(pinned.filter((x) => x !== p))}>
                  <X size={10} />
                </button>
              </span>
            ))}
          </div>
        )}
        <div className="composer-box">
          {popup && (
            <SuggestPopup
              id="composer-suggest"
              title={popup === 'slash' ? 'Commands' : 'Include a project file'}
              items={popupItems}
              active={sel}
              onPick={pick}
              onHover={setPopupIndex}
              empty={popup === 'slash' ? 'No matching command — Enter sends it as text' : 'No matching file'}
            />
          )}
          <textarea
            ref={input}
            rows={1}
            placeholder={
              agentTask
                ? `${name} is working — type to add instructions; it reads them after its current step`
                : busyTask
                  ? `${name} is busy with “${busyTask.title.replace(/^Genius: /, '')}” — messages are sent when it's free`
                  : picked
                    ? 'What should change about the selected element?'
                    : `Message ${name}… (/ for commands, @ for files)`
            }
            value={request}
            onChange={(e) => {
              setRequest(e.target.value);
              setCaret(e.target.selectionStart ?? e.target.value.length);
              setPopupIndex(0);
            }}
            onSelect={(e) => setCaret(e.currentTarget.selectionStart ?? 0)}
            onKeyDown={onKeyDown}
            aria-label="Change request"
            aria-autocomplete="list"
            aria-controls={popup ? 'composer-suggest' : undefined}
            aria-activedescendant={popup && popupItems.length ? `composer-suggest-${sel}` : undefined}
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
            <EffortSelect value={effort} onChange={setEffort} />
            <AutopilotToggle on={autopilot} onChange={setAutopilot} />
            {settings?.internetAccess && (
              <span className="faint" title={`${name} can search the web and read pages (turn off in Settings → AI provider)`}>
                <Globe size={13} />
              </span>
            )}
            <span className="spacer" />
            {messages.length > 0 && <ContextMeter used={used} budget={budget} onCompact={() => void compact()} disabled={!!busyTask} />}
            {agentTask && (
              <Button size="sm" variant="danger" icon={<Square size={12} />} onClick={() => void api.cancelTask(agentTask.id).catch((e) => toast('bad', errorMessage(e)))}>
                Stop
              </Button>
            )}
            {(!agentTask || request.trim()) && (
              <button className="send-btn" aria-label={busyTask ? 'Queue message' : 'Send'} title={busyTask ? `Queue for ${name} (Enter)` : 'Send (Enter)'} disabled={!canSend || sending} onClick={() => void send()}>
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
