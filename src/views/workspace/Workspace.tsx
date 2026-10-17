import { useEffect, useRef, useState } from 'react';
import { Panel, PanelGroup, PanelResizeHandle, type ImperativePanelHandle } from 'react-resizable-panels';
import {
  Brain, ChevronDown, ClipboardList, Code2, FileDiff, FileStack, Plug, Download, FolderTree, GitBranch, Hammer, History as HistoryIcon, LayoutPanelLeft, MoreHorizontal, Package, Play,
  Search, Square, Stethoscope, TestTube2, UploadCloud, Wand2, X, Activity as ActivityIcon, CircleDashed, HelpCircle,
} from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { clockTime, previewBusy, previewLabel, STATUS_LABEL, summarizeActivity } from '../../lib/logic';
import type { Operation } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Dot, Empty, IconButton, Spinner } from '../../components/ui';
import FileExplorer from './FileExplorer';
import SearchPanel from './SearchPanel';
import GitPanel from './GitPanel';
import HistoryPanel from './HistoryPanel';
import MemoryPanel from './MemoryPanel';
import ReferencesPanel from './ReferencesPanel';
import IntegrationsPanel from './IntegrationsPanel';
import EditorArea from './EditorArea';
import BottomPanel, { KIND_ICON } from './BottomPanel';
import AssistantPanel from './AssistantPanel';
import PreviewPanel from './PreviewPanel';
import ChangesPanel from './ChangesPanel';
import ExportDialog from './ExportDialog';
import PublishDialog from './PublishDialog';
import QuickOpen from './QuickOpen';
import { usePreviewActions } from './usePreviewActions';

type Side = 'files' | 'search' | 'git' | 'history' | 'memory' | 'references' | 'integrations';

