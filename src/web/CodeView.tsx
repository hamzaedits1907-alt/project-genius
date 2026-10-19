// Code workspace of Genius Web: a code assistant next to the code it wrote. A browser has no project
// folder, terminal or build, so this is honest about it: the assistant writes and explains code, the
// panel collects the files from the conversation (copy, download, preview a self-contained HTML page in a
// sandbox), and nothing is run or claimed to work.
import { useEffect, useMemo, useState } from 'react';
import { Check, Copy, Download, Eye, FileCode2, Plus } from 'lucide-react';
import { parseBlocks } from '../components/Markdown';
import { invoke } from '@tauri-apps/api/core';
import ChatView from '../views/chat/ChatView';
import { useChat } from '../store/chat';
import { useApp } from '../store/app';
import type { ChatConversation } from '../lib/types';

interface CodeFile {
  key: string;
  name: string;
  lang: string;
  body: string;
}

const EXT: Record<string, string> = { typescript: 'ts', ts: 'ts', tsx: 'tsx', javascript: 'js', js: 'js', jsx: 'jsx', python: 'py', py: 'py', html: 'html', css: 'css', json: 'json', rust: 'rs', rs: 'rs', go: 'go', java: 'java', sql: 'sql', bash: 'sh', sh: 'sh', yaml: 'yml', yml: 'yml', md: 'md', markdown: 'md', c: 'c', cpp: 'cpp', csharp: 'cs', php: 'php', ruby: 'rb', svg: 'svg' };

/** The files in a conversation: fenced blocks, named by a path comment on their first line when there is one. */
export function codeFilesOf(texts: string[]): CodeFile[] {
  const files = new Map<string, CodeFile>();
  texts.forEach((t, ti) =>
    parseBlocks(t).forEach((b, bi) => {
      if (b.kind !== 'code' || !b.body.trim()) return;
      const lang = (b.lang || '').toLowerCase();
      const first = b.body.split('\n', 1)[0];
      const named = /^\s*(?:\/\/|#|<!--|\/\*|--)\s*([\w./-]+\.\w{1,8})\s*(?:-->|\*\/)?\s*$/.exec(first);
      const name = named?.[1] ?? `snippet-${ti + 1}-${bi + 1}.${EXT[lang] ?? 'txt'}`;
      files.set(name, { key: name, name, lang, body: b.body });
    }),
  );
  return [...files.values()];
}

function Preview({ html }: { html: string }) {
  // scripts may run (the user asked for a page), but with no access to this app, its storage or its account
  return <iframe title="Preview" className="cv-preview" sandbox="allow-scripts" referrerPolicy="no-referrer" srcDoc={html} />;
}

export default function CodeView() {
  const messages = useChat((s) => s.messages);
  const navigate = useApp((s) => s.navigate);
  const [pick, setPick] = useState<string | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  const files = useMemo(() => codeFilesOf(messages.filter((m) => m.role === 'assistant' && m.kind !== 'error').map((m) => m.content)), [messages]);
  const open = files.find((f) => f.name === pick) ?? files[files.length - 1] ?? null;
  const isHtml = open && (open.lang === 'html' || /\.html?$/.test(open.name));

  const newSession = async () => {
    const c = (await invoke('web_create_code_session')) as ChatConversation;
    useChat.setState((s) => ({ conversations: { ...s.conversations, chat: [c, ...s.conversations.chat] }, all: [c, ...s.all], active: { ...s.active, chat: c.id }, messages: [], sources: [] }));
    setPick(null);
  };
  useEffect(() => {
    // open the most recent code session, or start one
    void (async () => {
      await useChat.getState().load('chat');
      const st = useChat.getState();
      const cur = st.all.find((c) => c.id === st.active.chat && c.origin?.flavor === 'code') ?? st.all.find((c) => c.origin?.flavor === 'code');
      if (cur) await st.select('chat', cur.id);
      else await newSession();
      setReady(true);
    })();
  }, []);

  const download = (f: CodeFile) => {
    const url = URL.createObjectURL(new Blob([f.body], { type: 'text/plain;charset=utf-8' }));
    const a = Object.assign(document.createElement('a'), { href: url, download: f.name.split('/').pop() });
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 2000);
  };

  return (
    <div className="cv-root">
      <div className="cv-chat">{ready ? <ChatView mode="chat" /> : null}</div>
      <aside className="cv-side" aria-label="Code from this conversation">
        <div className="row" style={{ padding: '12px 14px' }}>
          <FileCode2 size={15} />
          <b className="grow">Code</b>
          <button className="btn sm" onClick={() => void newSession()}><Plus size={12} /> New session</button>
        </div>
        <p className="small faint" style={{ margin: '0 14px 10px' }}>Genius Web writes and explains code here. It cannot run, build or test it: for that, open your project in Genius Desktop.</p>
        {files.length === 0 ? (
          <p className="small muted" style={{ padding: '0 14px' }}>Files from the conversation appear here. Ask for a small web page, a script or a function.</p>
        ) : (
          <>
            <ul className="cv-files" role="listbox" aria-label="Files">
              {files.map((f) => (
                <li key={f.key}>
                  <button role="option" aria-selected={open?.name === f.name} className={open?.name === f.name ? 'on' : ''} onClick={() => setPick(f.name)}>
                    <span className="ellipsis">{f.name}</span>
                    <span className="tiny faint">{f.body.split('\n').length} lines</span>
                  </button>
                </li>
              ))}
            </ul>
            {open && (
              <div className="cv-view">
                <div className="row wrap" style={{ padding: '0 14px' }}>
                  <button className="btn sm" onClick={() => { void navigator.clipboard?.writeText(open.body); setCopied(open.name); setTimeout(() => setCopied(null), 1200); }}>
                    {copied === open.name ? <Check size={12} /> : <Copy size={12} />} Copy
                  </button>
                  <button className="btn sm" onClick={() => download(open)}><Download size={12} /> Download</button>
                  {isHtml && <span className="badge accent" title="The app's security policy blocks scripts inside previews, so only the page's HTML and CSS are shown."><Eye size={11} /> HTML and CSS preview</span>}
                </div>
                {isHtml ? <Preview html={open.body} /> : <pre className="cv-code">{open.body}</pre>}
              </div>
            )}
          </>
        )}
        <button className="btn sm ghost" style={{ margin: '12px 14px' }} onClick={() => navigate({ name: 'projects' })}>About projects</button>
      </aside>
    </div>
  );
}
