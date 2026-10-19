import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowUp, Code2, FileText, Globe2, ImageIcon, MoreHorizontal, Paperclip, Rocket, Square, Trash2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { fileToBase64, stepLabel } from '../../lib/agent';
import { bytes } from '../../lib/logic';
import type { AgentStep, Attachment, ChatMode, CitationValidation, MessageRow, TaskInfo } from '../../lib/types';
import { useApp } from '../../store/app';
import { useChat } from '../../store/chat';
import { Button } from '../../components/ui';
import { GeniusAvatar } from '../../components/GeniusAvatar';
import { Markdown } from '../../components/Markdown';
import { Menu } from '../../components/Menu';
import { type MessageActions } from '../workspace/assistant/Message';
import { QuickSetup } from '../Home';
import { AttachmentThumb, ChatMessage } from './ChatMessage';
import { SourcesPanel } from './SourcesPanel';

const HELLO: Record<ChatMode, { title: string; text: string }> = {
  chat: { title: 'How can I help you today?', text: 'Ask anything, think a problem through, or attach a file, screenshot or document to analyse.' },
  research: { title: 'What should I research?', text: 'I search the web, read the actual pages, compare sources and write a report where every claim cites a page I really read.' },
};

function greeting(): string {
  const h = new Date().getHours();
  return h < 5 ? 'Up late?' : h < 12 ? 'Good morning' : h < 18 ? 'Good afternoon' : 'Good evening';
}