export default function Workspace({ projectId }: { projectId: string }) {
  const open = useWorkspace((s) => s.open);
  const project = useWorkspace((s) => s.project);
  const layout = useWorkspace((s) => s.layout);
  const [dialog, setDialog] = useState<'export' | 'publish' | 'quick' | null>(null);

  useEffect(() => {
    void open(projectId);
  }, [open, projectId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'p' && !e.shiftKey) {
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

  return (
    <div className="ws">
      <Header onDialog={setDialog} />
      {layout === 'simple' ? <SimpleBody /> : <AdvancedBody onPublish={() => setDialog('publish')} />}
      {dialog === 'export' && <ExportDialog onClose={() => setDialog(null)} />}
      {dialog === 'publish' && <PublishDialog onClose={() => setDialog(null)} />}
      {dialog === 'quick' && <QuickOpen onClose={() => setDialog(null)} />}
    </div>
  );
}

// ------------------------------------------------------------------ header

function Header({ onDialog }: { onDialog: (d: 'export' | 'publish') => void }) {
  const project = useWorkspace((s) => s.project)!;
  const tasks = useWorkspace((s) => s.tasks);
  const preview = useWorkspace((s) => s.preview);
  const layout = useWorkspace((s) => s.layout);
  const setLayout = useWorkspace((s) => s.setLayout);
  const setWs = useWorkspace((s) => s.set);
  const openTab = useWorkspace((s) => s.openTab);
  const hasPkg = useWorkspace((s) => s.files.some((f) => f.path === 'package.json'));
  const toast = useApp((s) => s.toast);
  const actions = usePreviewActions();
  const [busyOp, setBusyOp] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const running = tasks.filter((t) => t.kind !== 'verify');
  const verifying = tasks.filter((t) => t.kind === 'verify');
  const lastBuild = project.status.lastBuild;
  const lastWrite = useWorkspace((s) => s.activity.filter((e) => e.kind === 'file_write' && e.status === 'completed').at(-1)?.ts);
  const buildStale = !!lastBuild && !!lastWrite && lastWrite > lastBuild.at;
  const label = previewLabel(preview);
  const previewOn = !!preview.runId || previewBusy(preview.status);
  const previewFailed = preview.status === 'failed' && !!preview.problem?.fixable;
  const buildFailed = lastBuild?.status === 'failed' && !buildStale;
  const blocker = running.find((t) => t.kind !== 'repair');
  const noPkg = 'This project has no package.json yet — ask Genius to create the app first.';
  const busyReason = blocker ? `Wait for “${blocker.title}” to finish (or stop it).` : undefined;

  const runOp = async (op: Operation) => {
    setMenu(false);
    setBusyOp(op);
    try {
      await api.startOperation(project.id, op);
      setWs({ bottomTab: 'terminal' });
      if (layout === 'simple') toast('info', `${op === 'install' ? 'Installing dependencies' : op === 'build' ? 'Running the production build' : 'Running the tests'} — see Activity for the result.`);
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusyOp(null);
    }
  };
  const diagnose = async () => {
    setMenu(false);
    try {
      await api.startDiagnose(project.id, null);
      setWs({ rightTab: 'assistant' });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  type Item = { label: string; icon: React.ReactNode; onClick: () => void; disabled?: string };
  const items: Item[] = [
    { label: 'Install dependencies', icon: <Package size={14} />, onClick: () => void runOp('install'), disabled: !hasPkg ? noPkg : busyOp ? 'Another command is starting.' : undefined },
    { label: 'Production build', icon: <Hammer size={14} />, onClick: () => void runOp('build'), disabled: !hasPkg ? noPkg : undefined },
    { label: 'Run tests', icon: <TestTube2 size={14} />, onClick: () => void runOp('test'), disabled: !hasPkg ? noPkg : undefined },
    { label: 'Build Doctor (diagnose last failure)', icon: <Stethoscope size={14} />, onClick: () => void diagnose(), disabled: !buildFailed ? 'Available after a build or test failed.' : busyReason },
    { label: 'Plan', icon: <ClipboardList size={14} />, onClick: () => (setMenu(false), setLayout('advanced'), openTab('plan')), disabled: !project.spec ? 'This project has no plan.' : undefined },
    { label: 'Export source', icon: <Download size={14} />, onClick: () => (setMenu(false), onDialog('export')) },
    { label: 'Publish to GitHub', icon: <UploadCloud size={14} />, onClick: () => (setMenu(false), onDialog('publish')), disabled: !hasPkg ? noPkg : undefined },
  ];

  return (
    <header className="ws-top">
      <span className="name ellipsis" title={project.path}>
        {project.name}
      </span>
      <span className={`badge ${label.tone}`} title={preview.message} aria-live="polite">
        {label.tone === 'run' && <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} aria-hidden />}
        {label.text}
      </span>
      {lastBuild && layout === 'advanced' && (
        <Badge tone={buildStale ? 'idle' : lastBuild.status === 'succeeded' ? 'ok' : lastBuild.status === 'failed' ? 'bad' : 'idle'} title={buildStale ? 'Files changed after this build; it no longer describes the current code.' : undefined}>
          build {lastBuild.status}
          {buildStale ? ' · out of date' : ''}
        </Badge>
      )}
      {running.map((t) => (
        <span key={t.id} className="badge run" style={{ gap: 6 }}>
          <span className="spinner" style={{ width: 10, height: 10, borderWidth: 1.5 }} />
          <span className="ellipsis" style={{ maxWidth: 220 }}>{t.title}</span>
          <button className="icon-btn" style={{ width: 16, height: 16 }} aria-label={`Stop ${t.title}`} title="Stop" onClick={() => void api.cancelTask(t.id)}>
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
      {(previewFailed || buildFailed) && !running.some((t) => t.kind === 'repair') && (
        <Button size="sm" variant="primary" icon={<Wand2 size={14} />} loading={actions.busy === 'fix'} disabled={!!blocker} title={busyReason ?? 'Genius reads the real error, makes the smallest fix and checks it again (up to 3 attempts)'} onClick={() => void actions.autoFix()}>
          Fix automatically
        </Button>
      )}
      <Button
        size="sm"
        variant={previewOn ? 'ghost' : 'ok'}
        icon={previewOn ? <Square size={13} /> : <Play size={13} />}
        disabled={!hasPkg || preview.status === 'stopping'}
        title={!hasPkg ? noPkg : undefined}
        loading={actions.busy === 'start' || actions.busy === 'stop'}
        onClick={() => void (previewOn ? actions.stop() : actions.start())}
      >
        {previewOn ? 'Stop preview' : 'Preview'}
      </Button>
      <div className="menu-wrap">
        <Button size="sm" variant="ghost" icon={<MoreHorizontal size={14} />} aria-haspopup="menu" aria-expanded={menu} onClick={() => setMenu(!menu)}>
          More <ChevronDown size={12} />
        </Button>
        {menu && (
          <>
            <div className="menu-backdrop" onClick={() => setMenu(false)} />
            <div className="menu" role="menu" onKeyDown={(e) => e.key === 'Escape' && setMenu(false)}>
              {items.map((i) => (
                <button key={i.label} role="menuitem" className="menu-item" disabled={!!i.disabled} title={i.disabled} onClick={i.onClick}>
                  {i.icon}
                  <span className="grow">{i.label}</span>
                  {i.disabled && <span className="tiny faint">unavailable</span>}
                </button>
              ))}
            </div>
          </>
        )}
      </div>
      <Button size="sm" variant="ghost" icon={layout === 'simple' ? <Code2 size={14} /> : <LayoutPanelLeft size={14} />} onClick={() => setLayout(layout === 'simple' ? 'advanced' : 'simple')} title={layout === 'simple' ? 'Show files, editor, terminal, Git and other tools' : 'Back to the simple view: conversation and preview'}>
        {layout === 'simple' ? 'Show code & tools' : 'Simple view'}
      </Button>
    </header>
  );
}

// ------------------------------------------------------------------ simple layout

function SimpleBody() {
  const project = useWorkspace((s) => s.project)!;
  const [lower, setLower] = useState<'changes' | 'activity'>('changes');
  const pending = useWorkspace((s) => s.activity.filter((e) => e.kind === 'changeset' && e.status === 'awaiting_approval').length);
  return (
    <div className="ws-simple">
      <PanelGroup direction="horizontal" autoSaveId="genius-simple-h">
        <Panel defaultSize={38} minSize={24} order={1}>
          <div className="panel" style={{ borderRight: '1px solid var(--border)' }}>
            <div className="panel-head">
              <span className="panel-title">Ask Genius</span>
              <span className="tiny faint ellipsis">
                working on <strong>{project.name}</strong>
              </span>
            </div>
            <AssistantPanel />
          </div>
        </Panel>
        <PanelResizeHandle className="resize-h" />
        <Panel defaultSize={62} minSize={30} order={2}>
          <PanelGroup direction="vertical" autoSaveId="genius-simple-v">
            <Panel defaultSize={72} minSize={30}>
              <div className="panel">
                <PreviewPanel />
              </div>
            </Panel>
            <PanelResizeHandle className="resize-v" />
            <Panel defaultSize={28} minSize={10} collapsible collapsedSize={0}>
              <div className="panel" style={{ borderTop: '1px solid var(--border)' }}>
                <div className="panel-head">
                  <div className="subtabs" role="tablist">
                    <button role="tab" aria-selected={lower === 'changes'} className={lower === 'changes' ? 'on' : ''} onClick={() => setLower('changes')}>
                      <FileDiff size={13} /> Changes {pending > 0 && <Badge tone="warn">{pending}</Badge>}
                    </button>
                    <button role="tab" aria-selected={lower === 'activity'} className={lower === 'activity' ? 'on' : ''} onClick={() => setLower('activity')}>
                      <ActivityIcon size={13} /> Activity
                    </button>
                  </div>
                </div>
                <div style={{ flex: 1, minHeight: 0 }}>{lower === 'changes' ? <ChangesPanel /> : <ActivitySummary />}</div>
              </div>
            </Panel>
          </PanelGroup>
        </Panel>
      </PanelGroup>
    </div>
  );
}

/** What Genius is doing, in plain words, from real events. The full log is under Show code & tools. */
function ActivitySummary() {
  const activity = useWorkspace((s) => s.activity);
  const setLayout = useWorkspace((s) => s.setLayout);
  const set = useWorkspace((s) => s.set);
  const items = summarizeActivity(activity);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.scrollTop = ref.current.scrollHeight;
  }, [items.length]);
  return (
    <div className="scroll feed" ref={ref} style={{ height: '100%' }}>
      {items.length === 0 && <Empty icon={<CircleDashed size={18} />} title="Nothing yet">What Genius does — reading files, installing, building, starting the preview — appears here as it happens.</Empty>}
      {items.map((e) => (
        <div key={e.id} className="ev" title={e.files?.join('\n')}>
          <span className="ic" style={{ color: e.status === 'failed' ? 'var(--bad)' : e.status === 'awaiting_approval' ? 'var(--warn)' : undefined }}>
            {KIND_ICON[e.kind] ?? <HelpCircle size={14} />}
          </span>
          <div style={{ minWidth: 0 }}>
            <div className="t">{e.title}</div>
            <div className="m row" style={{ gap: 6 }}>
              <Dot status={e.status} /> {STATUS_LABEL[e.status] ?? e.status}
            </div>
          </div>
          <span className="time">{clockTime(e.ts)}</span>
        </div>
      ))}
      {items.length > 0 && (
        <div style={{ padding: 8 }}>
          <Button size="sm" variant="ghost" onClick={() => (set({ bottomTab: 'activity' }), setLayout('advanced'))}>
            Show the full technical log
          </Button>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------ advanced layout

function AdvancedBody({ onPublish }: { onPublish: () => void }) {
  const preview = useWorkspace((s) => s.preview);
  const tasks = useWorkspace((s) => s.tasks);
  const rightTab = useWorkspace((s) => s.rightTab);
  const setWs = useWorkspace((s) => s.set);
  const [side, setSide] = useState<Side>('files');
  const sideRef = useRef<ImperativePanelHandle>(null);
  const bottomRef = useRef<ImperativePanelHandle>(null);
  const running = tasks.filter((t) => t.kind !== 'verify');

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      const p = e.key === 'b' ? sideRef.current : e.key === 'j' ? bottomRef.current : null;
      if (!p) return;
      e.preventDefault();
      if (p.isCollapsed()) p.expand();
      else p.collapse();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const railItems: { id: Side; label: string; icon: React.ReactNode }[] = [
    { id: 'files', label: 'Files', icon: <FolderTree size={18} /> },
    { id: 'search', label: 'Search', icon: <Search size={18} /> },
    { id: 'git', label: 'Source control', icon: <GitBranch size={18} /> },
    { id: 'history', label: 'History & recovery', icon: <HistoryIcon size={18} /> },
    { id: 'memory', label: 'Project memory', icon: <Brain size={18} /> },
    { id: 'references', label: 'Imported files', icon: <FileStack size={18} /> },
    { id: 'integrations', label: 'Integrations', icon: <Plug size={18} /> },
  ];
  const dot = preview.status === 'ready' ? 'ok' : previewBusy(preview.status) ? 'run' : preview.status === 'failed' ? 'bad' : '';

  return (
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
            {side === 'git' && <GitPanel onPublish={onPublish} />}
            {side === 'history' && <HistoryPanel />}
            {side === 'memory' && <MemoryPanel />}
            {side === 'references' && <ReferencesPanel />}
            {side === 'integrations' && <IntegrationsPanel onPublish={onPublish} />}
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
                  Preview <span className={`dot ${dot}`} />
                </button>
              </div>
            </div>
            <div style={{ flex: 1, minHeight: 0, display: rightTab === 'assistant' ? 'flex' : 'none', flexDirection: 'column' }}>
              <AssistantPanel />
            </div>
            <div style={{ flex: 1, minHeight: 0, display: rightTab === 'preview' ? 'flex' : 'none', flexDirection: 'column' }}>
              <PreviewPanel />
            </div>
          </div>
        </Panel>
      </PanelGroup>
    </div>
  );
}
