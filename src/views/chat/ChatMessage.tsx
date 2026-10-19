import { useEffect, useState } from 'react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { AlertTriangle, Check, ChevronDown, Copy, FileText, Globe2, ImageIcon, Pencil, RotateCcw, Undo2 } from 'lucide-react';
import { api } from '../../lib/api';
import { answerDetails, webSourcesOf } from '../../lib/chat';
import { clockTime } from '../../lib/logic';
import { askOf, texts } from '../../lib/agent';
import type { AgentReport, MessageRow } from '../../lib/types';
import { Button, Modal } from '../../components/ui';
import { Markdown } from '../../components/Markdown';
import { GeniusAvatar } from '../../components/GeniusAvatar';
import type { MessageActions } from '../workspace/assistant/Message';

type Att = { id: string; name: string; kind: string };

/** An attached file in a message: images show the picture that was analysed (click to enlarge). */
export function AttachmentThumb({ att, workspaceId }: { att: Att; workspaceId: string | null }) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [big, setBig] = useState(false);
  const isImage = att.kind === 'image';
  useEffect(() => {
    if (!isImage || !workspaceId) return;
    let alive = true;
    api
      .attachmentPreview(workspaceId, att.id)
      .then((r) => alive && (r.imageUrl ? setUrl(r.imageUrl) : setFailed(true)))
      .catch(() => alive && setFailed(true));
    return () => {
      alive = false;
    };
  }, [att.id, isImage, workspaceId]);

  if (isImage && url) {
    return (
      <>
        <button className="gx-thumb" onClick={() => setBig(true)} aria-label={`Open image ${att.name}`} title={att.name}>
          <img src={url} alt={att.name} />
        </button>
        {big && (
          <Modal title={att.name} onClose={() => setBig(false)} wide>
            <img className="gx-lightbox" src={url} alt={att.name} />
          </Modal>
        )}
      </>
    );
  }
  return (
    <span className="chip gx-filechip" title={failed ? 'The picture is no longer stored (the file was deleted).' : att.name}>
      {isImage ? <ImageIcon size={12} /> : <FileText size={12} />} <span className="ellipsis">{att.name}</span>
    </span>
  );
}

function Sources({ report }: { report: AgentReport | undefined }) {
  const list = webSourcesOf(report as { webSources?: unknown } | undefined);
  if (!list.length) return null;
  return (
    <div className="gx-sources" aria-label="Web pages Genius read">
      <Globe2 size={13} aria-hidden />
      {list.map((s) => (
        <a
          key={s.url}
          href={s.url}
          title={s.url}
          onClick={(e) => {
            e.preventDefault();
            void openUrl(s.url).catch(() => window.open(s.url, '_blank', 'noopener'));
          }}
        >
          {s.host}
        </a>
      ))}
    </div>
  );
}

