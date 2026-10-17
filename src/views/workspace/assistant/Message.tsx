import { useEffect, useState } from 'react';
import { Check, Copy, FileDiff, FileText, ImageIcon, Pencil, RotateCcw, Undo2, X } from 'lucide-react';
import { api, errorMessage } from '../../../lib/api';
import { clockTime } from '../../../lib/logic';
import { askOf, reportFooter, reportSummary, texts, type Ask } from '../../../lib/agent';
import type { AgentReport, Changeset, MessageRow } from '../../../lib/types';
import { useApp } from '../../../store/app';
import { useWorkspace } from '../../../store/workspace';
import { Badge, Button } from '../../../components/ui';
import { GeniusAvatar } from '../../../components/GeniusAvatar';
import { Markdown } from '../../../components/Markdown';
import { Thinking } from './Thinking';

const KIND_LABEL: Record<string, { label: string; tone: 'ok' | 'bad' | 'warn' | 'accent' | 'idle' }> = {
  answer: { label: 'Answer', tone: 'idle' },
  plan: { label: 'Plan / suggestion', tone: 'accent' },
  question: { label: 'Question', tone: 'accent' },
  proposal: { label: 'Proposed changes', tone: 'warn' },
  error: { label: 'Stopped with an error', tone: 'bad' },
};

export interface MessageActions {
  /** Rewind to before this user message and send `text` instead. Resolves false when cancelled. */
  onEdit: (m: MessageRow, text: string) => Promise<boolean>;
  onRewind: (m: MessageRow) => void;
  onRetry: (m: MessageRow) => void;
}

export function Message({ m, name, isLast, busy, onAnswer, actions, prevUser }: { m: MessageRow; name: string; isLast: boolean; busy: boolean; onAnswer: (text: string) => void; actions: MessageActions; prevUser?: MessageRow }) {
  const meta = (m.meta ?? {}) as Record<string, unknown>;
  const kind = KIND_LABEL[m.kind ?? ''];
  const atts = (meta.attachments as { id: string; name: string; kind: string }[] | undefined) ?? [];
  const report = meta.report as AgentReport | undefined;
  const review = meta.review as { findings?: string; blockers?: number; error?: string } | undefined;
  const ask = askOf(meta);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(m.content);
  const [saving, setSaving] = useState(false);
  const [copied, setCopied] = useState(false);

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
      <div className="msg-wrap user fade-in">
        {!editing && (
          <div className="msg-actions" role="toolbar" aria-label="Message actions">
            <button
              className="icon-btn"
              aria-label="Edit message"
              title={busy ? 'Wait until Genius finishes' : 'Edit and resend (undoes everything after this message)'}
              disabled={busy}
              onClick={() => {
                setDraft(m.content);
                setEditing(true);
              }}
            >
              <Pencil size={13} />
            </button>
            <button className="icon-btn" aria-label="Rewind to here" title={busy ? 'Wait until Genius finishes' : 'Rewind the conversation and files to before this message'} disabled={busy} onClick={() => actions.onRewind(m)}>
              <Undo2 size={13} />
            </button>
          </div>
        )}
        <div className={`msg user ${editing ? 'editing' : ''}`}>
          <div className="who">
            You · {clockTime(m.createdAt)} {meta.mode === 'plan' && <Badge tone="accent">plan only</Badge>}
          </div>
          {editing ? (
            <div className="msg-edit">
              <textarea
                className="input"
                aria-label="Edited message"
                value={draft}
                autoFocus
                rows={Math.min(8, Math.max(2, draft.split('\n').length))}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Escape') {
                    e.stopPropagation();
                    setEditing(false);
                  }
                  if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    void save();
                  }
                }}
              />
              <div className="row" style={{ gap: 6, justifyContent: 'flex-end' }}>
                <Button size="sm" variant="ghost" onClick={() => setEditing(false)} disabled={saving}>
                  Cancel
                </Button>
                <Button size="sm" variant="primary" loading={saving} disabled={!draft.trim() || busy} onClick={() => void save()}>
                  Save &amp; resend
                </Button>
              </div>
            </div>
          ) : (
            <Markdown text={m.content} />
          )}
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
      </div>
    );
  }

  const ms = prevUser ? Date.parse(m.createdAt) - Date.parse(prevUser.createdAt) : NaN;
  const footer = m.kind === 'error' ? [] : reportFooter(report, Number.isFinite(ms) && ms > 0 && ms < 3_600_000 ? ms : null);
  return (
    <div className="msg-row msg-wrap assistant fade-in">
      <GeniusAvatar size={26} mood={m.kind === 'error' ? 'sad' : 'idle'} className="msg-avatar" />
      <div className={`msg assistant ${m.kind === 'error' ? 'error' : ''}`}>
        <div className="msg-actions" role="toolbar" aria-label="Answer actions">
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
            {copied ? <Check size={13} /> : <Copy size={13} />}
          </button>
          {prevUser && (
            <button className="icon-btn" aria-label="Retry" title={busy ? 'Wait until Genius finishes' : 'Ask again (undoes this answer and its changes)'} disabled={busy} onClick={() => actions.onRetry(m)}>
              <RotateCcw size={13} />
            </button>
          )}
        </div>
        <div className="who row" style={{ gap: 6 }}>
          <span>
            {name} · {clockTime(m.createdAt)}
          </span>
          {kind && m.kind !== 'answer' && <Badge tone={kind.tone}>{kind.label}</Badge>}
        </div>
        {m.taskId && <Thinking taskId={m.taskId} live={false} />}
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
        {report && (report.steps > 0 || texts(report.warnings).length > 0 || !!report.error) && <Report report={report} />}
        {footer.length > 0 && (
          <div className="msg-foot" aria-label="Answer details">
            {footer.join(' · ')}
          </div>
        )}
      </div>
    </div>
  );
}

