import { useMemo, useState } from 'react';
import { ChevronDown, ChevronRight, FilePlus, FileText, Folder, Lock, RefreshCw, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { buildTree, type TreeNode } from '../../lib/logic';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Empty, IconButton, Modal, Button, Field } from '../../components/ui';

export default function FileExplorer() {
  const files = useWorkspace((s) => s.files);
  const drafts = useWorkspace((s) => s.drafts);
  const active = useWorkspace((s) => s.active);
  const buffers = useWorkspace((s) => s.buffers);
  const openFile = useWorkspace((s) => s.openFile);
  const openTab = useWorkspace((s) => s.openTab);
  const refreshFiles = useWorkspace((s) => s.refreshFiles);
  const projectId = useWorkspace((s) => s.projectId);
  const toast = useApp((s) => s.toast);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  const [creating, setCreating] = useState(false);
  const [newPath, setNewPath] = useState('src/');

  // Show draft-only files (being written by the AI, not yet on disk) in the tree too.
  const tree = useMemo(() => {
    const all: { path: string }[] = [...files];
    for (const p of Object.keys(drafts)) if (!files.some((f) => f.path === p)) all.push({ path: p });
    return buildTree(all);
  }, [files, drafts]);

  const create = async () => {
    if (!projectId) return;
    try {
      await api.saveFile(projectId, newPath.trim(), '');
      await refreshFiles();
      await openFile(newPath.trim());
      setCreating(false);
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const remove = async (path: string) => {
    if (!projectId || !window.confirm(`Delete ${path}? You can restore it from History.`)) return;
    try {
      await api.deleteFile(projectId, path);
      useWorkspace.getState().closeTab(`file:${path}`);
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const render = (nodes: TreeNode[], depth: number): React.ReactNode =>
    nodes.map((n) => {
      const pad = 8 + depth * 12;
      if (n.dir) {
        const isCollapsed = collapsed[n.path];
        return (
          <div key={n.path}>
            <button className="tree-row" style={{ paddingLeft: pad }} onClick={() => setCollapsed({ ...collapsed, [n.path]: !isCollapsed })} aria-expanded={!isCollapsed}>
              {isCollapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}
              <Folder size={13} color="var(--text-3)" />
              {n.name}
            </button>
            {!isCollapsed && render(n.children, depth + 1)}
          </div>
        );
      }
      const draft = drafts[n.path];
      const onDisk = !!n.entry && 'size' in n.entry;
      const dirty = buffers[n.path] && buffers[n.path].content !== buffers[n.path].saved;
      const tabId = onDisk ? `file:${n.path}` : `draft:${n.path}`;
      return (
        <div key={n.path} className="row" style={{ gap: 0 }}>
          <button
            className={`tree-row ${active === tabId || active === `draft:${n.path}` ? 'on' : ''}`}
            style={{ paddingLeft: pad + 17 }}
            onClick={() => (onDisk ? void openFile(n.path) : openTab(`draft:${n.path}`))}
            title={n.entry?.sensitive ? 'Excluded from AI context by default' : n.path}
          >
            {n.entry?.sensitive ? <Lock size={12} color="var(--text-3)" /> : <FileText size={13} color="var(--text-3)" />}
            <span className="ellipsis">{n.name}</span>
            {draft && draft.status === 'streaming' && <span className="tag draft">writing…</span>}
            {draft && draft.status !== 'streaming' && <span className="tag draft">{draft.status === 'delete' ? 'delete?' : 'draft'}</span>}
            {dirty && <span className="tag mod">●</span>}
          </button>
          {onDisk && (
            <IconButton label={`Delete ${n.path}`} onClick={() => void remove(n.path)} style={{ width: 22, height: 22, opacity: 0.6 }}>
              <Trash2 size={12} />
            </IconButton>
          )}
        </div>
      );
    });

  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">Files</span>
        <IconButton label="New file" onClick={() => setCreating(true)}>
          <FilePlus size={15} />
        </IconButton>
        <IconButton label="Refresh" onClick={() => void refreshFiles()}>
          <RefreshCw size={14} />
        </IconButton>
      </div>
      <div className="scroll grow tree">
        {tree.length === 0 ? (
          <Empty title="No files yet">Generate the application from the plan, or create a file.</Empty>
        ) : (
          render(tree, 0)
        )}
      </div>
      <div className="faint tiny" style={{ padding: '6px 10px', borderTop: '1px solid var(--border)' }}>
        <Lock size={10} /> = never sent to the AI (secrets, lockfiles, binaries)
      </div>
      {creating && (
        <Modal
          title="New file"
          onClose={() => setCreating(false)}
          footer={
            <>
              <Button onClick={() => setCreating(false)}>Cancel</Button>
              <Button variant="primary" onClick={() => void create()} disabled={!newPath.trim() || newPath.endsWith('/')}>
                Create
              </Button>
            </>
          }
        >
          <Field label="Path inside the project">
            <input className="input mono" value={newPath} onChange={(e) => setNewPath(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void create()} />
          </Field>
        </Modal>
      )}
    </>
  );
}