export function ChatMessage({ m, name, isLast, busy, workspaceId, actions, prevUser, onAnswer }: { m: MessageRow; name: string; isLast: boolean; busy: boolean; workspaceId: string | null; actions: MessageActions; prevUser?: MessageRow; onAnswer: (text: string) => void }) {
  const meta = (m.meta ?? {}) as Record<string, unknown>;
  const atts = (meta.attachments as Att[] | undefined) ?? [];
  const report = meta.report as AgentReport | undefined;
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.content);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);
  const [details, setDetails] = useState(false);

  if (m.role === 'user') {
    const save = async () => {
      if (!draft.trim()) return;
      setSaving(true);
      try {
        if (await actions.onEdit(m, draft)) setEditing(false);
      } finally {
        setSaving(false);
      }
    };
    return (
      <div className="gx-msg user fade-in">
        {atts.length > 0 && (
          <div className="gx-atts">
            {atts.map((a) => (
              <AttachmentThumb key={a.id} att={a} workspaceId={workspaceId} />
            ))}
          </div>
        )}
        {editing ? (
          <div className="gx-edit">
            <textarea
              className="input"
              aria-label="Edited message"
              value={draft}
              autoFocus
              rows={Math.min(8, Math.max(2, draft.split('\n').length))}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setEditing(false);
                if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                  e.preventDefault();
                  void save();
                }
              }}
            />
            <div className="row" style={{ justifyContent: 'flex-end' }}>
              <Button size="sm" variant="ghost" onClick={() => setEditing(false)} disabled={saving}>
                Cancel
              </Button>
              <Button size="sm" variant="primary" loading={saving} disabled={!draft.trim() || busy} onClick={() => void save()}>
                Save &amp; resend
              </Button>
            </div>
          </div>
        ) : (
          <div className="gx-bubble">
            <Markdown text={m.content} />
          </div>
        )}
        {!editing && (
          <div className="gx-actions" role="toolbar" aria-label="Message actions">
            <button className="icon-btn" aria-label="Edit message" title={busy ? 'Wait until Genius finishes' : 'Edit and resend (removes everything after this message)'} disabled={busy} onClick={() => { setDraft(m.content); setEditing(true); }}>
              <Pencil size={13} />
            </button>
            <button className="icon-btn" aria-label="Rewind to here" title={busy ? 'Wait until Genius finishes' : 'Remove this message and everything after it, and put the text back in the box'} disabled={busy} onClick={() => actions.onRewind(m)}>
              <Undo2 size={13} />
            </button>
          </div>
        )}
      </div>
    );
  }

  const isError = m.kind === 'error';
  const ask = askOf(meta);
  const rows = answerDetails(report as unknown as Record<string, unknown> | undefined);
  const warnings = [...new Set(texts(report?.warnings))];
  return (
    <div className={`gx-msg assistant fade-in ${isError ? 'error' : ''}`}>
      <GeniusAvatar size={24} mood={isError ? 'sad' : 'idle'} className="gx-avatar" />
      <div className="gx-body">
        <div className="gx-who">
          {name} <span className="faint tiny">{clockTime(m.createdAt)}</span>
        </div>
        {isError ? (
          <div className="gx-error" role="alert">
            <AlertTriangle size={15} aria-hidden />
            <div className="grow">
              <Markdown text={m.content} />
            </div>
          </div>
        ) : (
          <Markdown text={m.content} />
        )}
        {ask && isLast && !busy && (
          <div className="gx-ask">
            <div className="gx-ask-q">{ask.question}</div>
            <div className="suggestions">
              {ask.options.map((o) => (
                <button key={o} onClick={() => onAnswer(o)}>
                  {o}
                </button>
              ))}
            </div>
          </div>
        )}
        {!isError && <Sources report={report} />}
        {warnings.length > 0 && (
          <div className="callout warn small" role="note">
            {warnings.join(' ')}
          </div>
        )}
        <div className="gx-actions inline" role="toolbar" aria-label="Answer actions">
          <button
            className="icon-btn"
            aria-label="Copy answer"
            title="Copy"
            onClick={() => {
              void navigator.clipboard?.writeText(m.content);
              setCopied(true);
              setTimeout(() => setCopied(false), 1200);
            }}
          >
            {copied ? <Check size={14} /> : <Copy size={14} />}
          </button>
          {prevUser && (isLast || isError) && (
            <button className="icon-btn" aria-label={isError ? 'Retry' : 'Regenerate'} title={busy ? 'Wait until Genius finishes' : isError ? 'Try again' : 'Ask again and replace this answer'} disabled={busy} onClick={() => actions.onRetry(m)}>
              <RotateCcw size={14} />
            </button>
          )}
          {rows.length > 0 && (
            <button className="gx-details-btn" aria-expanded={details} onClick={() => setDetails(!details)}>
              Details <ChevronDown size={12} style={{ transform: details ? 'rotate(180deg)' : undefined }} />
            </button>
          )}
        </div>
        {details && (
          <dl className="kv gx-details" aria-label="Answer details">
            {rows.map((r) => (
              <div key={r.label} style={{ display: 'contents' }}>
                <dt>{r.label}</dt>
                <dd>{r.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </div>
    </div>
  );
}