/** Whether an answer recorded file changes (so retrying it would undo them). */
export function changedFiles(m: MessageRow): boolean {
  const meta = (m.meta ?? {}) as Record<string, unknown>;
  const r = meta.report as AgentReport | undefined;
  return typeof meta.changesetId === 'string' || !!r?.changeset || texts(r?.checkpoint).length > 0 || texts(r?.autopilot?.applied).length > 0;
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

/**
 * Evidence-based summary of a finished run. Every row is built from recorded data and rows
 * without content are left out, so the list never shows empty bullets.
 */
export function Report({ report }: { report: AgentReport }) {
  const s = reportSummary(report);
  const read = texts(report.filesRead);
  const checks = (Array.isArray(report.checks) ? report.checks : []).filter((c) => c && typeof c.check === 'string' && c.check);
  const subagents = (Array.isArray(report.subagents) ? report.subagents : []).filter((x) => x && typeof x.role === 'string' && x.role);
  const failed = texts(report.failedSteps);
  const notVerified = [...new Set(texts(report.notVerified))];
  const warnings = [...new Set(texts(report.warnings))];
  const rows: { key: string; text: string; cls?: string }[] = [
    { key: 'read', text: `Files read: ${read.length ? read.join(', ') : 'none'}` },
    { key: 'search', text: `Searches: ${typeof report.searches === 'number' ? report.searches : 0}` },
    { key: 'checks', text: `Checks run: ${checks.length ? checks.map((c) => `${c.check}: ${c.status ?? (c.ok ? 'passed' : 'error')}${c.exitCode != null ? ` (exit ${c.exitCode})` : ''}`).join(', ') : 'none'}` },
  ];
  if (subagents.length) rows.push({ key: 'sub', text: `Subagents: ${subagents.map((x) => `${x.role}${x.ok ? '' : ' (failed)'}`).join(', ')}` });
  if (failed.length) rows.push({ key: 'failed', text: `Failed or refused steps: ${failed.join('; ')}` });
  notVerified.forEach((n) => rows.push({ key: `nv:${n}`, text: `Not verified: ${n}`, cls: 'warn-text' }));
  warnings.forEach((w) => rows.push({ key: `w:${w}`, text: w, cls: 'bad-text' }));
  if (typeof report.error === 'string' && report.error.trim()) rows.push({ key: 'error', text: `Error: ${report.error.trim()}`, cls: 'bad-text' });
  return (
    <details className="report">
      <summary>
        What Genius did · {s}
        {warnings.length > 0 && ' · ⚠'}
      </summary>
      <ul className="small">
        {rows.map((r) => (
          <li key={r.key} className={r.cls}>
            {r.text}
          </li>
        ))}
      </ul>
    </details>
  );
}
