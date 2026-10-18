import { useEffect, useRef, useState } from 'react';
import { ArrowUp, FileText, Paperclip, Pencil, Plus, Rocket, Search, Square, Trash2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { fileToBase64, stepLabel } from '../../lib/agent';
import { bytes } from '../../lib/logic';
import type { Attachment, ChatMode, CitationValidation, MessageRow } from '../../lib/types';
import { useApp } from '../../store/app';
import { useChat } from '../../store/chat';
import { Button } from '../../components/ui';
import { Markdown } from '../../components/Markdown';
import { Message, type MessageActions } from '../workspace/assistant/Message';
import { ModeSwitch } from './ModeSwitch';
import { SourcesPanel } from './SourcesPanel';

const HELLO: Record<ChatMode, { title: string; text: string; ideas: string[] }> = {
  chat: { title: 'Chat with Genius', text: 'Ask anything, think a problem through, or attach a file, log or image to analyse. Nothing here changes your projects.', ideas: ['Explain the difference between REST and GraphQL', 'Help me plan a small web app', 'Summarise the file I attach'] },
  research: { title: 'Genius Research', text: 'Genius searches the web, reads the actual pages, compares sources and writes a report where every claim cites a page it really read.', ideas: ['How do React Server Components work?', 'Compare Vite and webpack for a small app', 'What changed in the latest TypeScript release?'] },
};

/** Chat or Research: conversation list with search, the thread, the composer (and sources for Research). */
export default function ChatView({ mode }: { mode: ChatMode }) {
  const st = useChat();
  const settings = useApp((s) => s.settings);
  const toast = useApp((s) => s.toast);
  const navigate = useApp((s) => s.navigate);
  const [text, setText] = useState('');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [title, setTitle] = useState('');
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const bottom = useRef<HTMLDivElement>(null);
  const convs = st.conversations[mode];
  const activeId = st.active[mode];
  const conv = convs.find((c) => c.id === activeId);
  const task = st.tasks.find((t) => t.kind === mode);
  const busy = st.tasks.some((t) => t.kind === 'chat' || t.kind === 'research' || t.kind === 'spec');
  const advanced = settings?.uiMode === 'advanced';
  const internetOff = mode === 'research' && settings?.internetAccess === false;
  const hasModel = !!settings?.model || settings?.modelMode === 'auto';

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

  useEffect(() => {
    bottom.current?.scrollIntoView?.({ block: 'end' });
  }, [st.messages.length, st.tasks.length]);

  const send = async () => {
    const t = text;
    if (!t.trim()) return;
    setText('');
    const a = attachments;
    setAttachments([]);
    await st.send(mode, t, a);
  };

  const addFiles = async (files: File[]) => {
    let id = st.active[mode];
    if (!id) id = (await st.create(mode))?.id ?? null;
    if (!id || !st.workspaceId) return;
    const vision = await api.resolvedModels().then((r) => r.vision.length > 0).catch(() => false);
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
      if (prev) void rewind(prev).then(async (ok) => { if (ok) await st.send(mode, prev.content); });
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

  const hello = HELLO[mode];
  const steps = task ? (st.steps[task.id] ?? []) : [];
  const lastAssistant = [...st.messages].reverse().find((m) => m.role === 'assistant' && m.kind === 'answer');
  const queued = st.queue;

  return (
    <div className="chat-mode" data-mode={mode}>
      <aside className="chat-list" aria-label="Conversations">
        <div className="row" style={{ gap: 6 }}>
          <label className="chat-search grow">
            <Search size={13} />
            <input className="input sm" placeholder="Search conversations" aria-label="Search conversations" value={st.query} onChange={(e) => void st.search(mode, e.target.value)} />
          </label>
          <button className="icon-btn" aria-label="New conversation" title="New conversation" onClick={() => void st.create(mode)}>
            <Plus size={15} />
          </button>
        </div>
        <ul>
          {convs.map((c) => (
            <li key={c.id} className={c.id === activeId ? 'on' : ''}>
              {renaming === c.id ? (
                <input
                  className="input sm"
                  aria-label="Conversation title"
                  autoFocus
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  onBlur={() => setRenaming(null)}
                  onKeyDown={(e) => {
                    if (e.key === 'Escape') setRenaming(null);
                    if (e.key === 'Enter' && title.trim()) {
                      void st.rename(mode, c.id, title).then(() => setRenaming(null));
                    }
                  }}
                />
              ) : (
                <button className="chat-item" onClick={() => void st.select(mode, c.id)} disabled={busy}>
                  <span className="ellipsis">{c.title}</span>
                  <span className="faint tiny ellipsis">{c.snippet}</span>
                </button>
              )}
              <button className="icon-btn" aria-label={`Rename ${c.title}`} onClick={() => { setTitle(c.title); setRenaming(c.id); }}>
                <Pencil size={12} />
              </button>
              <button
                className="icon-btn"
                aria-label={`Delete ${c.title}`}
                disabled={busy}
                onClick={() => {
                  if (window.confirm(`Delete “${c.title}”? Its messages${mode === 'research' ? ' and sources ledger' : ''} are removed.`)) void st.remove(mode, c.id);
                }}
              >
                <Trash2 size={12} />
              </button>
            </li>
          ))}
          {convs.length === 0 && <li className="faint small" style={{ padding: 8 }}>{st.query ? 'No conversation matches.' : 'No conversations yet.'}</li>}
        </ul>
      </aside>

      <section className="chat-main">
        <div className="chat-top">
          <ModeSwitch mode={mode} />
          <span className="grow" />
          {conv?.origin?.projectId && (
            <Button size="sm" variant="ghost" onClick={() => navigate({ name: 'project', id: conv.origin!.projectId! })}>
              Open project “{conv.origin.projectName}”
            </Button>
          )}
          {mode === 'research' && (
            <Button size="sm" disabled={!lastAssistant || busy || busyAction === 'spec'} loading={busyAction === 'spec'} onClick={() => void spec()} title="Send the report to the planner for a project spec and implementation plan">
              Turn research into a spec
            </Button>
          )}
          <Button size="sm" icon={<Rocket size={13} />} disabled={!st.messages.some((m) => m.role === 'user') || busy || busyAction === 'project'} loading={busyAction === 'project'} onClick={() => void startProject()} title="Genius summarises the requirements, creates a code project and opens it in Code mode with this conversation as context">
            Start a project from this conversation
          </Button>
        </div>
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
        <div className="chat-body">
          <div className="scroll grow chat" aria-live="polite">
            {st.messages.length === 0 && !task && (
              <div className="chat-hello fade-in">
                <h3>{hello.title}</h3>
                <p>{hello.text}</p>
                <div className="suggestions">
                  {hello.ideas.map((i) => (
                    <button key={i} disabled={!hasModel || internetOff} onClick={() => void st.send(mode, i)}>
                      {i}
                    </button>
                  ))}
                </div>
                {!hasModel && <p className="faint small">Choose a model in Settings → AI provider to start.</p>}
              </div>
            )}
            {st.messages.map((m, i) => {
              const v = m.meta?.validation as CitationValidation | undefined;
              return (
                <div key={m.id}>
                  {m.kind === 'plan' && m.meta?.spec ? (
                    <div className="msg-row msg-wrap assistant">
                      <div className="msg assistant">
                        <div className="who">Project spec</div>
                        <Markdown text={m.content} />
                        <Button size="sm" variant="primary" icon={<Rocket size={13} />} disabled={busy || busyAction === 'project'} onClick={() => void startProject(m.id)}>
                          Start a project from this spec
                        </Button>
                      </div>
                    </div>
                  ) : (
                    <Message m={m} name="Genius" isLast={i === st.messages.length - 1} busy={busy} onAnswer={(t) => void st.send(mode, t)} actions={actions} prevUser={m.role === 'assistant' ? st.messages.slice(0, i).reverse().find((x) => x.role === 'user') : undefined} />
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
            {task && (
              <div className="msg-row fade-in">
                <div className="task-card live" aria-label={`Running: ${task.title}`}>
                  <div className="row">
                    <strong className="grow ellipsis">{task.title.replace(/^(Chat|Research): /, mode === 'research' ? 'Researching: ' : 'Thinking about: ')}</strong>
                    <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))}>
                      Stop
                    </Button>
                  </div>
                  {steps.length === 0 && <div className="small faint thinking-dots">Working</div>}
                  <ul className="small">
                    {steps.slice(advanced ? -12 : -4).map((s) => (
                      <li key={s.id} className={s.ok ? '' : 'bad-text'}>
                        {stepLabel(s)}
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            )}
            {queued.map((q) => (
              <div key={q.id} className="msg-wrap user">
                <div className="msg user queued">
                  <div>{q.text}</div>
                  <div className="faint tiny">{q.status === 'delivered' ? 'Delivered — Genius read this' : q.status === 'undelivered' ? 'Not delivered' : 'Queued — Genius will read this after its current step'}</div>
                </div>
              </div>
            ))}
            <div ref={bottom} />
          </div>
          {mode === 'research' && <SourcesPanel sources={st.sources} />}
        </div>
        <div
          className="composer"
          onDragOver={(e) => e.preventDefault()}
          onDrop={(e) => {
            e.preventDefault();
            void addFiles(Array.from(e.dataTransfer.files));
          }}
        >
          {attachments.length > 0 && (
            <div className="chips" aria-label="Attached to this message">
              {attachments.map((a) => (
                <span key={a.id} className="chip">
                  <FileText size={11} /> {a.name} · {bytes(a.size)}
                  <button className="x" aria-label={`Remove ${a.name}`} onClick={() => setAttachments(attachments.filter((x) => x.id !== a.id))}>
                    <X size={10} />
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="composer-box">
            <textarea
              rows={2}
              aria-label={mode === 'research' ? 'Research question' : 'Message'}
              placeholder={task ? 'Genius is working — type to add instructions; it reads them after its current step' : mode === 'research' ? 'Ask a research question…' : 'Message Genius…'}
              value={text}
              disabled={internetOff}
              onChange={(e) => setText(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void send();
                }
              }}
            />
            <div className="row composer-tools">
              <input ref={fileInput} type="file" multiple hidden aria-label="Choose files" onChange={(e) => { void addFiles(Array.from(e.target.files ?? [])); e.target.value = ''; }} />
              <button className="icon-btn" aria-label="Attach files" title="Attach text, code, CSV, JSON, PDF, DOCX, ZIP files (images only when a vision model is available)" onClick={() => fileInput.current?.click()}>
                <Paperclip size={15} />
              </button>
              <span className="spacer" />
              <button className="send-btn" aria-label={busy ? 'Queue message' : 'Send'} disabled={!text.trim() || !hasModel || internetOff} onClick={() => void send()}>
                <ArrowUp size={16} />
              </button>
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}
