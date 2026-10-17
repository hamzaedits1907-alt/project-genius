import { useEffect, useRef, useState } from 'react';
import { Panel, PanelGroup, PanelResizeHandle, type ImperativePanelHandle } from 'react-resizable-panels';
import { Brain, ClipboardList, Code2, FileStack, Plug, Download, FolderTree, GitBranch, Hammer, History as HistoryIcon, Monitor, Package, Play, Search, Square, Stethoscope, TerminalSquare, TestTube2, UploadCloud, Wand2, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import type { Operation } from '../../lib/types';
import { useApp } from '../../store/app';
import { runApp, useWorkspace } from '../../store/workspace';
import { Badge, Button, IconButton, Segmented, Spinner, StatusBadge } from '../../components/ui';
import PlanView from './PlanView';
import FileExplorer from './FileExplorer';
import SearchPanel from './SearchPanel';
import GitPanel from './GitPanel';
import HistoryPanel from './HistoryPanel';
import MemoryPanel from './MemoryPanel';
import ReferencesPanel from './ReferencesPanel';
import IntegrationsPanel from './IntegrationsPanel';
import EditorArea from './EditorArea';
import BottomPanel from './BottomPanel';
import AssistantPanel from './AssistantPanel';
import PreviewPanel from './PreviewPanel';
import ExportDialog from './ExportDialog';
import PublishDialog from './PublishDialog';
import QuickOpen from './QuickOpen';
import { registerCommands, type PaletteCommand } from '../../lib/commands';

type Side = 'files' | 'search' | 'git' | 'history' | 'memory' | 'references' | 'integrations';

export default function Workspace({ projectId }: { projectId: string }) {
  const open = useWorkspace((s) => s.open);
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const preview = useWorkspace((s) => s.preview);
  const rightTab = useWorkspace((s) => s.rightTab);
  const setWs = useWorkspace((s) => s.set);
  const openTab = useWorkspace((s) => s.openTab);
  const toast = useApp((s) => s.toast);
  const uiMode = useApp((s) => s.settings?.uiMode ?? 'simple');
  const setUiMode = useApp((s) => s.setUiMode);
  const stage = useWorkspace((s) => s.stage);
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

  const running = tasks.filter((t) => t.kind !== 'verify');
  const verifying = tasks.filter((t) => t.kind === 'verify');
  const lastBuild = project?.status.lastBuild;

  const runOp = async (op: Operation) => {
    if (!project) return;
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
    if (!project) return;
    setBusyOp('preview');
    try {
      if (preview.status === 'ready' || preview.status === 'starting') {
        await api.stopPreview(project.id);
      } else {
        setWs({ rightTab: 'preview', stage: 'preview' });
        await runApp(project.id);
      }
    } catch (e) {
      useWorkspace.getState().setPreview({ status: 'failed', error: errorMessage(e) });
      toast('bad', errorMessage(e));
    } finally {
      setBusyOp(null);
    }
  };
  const diagnose = async () => {
    if (!project) return;
    try {
      await api.startDiagnose(project.id, null);
      setWs({ rightTab: 'assistant' });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const previewOn = preview.status === 'ready' || preview.status === 'starting';
  /** Opens an IDE panel, switching to advanced mode first when needed. */
  const showAdvanced = async (then: () => void) => {
    if (uiMode !== 'advanced') await setUiMode('advanced');
    // Panels mount with the advanced layout; let it render before expanding them.
    setTimeout(then, 0);
  };
  const showSide = (id: Side) =>
    void showAdvanced(() => {
      setSide(id);
      if (sideRef.current?.isCollapsed()) sideRef.current.expand();
    });

  // Command palette (Ctrl+K): this workspace's actions, re-registered with fresh state each render.
  useEffect(() => {
    if (!project || project.id !== projectId) return;
    const simpleMode = uiMode === 'simple';
    const cmds: PaletteCommand[] = [];
    const add = (c: Omit<PaletteCommand, 'group'>) => cmds.push({ group: 'Project', ...c });
    if (hasPkg) {
      add({ id: 'ws.run', title: previewOn ? 'Stop app' : 'Run app', subtitle: previewOn ? 'Stop the live preview' : 'Start the live preview', keywords: ['preview', 'start', 'dev server', 'launch'], icon: previewOn ? <Square size={15} /> : <Play size={15} />, run: () => togglePreview() });
      add({ id: 'ws.publish', title: 'Publish', subtitle: 'Put the project on GitHub', keywords: ['deploy', 'github', 'push', 'share'], icon: <UploadCloud size={15} />, run: () => setDialog('publish') });
    }
    if (project.spec) add({ id: 'ws.plan', title: 'Show Plan', icon: <ClipboardList size={15} />, run: () => (simpleMode ? setWs({ stage: 'plan' }) : openTab('plan')) });
    add({ id: 'ws.code', title: 'Show Code', keywords: ['files', 'editor', 'explorer'], shortcut: simpleMode ? undefined : 'Ctrl+B', icon: <Code2 size={15} />, run: () => (simpleMode ? setWs({ stage: 'code' }) : showSide('files')) });
    add({ id: 'ws.preview', title: 'Show Preview', icon: <Monitor size={15} />, run: () => setWs(simpleMode ? { stage: 'preview' } : { rightTab: 'preview' }) });
    add({ id: 'ws.quickopen', title: 'Go to file…', keywords: ['quick open', 'find file'], shortcut: 'Ctrl+P', icon: <Search size={15} />, run: () => setDialog('quick') });
    add({ id: 'ws.history', title: 'Open History', subtitle: simpleMode ? 'Advanced mode' : 'History & recovery', keywords: ['undo', 'restore', 'changes', 'recovery'], icon: <HistoryIcon size={15} />, run: () => showSide('history') });
    add({ id: 'ws.git', title: 'Open Git', subtitle: simpleMode ? 'Advanced mode' : 'Source control', keywords: ['source control', 'commit', 'branch', 'version'], icon: <GitBranch size={15} />, run: () => showSide('git') });
    add({
      id: 'ws.terminal',
      title: 'Open Terminal',
      subtitle: simpleMode ? 'Advanced mode' : undefined,
      keywords: ['console', 'output', 'logs'],
      shortcut: simpleMode ? undefined : 'Ctrl+J',
      icon: <TerminalSquare size={15} />,
      run: () =>
        void showAdvanced(() => {
          setWs({ bottomTab: 'terminal' });
          if (bottomRef.current?.isCollapsed()) bottomRef.current.expand();
        }),
    });
    if (!simpleMode) {
      add({ id: 'ws.assistant', title: 'Show Assistant', keywords: ['chat', 'genius', 'ai'], icon: <Wand2 size={15} />, run: () => setWs({ rightTab: 'assistant' }) });
      if (hasPkg) {
        add({ id: 'ws.install', title: 'Install dependencies', keywords: ['npm install'], icon: <Package size={15} />, run: () => runOp('install') });
        add({ id: 'ws.build', title: 'Build', subtitle: 'Production build', keywords: ['compile', 'npm run build'], icon: <Hammer size={15} />, run: () => runOp('build') });
        add({ id: 'ws.test', title: 'Run tests', keywords: ['npm test', 'vitest'], icon: <TestTube2 size={15} />, run: () => runOp('test') });
      }
      add({ id: 'ws.export', title: 'Export source', keywords: ['zip', 'download'], icon: <Download size={15} />, run: () => setDialog('export') });
    }
    if (lastBuild?.status === 'failed' && running.length === 0) add({ id: 'ws.doctor', title: simpleMode ? 'Fix it for me' : 'Build Doctor', subtitle: 'Diagnose the failed build', keywords: ['repair', 'fix', 'error'], icon: <Stethoscope size={15} />, run: () => diagnose() });
    for (const t of running) add({ id: `ws.cancel.${t.id}`, title: `Stop: ${t.title}`, subtitle: 'Cancel the running task', keywords: ['cancel', 'abort'], icon: <X size={15} />, run: () => api.cancelTask(t.id) });
    return registerCommands('workspace', cmds);
  });

  if (!project || project.id !== projectId) {
    return (
      <div className="center" style={{ height: '100%' }}>
        <Spinner size={20} />
      </div>
    );
  }

  const railItems: { id: Side; label: string; icon: React.ReactNode }[] = [
    { id: 'files', label: 'Files', icon: <FolderTree size={18} /> },
    { id: 'search', label: 'Search', icon: <Search size={18} /> },
    { id: 'git', label: 'Source control', icon: <GitBranch size={18} /> },
    { id: 'history', label: 'History & recovery', icon: <HistoryIcon size={18} /> },
    { id: 'memory', label: 'Project memory', icon: <Brain size={18} /> },
    { id: 'references', label: 'Imported files', icon: <FileStack size={18} /> },
    { id: 'integrations', label: 'Integrations', icon: <Plug size={18} /> },
  ];

  const simple = uiMode === 'simple';
  const modeToggle = (
    <Segmented<'simple' | 'advanced'>
      label="Interface mode"
      value={uiMode}
      onChange={(m) => void setUiMode(m)}
      options={[
        { value: 'simple', label: 'Simple' },
        { value: 'advanced', label: 'Advanced' },
      ]}
    />
  );
  const runButton = (
    <Button size="sm" variant={previewOn ? 'danger' : 'ok'} icon={previewOn ? <Square size={13} /> : <Play size={13} />} disabled={!hasPkg} loading={busyOp === 'preview'} onClick={() => void togglePreview()}>
      {previewOn ? 'Stop' : 'Run'}
    </Button>
  );
  const publishButton = (
    <Button size="sm" variant="primary" icon={<UploadCloud size={14} />} onClick={() => setDialog('publish')} disabled={!hasPkg}>
      Publish
    </Button>
  );

  return (
    <div className={`ws ${simple ? 'simple' : 'advanced'}`}>
      <header className="ws-top">
        <span className="name ellipsis" title={project.path}>
          {project.name}
        </span>
        {simple ? (
          <FriendlyStatus building={running.length > 0 || verifying.length > 0} lastBuild={lastBuild?.status} />
        ) : lastBuild ? (
          <StatusBadge status={lastBuild.status} label={`build ${lastBuild.status}`} />
        ) : (
          <Badge>not built</Badge>
        )}
        {!simple &&
          running.map((t) => (
            <span key={t.id} className="badge run" style={{ gap: 6 }}>
              <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} />
              {t.title}
              <button className="icon-btn" style={{ width: 16, height: 16 }} aria-label={`Cancel ${t.title}`} title="Cancel" onClick={() => void api.cancelTask(t.id)}>
                <X size={11} />
              </button>
            </span>
          ))}
        {!simple && verifying.length > 0 && (
          <span className="badge run">
            <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} /> {verifying[0].title}
          </span>
        )}
        <span className="spacer" />
        {modeToggle}
        {simple ? (
          <>
            {lastBuild?.status === 'failed' && (
              <Button size="sm" variant="ghost" icon={<Wand2 size={14} />} onClick={() => void diagnose()} disabled={running.length > 0}>
                Fix it for me
              </Button>
            )}
            {runButton}
            {publishButton}
          </>
        ) : (
          <>
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
            {runButton}
            <IconButton label="Export source" onClick={() => setDialog('export')}>
              <Download size={16} />
            </IconButton>
            {publishButton}
          </>
        )}
      </header>
      {simple ? (
        <PanelGroup direction="horizontal" autoSaveId="genius-simple-h" className="ws-simple">
          <Panel defaultSize={40} minSize={28} order={1}>
            <div className="panel chat-panel">
              <AssistantPanel variant="simple" />
            </div>
          </Panel>
          <PanelResizeHandle className="resize-h" />
          <Panel defaultSize={60} minSize={30} order={2}>
            <div className="panel stage">
              <div className="stage-tabs" role="tablist" aria-label="View">
                <button role="tab" aria-selected={stage === 'preview'} className={stage === 'preview' ? 'on' : ''} onClick={() => setWs({ stage: 'preview' })}>
                  <Monitor size={14} /> Preview <span className={`dot ${preview.status === 'ready' ? 'ok' : preview.status === 'starting' ? 'run' : preview.status === 'failed' ? 'bad' : ''}`} />
                </button>
                {project.spec && (
                  <button role="tab" aria-selected={stage === 'plan'} className={stage === 'plan' ? 'on' : ''} onClick={() => setWs({ stage: 'plan' })}>
                    <ClipboardList size={14} /> Plan
                  </button>
                )}
                <button role="tab" aria-selected={stage === 'code'} className={stage === 'code' ? 'on' : ''} onClick={() => setWs({ stage: 'code' })}>
                  <Code2 size={14} /> Code
                </button>
              </div>
              <div className="stage-body">
                {stage === 'preview' && <PreviewPanel onStart={() => void togglePreview()} />}
                {stage === 'plan' && (
                  <div className="scroll" style={{ height: '100%' }}>
                    <PlanView />
                  </div>
                )}
                {stage === 'code' && (
                  <div className="stage-code">
                    <div className="panel" style={{ borderRight: '1px solid var(--border)' }}>
                      <FileExplorer />
                    </div>
                    <EditorArea />
                  </div>
                )}
              </div>
            </div>
          </Panel>
        </PanelGroup>
      ) : (
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
                  // expand() on a panel that is already open is not a no-op in react-resizable-panels.
                  if (p?.isCollapsed()) p.expand();
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
              {side === 'references' && <ReferencesPanel />}
              {side === 'integrations' && <IntegrationsPanel onPublish={() => setDialog('publish')} />}
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
      )}
      {dialog === 'export' && <ExportDialog onClose={() => setDialog(null)} />}
      {dialog === 'publish' && <PublishDialog onClose={() => setDialog(null)} />}
      {dialog === 'quick' && <QuickOpen onClose={() => setDialog(null)} />}
    </div>
  );
}

/** Plain-language project status for simple mode. */
function FriendlyStatus({ building, lastBuild }: { building: boolean; lastBuild?: string }) {
  if (building)
    return (
      <span className="pill run">
        <span className="dot run" /> Working…
      </span>
    );
  if (lastBuild === 'failed')
    return (
      <span className="pill bad">
        <span className="dot bad" /> Needs a fix
      </span>
    );
  if (lastBuild === 'succeeded')
    return (
      <span className="pill ok">
        <span className="dot ok" /> Ready
      </span>
    );
  return (
    <span className="pill">
      <span className="dot" /> Not built yet
    </span>
  );
}
