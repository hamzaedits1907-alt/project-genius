import { useEffect, useRef, useState } from 'react';
import { Panel, PanelGroup, PanelResizeHandle, type ImperativePanelHandle } from 'react-resizable-panels';
import { Brain, ClipboardList, Download, FolderTree, GitBranch, Hammer, History as HistoryIcon, Package, Play, Search, Square, Stethoscope, TestTube2, UploadCloud, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import type { Operation } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, IconButton, Spinner, StatusBadge } from '../../components/ui';
import FileExplorer from './FileExplorer';
import SearchPanel from './SearchPanel';
import GitPanel from './GitPanel';
import HistoryPanel from './HistoryPanel';
import MemoryPanel from './MemoryPanel';
import EditorArea from './EditorArea';
import BottomPanel from './BottomPanel';
import AssistantPanel from './AssistantPanel';
import PreviewPanel from './PreviewPanel';
import ExportDialog from './ExportDialog';
import PublishDialog from './PublishDialog';
import QuickOpen from './QuickOpen';

type Side = 'files' | 'search' | 'git' | 'history' | 'memory';

export default function Workspace({ projectId }: { projectId: string }) {
  const open = useWorkspace((s) => s.open);
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const preview = useWorkspace((s) => s.preview);
  const rightTab = useWorkspace((s) => s.rightTab);
  const setWs = useWorkspace((s) => s.set);
  const openTab = useWorkspace((s) => s.openTab);
  const toast = useApp((s) => s.toast);
  const hasPkg = useWorkspace((s) => s.files.some((f) => f.path === 'package.json'));
  const [side, setSide] = useState<Side>('files');
  const [dialog, setDialog] = useState<'export' | 'publish' | 'quick' | null>(null);
  const [busyOp, setBusyOp] = useState<string | null>(null);
  const sideRef = useRef<ImperativePanelHandle>(null);
  const bottomRef = useRef<ImperativePanelHandle>(null);

  useEffect(() => {
    void open(projectId);
  }, [open, projectId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      if (e.key === 'b') {
        e.preventDefault();
        const p = sideRef.current;
        if (p?.isCollapsed()) p.expand();
        else p?.collapse();
      } else if (e.key === 'j') {
        e.preventDefault();
        const p = bottomRef.current;
        if (p?.isCollapsed()) p.expand();
        else p?.collapse();
      } else if (e.key === 'p' && !e.shiftKey) {
        e.preventDefault();
        setDialog('quick');
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  if (!project || project.id !== projectId) {
    return (
      <div className="center" style={{ height: '100%' }}>
        <Spinner size={20} />
      </div>
    );
  }

  const running = tasks.filter((t) => t.kind !== 'verify');
  const verifying = tasks.filter((t) => t.kind === 'verify');
  const lastBuild = project.status.lastBuild;

  const runOp = async (op: Operation) => {
    setBusyOp(op);
    try {
      await api.startOperation(project.id, op);
      setWs({ bottomTab: 'terminal' });
      bottomRef.current?.expand();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusyOp(null);
    }
  };
  const togglePreview = async () => {
    setBusyOp('preview');
    try {
      if (preview.status === 'ready' || preview.status === 'starting') {
        await api.stopPreview(project.id);
      } else {
        setWs({ rightTab: 'preview' });
        useWorkspace.getState().setPreview({ status: 'starting', error: undefined });
        await api.startPreview(project.id);
      }
    } catch (e) {
      useWorkspace.getState().setPreview({ status: 'failed', error: errorMessage(e) });
      toast('bad', errorMessage(e));
    } finally {
      setBusyOp(null);
    }
  };
  const diagnose = async () => {
    try {
      await api.startDiagnose(project.id, null);
      setWs({ rightTab: 'assistant' });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const railItems: { id: Side; label: string; icon: React.ReactNode }[] = [
    { id: 'files', label: 'Files', icon: <FolderTree size={18} /> },
    { id: 'search', label: 'Search', icon: <Search size={18} /> },
    { id: 'git', label: 'Source control', icon: <GitBranch size={18} /> },
    { id: 'history', label: 'History & recovery', icon: <HistoryIcon size={18} /> },
    { id: 'memory', label: 'Project memory', icon: <Brain size={18} /> },
  ];

  return (
    <div className="ws">
      <header className="ws-top">
        <span className="name ellipsis" title={project.path}>
          {project.name}
        </span>
        {lastBuild ? <StatusBadge status={lastBuild.status} label={`build ${lastBuild.status}`} /> : <Badge>not built</Badge>}
        {running.map((t) => (
          <span key={t.id} className="badge run" style={{ gap: 6 }}>
            <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} />
            {t.title}
            <button className="icon-btn" style={{ width: 16, height: 16 }} aria-label={`Cancel ${t.title}`} title="Cancel" onClick={() => void api.cancelTask(t.id)}>
              <X size={11} />
            </button>
          </span>
        ))}
        {verifying.length > 0 && (
          <span className="badge run">
            <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} /> {verifying[0].title}
          </span>
        )}
        <span className="spacer" />
        <Button size="sm" variant="ghost" icon={<ClipboardList size={14} />} onClick={() => openTab('plan')} disabled={!project.spec}>
          Plan
        </Button>
        <Button size="sm" variant="ghost" icon={<Package size={14} />} disabled={!hasPkg || !!busyOp} loading={busyOp === 'install'} onClick={() => void runOp('install')}>
          Install
        </Button>
        <Button size="sm" variant="ghost" icon={<Hammer size={14} />} disabled={!hasPkg || !!busyOp} loading={busyOp === 'build'} onClick={() => void runOp('build')}>
          Build
        </Button>
        <Button size="sm" variant="ghost" icon={<TestTube2 size={14} />} disabled={!hasPkg || !!busyOp} loading={busyOp === 'test'} onClick={() => void runOp('test')}>
          Test
        </Button>
        {lastBuild?.status === 'failed' && (
          <Button size="sm" variant="ghost" icon={<Stethoscope size={14} />} onClick={() => void diagnose()} disabled={running.length > 0}>
            Build Doctor
          </Button>
        )}
        <Button size="sm" variant={preview.status === 'ready' ? 'danger' : 'ok'} icon={preview.status === 'ready' || preview.status === 'starting' ? <Square size={13} /> : <Play size={13} />} disabled={!hasPkg} loading={busyOp === 'preview'} onClick={() => void togglePreview()}>
          {preview.status === 'ready' || preview.status === 'starting' ? 'Stop' : 'Run'}
        </Button>
        <IconButton label="Export source" onClick={() => setDialog('export')}>
          <Download size={16} />
        </IconButton>
        <Button size="sm" variant="primary" icon={<UploadCloud size={14} />} onClick={() => setDialog('publish')} disabled={!hasPkg}>
          Publish
        </Button>
      </header>
      <div className="ws-body">
        <nav className="ws-rail" aria-label="Workspace sections">
          {railItems.map((r) => (
            <IconButton
              key={r.id}
              label={r.label}
              active={side === r.id}
              onClick={() => {
                const p = sideRef.current;
                if (side === r.id && p && !p.isCollapsed()) p.collapse();
                else {
                  setSide(r.id);
                  p?.expand();
                }
              }}
            >
              {r.icon}
            </IconButton>
          ))}
        </nav>
        <PanelGroup direction="horizontal" autoSaveId="genius-ws-h">
          <Panel ref={sideRef} defaultSize={18} minSize={12} collapsible collapsedSize={0} order={1}>
            <div className="panel" style={{ borderRight: '1px solid var(--border)' }}>
              {side === 'files' && <FileExplorer />}
              {side === 'search' && <SearchPanel />}
              {side === 'git' && <GitPanel onPublish={() => setDialog('publish')} />}
              {side === 'history' && <HistoryPanel />}
              {side === 'memory' && <MemoryPanel />}
            </div>
          </Panel>
          <PanelResizeHandle className="resize-h" />
          <Panel defaultSize={50} minSize={25} order={2}>
            <PanelGroup direction="vertical" autoSaveId="genius-ws-v">
              <Panel defaultSize={66} minSize={20}>
                <EditorArea />
              </Panel>
              <PanelResizeHandle className="resize-v" />
              <Panel ref={bottomRef} defaultSize={34} minSize={10} collapsible collapsedSize={0}>
                <BottomPanel />
              </Panel>
            </PanelGroup>
          </Panel>
          <PanelResizeHandle className="resize-h" />
          <Panel defaultSize={32} minSize={20} order={3}>
            <div className="panel" style={{ borderLeft: '1px solid var(--border)' }}>
              <div className="panel-head">
                <div className="subtabs" role="tablist">
                  <button role="tab" aria-selected={rightTab === 'assistant'} className={rightTab === 'assistant' ? 'on' : ''} onClick={() => setWs({ rightTab: 'assistant' })}>
                    Assistant {running.length > 0 && <span className="dot run" />}
                  </button>
                  <button role="tab" aria-selected={rightTab === 'preview'} className={rightTab === 'preview' ? 'on' : ''} onClick={() => setWs({ rightTab: 'preview' })}>
                    Preview <span className={`dot ${preview.status === 'ready' ? 'ok' : preview.status === 'starting' ? 'run' : preview.status === 'failed' ? 'bad' : ''}`} />
                  </button>
                </div>
              </div>
              <div style={{ flex: 1, minHeight: 0, display: rightTab === 'assistant' ? 'flex' : 'none', flexDirection: 'column' }}>
                <AssistantPanel />
              </div>
              <div style={{ flex: 1, minHeight: 0, display: rightTab === 'preview' ? 'flex' : 'none', flexDirection: 'column' }}>
                <PreviewPanel onStart={() => void togglePreview()} />
              </div>
            </div>
          </Panel>
        </PanelGroup>
      </div>
      {dialog === 'export' && <ExportDialog onClose={() => setDialog(null)} />}
      {dialog === 'publish' && <PublishDialog onClose={() => setDialog(null)} />}
      {dialog === 'quick' && <QuickOpen onClose={() => setDialog(null)} />}
    </div>
  );
}
