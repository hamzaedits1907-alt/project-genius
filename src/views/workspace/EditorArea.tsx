import { ClipboardList, FileDiff, FileText, GitCommit, Radio, Save, X } from 'lucide-react';
import { useWorkspace, type TabId } from '../../store/workspace';
import CodeEditor from '../../components/CodeEditor';
import { Badge, Button, Empty, Kbd } from '../../components/ui';
import ReviewView from './ReviewView';
import PlanView from './PlanView';
import CommitView from './CommitView';

function tabLabel(tab: TabId): { label: string; icon: React.ReactNode } {
  const [kind, ...rest] = tab.split(':');
  const val = rest.join(':');
  const base = val.split('/').pop() ?? val;
  switch (kind) {
    case 'file':
      return { label: base, icon: <FileText size={13} /> };
    case 'draft':
      return { label: `${base} (AI draft)`, icon: <Radio size={13} color="var(--run)" /> };
    case 'review':
      return { label: 'Changes', icon: <FileDiff size={13} color="var(--warn)" /> };
    case 'commit':
      return { label: `Commit ${val.slice(0, 7)}`, icon: <GitCommit size={13} /> };
    case 'plan':
      return { label: 'Plan', icon: <ClipboardList size={13} /> };
    default:
      return { label: tab, icon: null };
  }
}

export default function EditorArea() {
  const tabs = useWorkspace((s) => s.tabs);
  const active = useWorkspace((s) => s.active);
  const buffers = useWorkspace((s) => s.buffers);
  const openTab = useWorkspace((s) => s.openTab);
  const closeTab = useWorkspace((s) => s.closeTab);

  return (
    <div className="panel">
      <div className="tabs" role="tablist">
        {tabs.map((t) => {
          const { label, icon } = tabLabel(t);
          const path = t.startsWith('file:') ? t.slice(5) : null;
          const dirty = path && buffers[path] && buffers[path].content !== buffers[path].saved;
          return (
            <div key={t} className={`tab ${active === t ? 'on' : ''}`} role="tab" aria-selected={active === t} tabIndex={0} onClick={() => openTab(t)} onKeyDown={(e) => e.key === 'Enter' && openTab(t)} title={t.split(':').slice(1).join(':')}>
              {icon}
              {label}
              {dirty && <span style={{ color: 'var(--warn)' }}>●</span>}
              <button
                className="x"
                aria-label={`Close ${label}`}
                onClick={(e) => {
                  e.stopPropagation();
                  if (dirty && !window.confirm(`${label} has unsaved changes. Close anyway?`)) return;
                  closeTab(t);
                }}
              >
                <X size={12} />
              </button>
            </div>
          );
        })}
      </div>
      <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
        {!active ? (
          <Empty icon={<FileText size={20} />} title="No file open">
            Pick a file from the explorer, or press <Kbd>Ctrl</Kbd> + <Kbd>P</Kbd> to open one quickly. While the AI writes code, its drafts open here live.
          </Empty>
        ) : active.startsWith('file:') ? (
          <FileTab path={active.slice(5)} />
        ) : active.startsWith('draft:') ? (
          <DraftTab path={active.slice(6)} />
        ) : active.startsWith('review:') ? (
          <ReviewView key={active} changesetId={active.slice(7)} />
        ) : active.startsWith('commit:') ? (
          <CommitView key={active} sha={active.slice(7)} />
        ) : active === 'plan' ? (
          <PlanView />
        ) : null}
      </div>
    </div>
  );
}

function FileTab({ path }: { path: string }) {
  const buf = useWorkspace((s) => s.buffers[path]);
  const setBuffer = useWorkspace((s) => s.setBuffer);
  const saveFile = useWorkspace((s) => s.saveFile);
  const draft = useWorkspace((s) => s.drafts[path]);
  const openTab = useWorkspace((s) => s.openTab);
  if (!buf) return <Empty title="Loading…" />;
  if (buf.error) return <Empty title="Cannot open this file">{buf.error}</Empty>;
  const dirty = buf.content !== buf.saved;
  return (
    <>
      <div className="banner" style={{ background: 'var(--bg-1)' }}>
        <span className="mono faint grow ellipsis">{path}</span>
        {draft && (
          <Button size="sm" variant="ghost" onClick={() => openTab(`draft:${path}`)}>
            AI draft available
          </Button>
        )}
        {dirty ? <Badge tone="warn">Unsaved</Badge> : <Badge>Saved</Badge>}
        <Button size="sm" icon={<Save size={13} />} disabled={!dirty} onClick={() => void saveFile(path)} title="Save (Ctrl+S)">
          Save
        </Button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <CodeEditor path={path} value={buf.content} onChange={(v) => setBuffer(path, v)} onSave={() => void saveFile(path)} />
      </div>
    </>
  );
}

/** AI output as it streams. Explicitly not on disk until a changeset is applied. */
function DraftTab({ path }: { path: string }) {
  const draft = useWorkspace((s) => s.drafts[path]);
  const onDisk = useWorkspace((s) => s.files.some((f) => f.path === path));
  const openFile = useWorkspace((s) => s.openFile);
  if (!draft) {
    return (
      <Empty title="Draft no longer pending" action={onDisk ? <Button onClick={() => void openFile(path)}>Open the saved file</Button> : undefined}>
        This draft was applied, rejected, or belongs to a finished task.
      </Empty>
    );
  }
  const label =
    draft.status === 'streaming'
      ? 'AI is writing this file — not saved to disk'
      : draft.status === 'complete'
        ? 'AI draft complete — not saved until the change is applied'
        : draft.status === 'incomplete'
          ? 'Draft was cut off and will NOT be applied'
          : 'AI proposes deleting this file';
  return (
    <>
      <div className="banner draft">
        {draft.status === 'streaming' ? <span className="spinner" style={{ width: 12, height: 12 }} /> : <Radio size={14} color="var(--run)" />}
        <span className="mono small">{path}</span>
        <span className="small grow">{label}</span>
        <Badge tone="run">{draft.content.split('\n').length - 1} lines</Badge>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <CodeEditor path={path} value={draft.content} readOnly follow={draft.status === 'streaming'} />
      </div>
    </>
  );
}
