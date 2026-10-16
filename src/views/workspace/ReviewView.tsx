import { useEffect, useState } from 'react';
import { Check, RotateCcw, Undo2, X } from 'lucide-react';
import { api, errorKind, errorMessage } from '../../lib/api';
import { timeAgo } from '../../lib/logic';
import type { Changeset, FileChange } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import CodeEditor from '../../components/CodeEditor';
import { Badge, Button, Callout, Empty, Segmented, Spinner, StatusBadge } from '../../components/ui';

/** Review a changeset: exact per-file diffs, then apply/reject (proposed) or revert/restore (applied). */
export default function ReviewView({ changesetId }: { changesetId: string }) {
  const toast = useApp((s) => s.toast);
  const openTab = useWorkspace((s) => s.openTab);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const [data, setData] = useState<{ changeset: Changeset; files: FileChange[] } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [sel, setSel] = useState(0);
  const [mode, setMode] = useState<'diff' | 'full'>('diff');
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    api
      .getChangeset(changesetId)
      .then((d) => setData(d))
      .catch((e) => setErr(errorMessage(e)));
  }, [changesetId, historyVersion]);

  if (err) return <Empty title="Could not load changes">{err}</Empty>;
  if (!data) return <Empty title="Loading changes…" icon={<Spinner />} />;
  const { changeset: cs, files } = data;
  const file = files[Math.min(sel, files.length - 1)];

  const act = async (what: string, fn: () => Promise<unknown>) => {
    setBusy(what);
    try {
      await fn();
      setData(await api.getChangeset(changesetId));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const revert = async () => {
    setBusy('revert');
    try {
      const r = await api.proposeRevert(cs.id, false);
      openTab(`review:${r.id}`);
    } catch (e) {
      if (errorKind(e) === 'conflict' && window.confirm(`${errorMessage(e)}\n\nCreate a revert proposal that overwrites those later edits? You will review it before anything changes.`)) {
        const r = await api.proposeRevert(cs.id, true).catch((x) => (toast('bad', errorMessage(x)), null));
        if (r) openTab(`review:${r.id}`);
      } else if (errorKind(e) !== 'conflict') toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const restore = async () => {
    setBusy('restore');
    try {
      const r = await api.proposeRestoreTo(cs.id);
      openTab(`review:${r.id}`);
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div className={`banner ${cs.status === 'proposed' ? 'pending' : ''}`} style={{ alignItems: 'flex-start', flexDirection: 'column', gap: 6 }}>
        <div className="row" style={{ width: '100%' }}>
          <strong className="grow ellipsis">{cs.title}</strong>
          <Badge>{cs.kind}</Badge>
          <StatusBadge status={cs.status} />
          <span className="faint tiny">{timeAgo(cs.appliedAt ?? cs.createdAt)}</span>
        </div>
        {cs.summary && !cs.summary.startsWith('reverts:') && !cs.summary.startsWith('restores-to:') && (
          <div className="small muted" style={{ whiteSpace: 'pre-wrap', maxHeight: 90, overflow: 'auto', width: '100%' }}>
            {cs.summary}
          </div>
        )}
        <div className="row" style={{ width: '100%' }}>
          {cs.status === 'proposed' && (
            <>
              <span className="small grow">Nothing has been written yet. Review the diff, then apply or reject.</span>
              <Button size="sm" variant="ghost" icon={<X size={13} />} loading={busy === 'reject'} disabled={!!busy} onClick={() => void act('reject', () => api.rejectChangeset(cs.id))}>
                Reject
              </Button>
              <Button size="sm" variant="primary" icon={<Check size={13} />} loading={busy === 'apply'} disabled={!!busy} onClick={() => void act('apply', () => api.applyChangeset(cs.id))}>
                Apply {files.length} file{files.length === 1 ? '' : 's'}
              </Button>
            </>
          )}
          {cs.status === 'applied' && (
            <>
              <span className="small grow muted">Applied. You can undo this change or return the whole project to this point.</span>
              <Button size="sm" icon={<Undo2 size={13} />} loading={busy === 'revert'} disabled={!!busy} onClick={() => void revert()}>
                Revert this change
              </Button>
              <Button size="sm" icon={<RotateCcw size={13} />} loading={busy === 'restore'} disabled={!!busy} onClick={() => void restore()}>
                Restore project to here
              </Button>
            </>
          )}
          {(cs.status === 'rejected' || cs.status === 'reverted') && <span className="small muted">This change is {cs.status}; files on disk were not modified by it.</span>}
        </div>
      </div>
      {files.length === 0 ? (
        <Empty title="No file changes" />
      ) : (
        <div className="review" style={{ flex: 1 }}>
          <div className="review-files" role="listbox" aria-label="Changed files">
            {files.map((f, i) => {
              const st = cs.files.find((x) => x.path === f.path);
              return (
                <button key={f.path} className={`review-file ${i === sel ? 'on' : ''}`} onClick={() => setSel(i)} role="option" aria-selected={i === sel}>
                  <Badge tone={f.op === 'create' ? 'ok' : f.op === 'delete' ? 'bad' : 'warn'}>{f.op[0].toUpperCase()}</Badge>
                  <span className="ellipsis grow mono tiny">{f.path}</span>
                  {st && (
                    <>
                      <span className="stat-add">+{st.added}</span>
                      <span className="stat-del">−{st.removed}</span>
                    </>
                  )}
                </button>
              );
            })}
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', minHeight: 0 }}>
            <div className="banner" style={{ background: 'var(--bg-1)' }}>
              <span className="mono small grow ellipsis">{file.path}</span>
              <Segmented label="View" value={mode} onChange={setMode} options={[{ value: 'diff', label: 'Diff' }, { value: 'full', label: 'Resulting file' }]} />
            </div>
            {cs.status === 'proposed' && file.op === 'modify' && file.path === 'package.json' && (
              <div style={{ padding: 8 }}>
                <Callout tone="warn">This change edits package.json. New dependencies are installed only after you approve the install.</Callout>
              </div>
            )}
            <div style={{ flex: 1, minHeight: 0 }}>
              {file.op === 'delete' ? (
                <CodeEditor key={`${cs.id}:${file.path}:del`} path={file.path} value="" original={file.before ?? ''} readOnly />
              ) : mode === 'diff' ? (
                <CodeEditor key={`${cs.id}:${file.path}:diff`} path={file.path} value={file.after ?? ''} original={file.before ?? ''} readOnly />
              ) : (
                <CodeEditor key={`${cs.id}:${file.path}:full`} path={file.path} value={file.after ?? ''} readOnly />
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
