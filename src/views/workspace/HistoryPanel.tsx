import { useEffect, useState } from 'react';
import { Bot, FilePen, LayoutTemplate, RotateCcw, Stethoscope, Undo2, Wand2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { timeAgo } from '../../lib/logic';
import type { Changeset } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Empty, StatusBadge } from '../../components/ui';

const KIND_ICON: Record<Changeset['kind'], React.ReactNode> = {
  template: <LayoutTemplate size={14} />,
  generate: <Wand2 size={14} />,
  change: <Bot size={14} />,
  repair: <Stethoscope size={14} />,
  manual: <FilePen size={14} />,
  restore: <Undo2 size={14} />,
};

const KIND_LABEL: Record<Changeset['kind'], string> = {
  template: 'Template',
  generate: 'AI generation',
  change: 'AI change',
  repair: 'Build Doctor fix',
  manual: 'Your edit',
  restore: 'Restore',
};

export default function HistoryPanel() {
  const projectId = useWorkspace((s) => s.projectId);
  const version = useWorkspace((s) => s.historyVersion);
  const openTab = useWorkspace((s) => s.openTab);
  const toast = useApp((s) => s.toast);
  const [items, setItems] = useState<Changeset[] | null>(null);
  useEffect(() => {
    if (projectId) api.listChangesets(projectId).then(setItems).catch((e) => toast('bad', errorMessage(e)));
  }, [projectId, version, toast]);

  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">History & recovery</span>
      </div>
      <div className="faint tiny" style={{ padding: '6px 12px' }}>
        Every change to project files is recorded here — AI output, fixes and your own edits. Open one to see its diff, revert it, or restore the project to that point.
      </div>
      <div className="scroll grow">
        {items?.length === 0 && <Empty icon={<RotateCcw size={18} />} title="No history yet" />}
        {items?.map((c) => (
          <button key={c.id} className="run-item" onClick={() => openTab(`review:${c.id}`)}>
            <div className="row" style={{ gap: 6 }}>
              <span className="faint">{KIND_ICON[c.kind]}</span>
              <span className="ellipsis grow small" style={{ fontWeight: 600 }}>
                {c.title}
              </span>
              <StatusBadge status={c.status} />
            </div>
            <div className="row faint tiny" style={{ whiteSpace: 'nowrap', overflow: 'hidden' }}>
              <span>{KIND_LABEL[c.kind]}</span>·<span>{c.files.length} file(s)</span>·
              <span className="stat-add">+{c.files.reduce((a, f) => a + f.added, 0)}</span>
              <span className="stat-del">−{c.files.reduce((a, f) => a + f.removed, 0)}</span>
              <span className="spacer" />
              <span>{timeAgo(c.appliedAt ?? c.createdAt)}</span>
            </div>
          </button>
        ))}
      </div>
    </>
  );
}