/** Chat or Research: the thread, the activity of a running answer, and the composer. The conversation list lives in the sidebar. */
export default function ChatView({ mode }: { mode: ChatMode }) {
  const st = useChat();
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const toast = useApp((s) => s.toast);
  const navigate = useApp((s) => s.navigate);
  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const box = useRef<HTMLTextAreaElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const activeId = st.active[mode];
  const conv = st.conversations[mode].find((c) => c.id === activeId) ?? st.all.find((c) => c.id === activeId);
  const task = st.tasks.find((t) => t.kind === mode);
  const busy = st.tasks.some((t) => t.kind === 'chat' || t.kind === 'research' || t.kind === 'spec');
  const internetOff = mode === 'research' && settings?.internetAccess === false;
  const connected = !!keyStatus?.stored;
  const hasModel = connected && (!!settings?.model || settings?.modelMode === 'auto');
  const empty = st.messages.length === 0 && !task;

  useEffect(() => {
    void useChat.getState().load(mode).then(() => {
      const p = useChat.getState().pending;
      if (p && p.mode === mode) {
        useChat.getState().setPending(null);
        void (async () => {
          await useChat.getState().create(mode);
          await useChat.getState().send(mode, p.text);
        })();
      }
    });
  }, [mode]);

  const stepCount = task ? (st.steps[task.id]?.length ?? 0) : 0;
  useEffect(() => {
    bottom.current?.scrollIntoView?.({ block: 'end' });
  }, [st.messages.length, st.tasks.length, stepCount]);

  // The box grows with its text (up to a limit) and keeps focus after a send.
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 220)}px`;
  }, [text]);

  const send = async () => {
    const t = text;
    if (!t.trim() || !hasModel || internetOff) return;
    setText('');
    const a = attachments;
    setAttachments([]);
    await st.send(mode, t, a);
    box.current?.focus();
  };

  const addFiles = async (files: File[]) => {
    if (!files.length) return;
    let id = st.active[mode];
    if (!id) id = (await st.create(mode))?.id ?? null;
    if (!id || !st.workspaceId) return;
    const vision = files.some((f) => /\.(png|jpe?g|gif|webp)$/i.test(f.name)) ? await api.resolvedModels().then((r) => r.vision.length > 0).catch(() => false) : true;
    for (const f of files) {
      if (/\.(png|jpe?g|gif|webp)$/i.test(f.name) && !vision) {
        toast('bad', `${f.name}: none of your available models can read images, so it can't be analysed. Refresh the model list in Settings → AI provider.`);
        continue;
      }
      try {
        const a = await api.importAttachmentData(st.workspaceId, f.name, await fileToBase64(f), 'reference', id);
        if (a.status === 'failed') toast('bad', `${a.name}: ${a.error}`);
        else setAttachments((cur) => [...cur, a]);
      } catch (e) {
        toast('bad', `${f.name}: ${errorMessage(e)}`);
      }
    }
  };

  const rewind = async (m: MessageRow) => {
    if (!activeId) return false;
    try {
      await api.rewindConversation(activeId, m.id, false);
      await st.refresh(mode);
      return true;
    } catch (e) {
      toast('bad', errorMessage(e));
      return false;
    }
  };
  const actions: MessageActions = {
    onEdit: async (m, t) => {
      if (busy || !(await rewind(m))) return false;
      await st.send(mode, t);
      return true;
    },
    onRewind: (m) => {
      if (busy) return;
      void rewind(m).then((ok) => ok && setText(m.content));
    },
    onRetry: (m) => {
      if (busy) return;
      const i = st.messages.findIndex((x) => x.id === m.id);
      const prev = st.messages.slice(0, i).reverse().find((x) => x.role === 'user');
      if (!prev) return;
      const files = ((prev.meta?.attachments as { id: string }[] | undefined) ?? []).map((a) => a.id);
      void rewind(prev).then(async (ok) => {
        if (ok) await st.send(mode, prev.content, files.map((id) => ({ id, status: 'ready' }) as Attachment));
      });
    },
  };

  const startProject = async (sourceMessageId?: string) => {
    if (!activeId) return;
    setBusyAction('project');
    try {
      const p = await api.projectFromConversation(activeId, undefined, sourceMessageId);
      await api.startPlan(p.id, p.description, '').catch((e) => toast('info', `Project created; planning did not start: ${errorMessage(e)}`));
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusyAction(null);
    }
  };

  const spec = async () => {
    if (!activeId) return;
    setBusyAction('spec');
    try {
      await api.startResearchSpec(activeId);
      await st.refresh(mode);
      const tasks = await api.activeTasks(st.workspaceId ?? undefined);
      useChat.setState({ tasks });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusyAction(null);
    }
  };

  /** Switch between Chat and Research, taking along what was typed. */
  const switchMode = () => {
    const other: ChatMode = mode === 'chat' ? 'research' : 'chat';
    if (text.trim()) {
      useChat.getState().setPending({ mode: other, text: text.trim() });
      setText('');
    } else {
      useChat.getState().fresh(other);
    }
    navigate({ name: other });
  };

  const toggleWeb = async () => {
    if (!settings) return;
    try {
      useApp.getState().setSettings(await api.saveSettings({ ...settings, internetAccess: !settings.internetAccess }));
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const hello = HELLO[mode];
  const lastUserIndex = st.messages.map((m) => m.role).lastIndexOf('user');
  const hasUser = lastUserIndex >= 0;
  const ideas: { icon: ReactNode; title: string; hint: string; run: () => void }[] =
    mode === 'chat'
      ? [
          { icon: <ArrowUp size={16} />, title: 'Explain something', hint: 'REST versus GraphQL, in plain words', run: () => void st.send(mode, 'Explain the difference between REST and GraphQL in plain words, with a small example of each.') },
          { icon: <ImageIcon size={16} />, title: 'Analyse an image or file', hint: 'Screenshots, PDFs, CSV, code, logs', run: () => fileInput.current?.click() },
          { icon: <Globe2 size={16} />, title: 'Research a topic', hint: 'Sources you can check', run: () => { useChat.getState().fresh('research'); navigate({ name: 'research' }); } },
          { icon: <Code2 size={16} />, title: 'Build something', hint: 'Open the Code workspace', run: () => navigate({ name: 'home' }) },
        ]
      : [
          { icon: <Globe2 size={16} />, title: 'How do React Server Components work?', hint: 'Official docs first', run: () => void st.send(mode, 'How do React Server Components work?') },
          { icon: <Globe2 size={16} />, title: 'Compare Vite and webpack', hint: 'For a small app', run: () => void st.send(mode, 'Compare Vite and webpack for a small app.') },
          { icon: <Globe2 size={16} />, title: 'What changed in the latest TypeScript?', hint: 'Release notes', run: () => void st.send(mode, 'What changed in the latest TypeScript release?') },
        ];

  const composer = (
    <div
      className={`gx-composer ${dragging ? 'dropping' : ''}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        e.preventDefault();
        setDragging(false);
        void addFiles(Array.from(e.dataTransfer.files));
      }}
    >
      {attachments.length > 0 && (
        <div className="gx-atts" aria-label="Attached to this message">
          {attachments.map((a) => (
            <span key={a.id} className="gx-att">
              {a.kind === 'image' ? <AttachmentThumb att={a} workspaceId={st.workspaceId} /> : (
                <span className="chip gx-filechip">
                  <FileText size={12} /> <span className="ellipsis">{a.name}</span> <span className="faint tiny">{bytes(a.size)}</span>
                </span>
              )}
              <button className="gx-att-x" aria-label={`Remove ${a.name}`} onClick={() => setAttachments(attachments.filter((x) => x.id !== a.id))}>
                <X size={11} />
              </button>
            </span>
          ))}
        </div>
      )}
      <textarea
        ref={box}
        rows={1}
        aria-label={mode === 'research' ? 'Research question' : 'Message'}
        placeholder={task ? 'Genius is working — type to add instructions; it reads them after its current step' : mode === 'research' ? 'Ask a research question…' : 'Message Genius…'}
        value={text}
        disabled={internetOff}
        onChange={(e) => setText(e.target.value)}
        onPaste={(e) => {
          const files = Array.from(e.clipboardData?.files ?? []);
          if (files.length) {
            e.preventDefault();
            void addFiles(files);
          }
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <div className="gx-tools">
        <input ref={fileInput} type="file" multiple hidden aria-label="Choose files" onChange={(e) => { void addFiles(Array.from(e.target.files ?? [])); e.target.value = ''; }} />
        <button className="icon-btn" aria-label="Attach files" title="Attach images, PDF, DOCX, CSV, JSON, code, text or ZIP files (images need a vision-capable model)" onClick={() => fileInput.current?.click()}>
          <Paperclip size={16} />
        </button>
        <button className={`gx-pill ${settings?.internetAccess ? 'on' : ''}`} aria-pressed={!!settings?.internetAccess} onClick={() => void toggleWeb()} title={settings?.internetAccess ? 'Genius may search the web and read pages when that helps. Click to turn it off.' : 'Web access is off. Click to let Genius search the web and read pages.'}>
          <Globe2 size={13} /> Web {settings?.internetAccess ? 'on' : 'off'}
        </button>
        <button className={`gx-pill ${mode === 'research' ? 'on' : ''}`} aria-pressed={mode === 'research'} onClick={switchMode} title={mode === 'research' ? 'Researching with cited sources. Click to go back to everyday chat.' : 'Switch to Research: plan queries, read sources, write a cited report'}>
          Research
        </button>
        <span className="spacer" />
        {task && !text.trim() ? (
          <button className="send-btn stop" aria-label="Stop" title="Stop" onClick={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))}>
            <Square size={13} fill="currentColor" />
          </button>
        ) : (
          <button className="send-btn" aria-label={busy ? 'Queue message' : 'Send'} title={busy ? 'Genius reads this after its current step' : 'Send (Enter)'} disabled={!text.trim() || !hasModel || internetOff} onClick={() => void send()}>
            <ArrowUp size={16} />
          </button>
        )}
      </div>
    </div>
  );

  return (
    <div className="gx-chat" data-mode={mode}>
      <header className="gx-chat-head">
        <h2 className="ellipsis">{conv?.title && conv.title !== 'New conversation' ? conv.title : mode === 'research' ? 'New research' : 'New chat'}</h2>
        {mode === 'research' && <span className="badge accent">Research</span>}
        <span className="spacer" />
        {conv?.origin?.projectId && (
          <Button size="sm" variant="ghost" onClick={() => navigate({ name: 'project', id: conv.origin!.projectId! })}>
            Open project “{conv.origin.projectName}”
          </Button>
        )}
        <Menu
          label="Conversation actions"
          trigger={<MoreHorizontal size={17} />}
          items={[
            { label: 'Start a project from this conversation', icon: <Rocket size={14} />, disabled: !hasUser || busy || busyAction === 'project', hint: 'Talk about what you want to build first', onSelect: () => void startProject() },
            ...(mode === 'research' ? [{ label: 'Turn research into a spec', icon: <FileText size={14} />, disabled: !st.messages.some((m) => m.role === 'assistant' && m.kind === 'answer') || busy || busyAction === 'spec', hint: 'Ask a research question first', onSelect: () => void spec() }] : []),
            {
              label: 'Delete conversation',
              icon: <Trash2 size={14} />,
              danger: true,
              disabled: !activeId || busy,
              hint: 'Nothing to delete yet',
              onSelect: () => {
                if (activeId && window.confirm(`Delete “${conv?.title ?? 'this conversation'}”? Its messages${mode === 'research' ? ' and sources ledger' : ''} are removed.`)) void st.remove(mode, activeId);
              },
            },
          ]}
        />
      </header>

      {internetOff && (
        <div className="callout warn" role="alert">
          <div className="grow">
            <b>Internet access is turned off.</b> Research needs it to search the web and read pages.
          </div>
          <Button size="sm" variant="primary" onClick={() => settings && void api.saveSettings({ ...settings, internetAccess: true }).then((s) => useApp.getState().setSettings(s))}>
            Turn on internet access
          </Button>
        </div>
      )}

      <div className="gx-chat-body">
        <div className="gx-thread scroll" aria-live="polite">
          {empty ? (
            <div className="gx-welcome fade-in">
              <GeniusAvatar size={68} mood="idle" />
              <p className="gx-greet">{greeting()}</p>
              <h1>{hello.title}</h1>
              <p className="lead">{hello.text}</p>
              {!connected && <QuickSetup />}
              {connected && !hasModel && <p className="faint small">Choose a model in Settings → AI provider to start.</p>}
              <div className="gx-welcome-composer">{composer}</div>
              <div className="gx-ideas">
                {ideas.map((i) => (
                  <button key={i.title} className="gx-idea" disabled={!hasModel || internetOff} onClick={i.run}>
                    <span className="gx-idea-ic">{i.icon}</span>
                    <span className="gx-idea-t">{i.title}</span>
                    <span className="gx-idea-h">{i.hint}</span>
                  </button>
                ))}
              </div>
            </div>
          ) : (
            <div className="gx-col">
              {st.messages.map((m, i) => {
                const v = m.meta?.validation as CitationValidation | undefined;
                return (
                  <div key={m.id}>
                    {m.kind === 'plan' && m.meta?.spec ? (
                      <div className="gx-msg assistant">
                        <GeniusAvatar size={24} className="gx-avatar" />
                        <div className="gx-body">
                          <div className="gx-who">Project spec</div>
                          <Markdown text={m.content} />
                          <Button size="sm" variant="primary" icon={<Rocket size={13} />} disabled={busy || busyAction === 'project'} onClick={() => void startProject(m.id)}>
                            Start a project from this spec
                          </Button>
                        </div>
                      </div>
                    ) : (
                      <ChatMessage m={m} name={settings?.assistantName || 'Genius'} isLast={i === st.messages.length - 1} busy={busy} workspaceId={st.workspaceId} onAnswer={(t) => void st.send(mode, t)} actions={actions} prevUser={m.role === 'assistant' ? st.messages.slice(0, i).reverse().find((x) => x.role === 'user') : undefined} />
                    )}
                    {v && !v.ok && (
                      <div className="callout warn small" role="note" aria-label="Citation check">
                        Citation check: {v.uncited.length} claim(s) without a source, {v.unknownCitations.length} citation(s) to sources that were never fetched, {v.unfetchedUrls.length} URL(s) never fetched.
                      </div>
                    )}
                    {v?.ok && (
                      <div className="faint tiny" aria-label="Citation check">
                        Citation check passed: {v.facts} cited claim(s), {v.assumptions} assumption(s), {v.uncertain} uncertain.
                      </div>
                    )}
                  </div>
                );
              })}
              {task && <Activity task={task} mode={mode} steps={st.steps[task.id] ?? []} onStop={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))} />}
              {st.queue.map((q) => (
                <div key={q.id} className="gx-msg user">
                  <div className="gx-bubble queued">
                    {q.text}
                    <div className="faint tiny">{q.status === 'delivered' ? 'Delivered — Genius read this' : q.status === 'undelivered' ? 'Not delivered' : 'Queued — Genius will read this after its current step'}</div>
                  </div>
                </div>
              ))}
              <div ref={bottom} />
            </div>
          )}
        </div>
        {mode === 'research' && !empty && <SourcesPanel sources={st.sources} />}
      </div>

      {!empty && (
        <div className="gx-dock">
          {composer}
          <div className="gx-fine faint tiny">Genius can make mistakes. Check important information.</div>
        </div>
      )}
    </div>
  );
}

/** What a running answer is doing: the tool steps so far, how long it has been, and Stop. */
function Activity({ task, mode, steps, onStop }: { task: TaskInfo; mode: ChatMode; steps: AgentStep[]; onStop: () => void }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  const secs = Math.max(0, Math.round((now - Date.parse(task.startedAt)) / 1000));
  return (
    <div className="gx-msg assistant fade-in">
      <GeniusAvatar size={24} mood="thinking" className="gx-avatar" />
      <div className="gx-body">
        <div className="gx-activity" role="status" aria-label={`Running: ${task.title}`}>
          <div className="row">
            <strong className="grow ellipsis">{task.title.replace(/^(Chat|Research): /, mode === 'research' ? 'Researching: ' : 'Thinking about: ')}</strong>
            <span className="faint tiny">{Number.isFinite(secs) ? `${secs}s` : ''}</span>
            <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={onStop}>
              Stop
            </Button>
          </div>
          {steps.length === 0 && <div className="small faint thinking-dots">Working</div>}
          <ul className="small">
            {steps.slice(-5).map((s) => (
              <li key={s.id} className={s.ok ? '' : 'bad-text'}>
                {stepLabel(s)}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
