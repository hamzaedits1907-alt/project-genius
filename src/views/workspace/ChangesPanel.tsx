import { useEffect, useState } from 'react';
import { Check, FileDiff, Undo2, X } from 'lucide-react';
import { api, errorKind, errorMessage } from '../../lib/api';
import { timeAgo } from '../../lib/logic';
import type { Changeset } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Empty, Modal } from '../../components/ui';
import ReviewView from './ReviewView';

const KIND: Record<string, string> = { change: 'AI change', repair: 'Automatic fix', generate: 'Generated app', template: 'Template', manual: 'Your edit', restore: 'Undo / restore' };

/** What changed, in plain terms, with Apply / Reject / Undo. Diffs open on request. */
export default function ChangesPanel() {
  const projectId = useWorkspace((s) => s.projectId);
  const historyVersion = useWorkspace((s) => s.historyVersion);
  const toast = useApp((s) => s.toast);
  const [list, setList] = useState<Changeset[] | null>(null);
  const [review, setReview] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    if (!projectId) return;
    api.listChangesets(projectId).then(setList).catch((e) => toast('bad', errorMessage(e)));
  }, [projectId, historyVersion, toast]);

  const act = async (id: string, fn: () => Promise<unknown>) => {
    setBusy(id);
    try {
      await fn();
      if (projectId) setList(await api.listChangesets(projectId));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const undo = (cs: Changeset) =>
    act(cs.id, async () => {
      try {
        const r = await api.proposeRevert(cs.id, false);
        setReview(r.id);
      } catch (e) {
        if (errorKind(e) === 'conflict' && window.confirm(`${errorMessage(e)}\n\nPrepare an undo that also replaces those later edits? You will see it before anything changes.`)) {
          setReview((await api.proposeRevert(cs.id, true)).id);
        } else throw e;
      }
    });

  if (!list) return null;
  const pending = list.filter((c) => c.status === 'proposed');
  const recent = list.filter((c) => c.status !== 'proposed' && c.status !== 'rejected' && c.kind !== 'template').slice(0, 15);
  return (
    <div className="scroll" style={{ height: '100%', padding: 10 }}>
      {pending.length === 0 && recent.length === 0 && <Empty icon={<FileDiff size={18} />} title="No changes yet">When Genius changes your project, each change appears here. You can look at it, apply it, or undo it later.</Empty>}
      {pending.length > 0 && <div className="section-label">Waiting for you</div>}
      {pending.map((cs) => (
        <div key={cs.id} className="change-row pending">
          <div className="grow" style={{ minWidth: 0 }}>
            <div className="ellipsis" style={{ fontWeight: 600 }}>{cs.title}</div>
            <div className="tiny faint">{KIND[cs.kind] ?? cs.kind} · {cs.files.length} file(s): {cs.files.map((f) => f.path.split('/').pop()).join(', ')}</div>
          </div>
          <Button size="sm" variant="ghost" onClick={() => setReview(cs.id)}>View</Button>
          <Button size="sm" variant="ok" icon={<Check size={12} />} loading={busy === cs.id} onClick={() => void act(cs.id, () => api.applyChangeset(cs.id))}>
            Apply
          </Button>
          <Button size="sm" variant="ghost" icon={<X size={12} />} disabled={busy === cs.id} onClick={() => void act(cs.id, () => api.rejectChangeset(cs.id))}>
            Reject
          </Button>
        </div>
      ))}
      {recent.length > 0 && <div className="section-label">Recent</div>}
      {recent.map((cs) => (
        <div key={cs.id} className="change-row">
          <div className="grow" style={{ minWidth: 0 }}>
            <div className="ellipsis">{cs.title}</div>
            <div className="tiny faint">
              {KIND[cs.kind] ?? cs.kind} · {timeAgo(cs.appliedAt ?? cs.createdAt)} · {cs.files.length} file(s)
            </div>
          </div>
          {cs.status === 'reverted' ? <Badge>undone</Badge> : null}
          <Button size="sm" variant="ghost" onClick={() => setReview(cs.id)}>View</Button>
          {cs.status === 'applied' && (
            <Button size="sm" variant="ghost" icon={<Undo2 size={12} />} loading={busy === cs.id} onClick={() => void undo(cs)} title="Prepare an undo of this change (you review it first)">
              Undo
            </Button>
          )}
        </div>
      ))}
      {review && (
        <Modal title="Changes" wide onClose={() => setReview(null)}>
          <div style={{ height: '70vh', display: 'flex', flexDirection: 'column' }}>
            <ReviewView changesetId={review} onOpen={setReview} />
          </div>
        </Modal>
      )}
    </div>
  );
}
