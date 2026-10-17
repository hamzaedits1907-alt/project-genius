import { Component, useEffect, type ErrorInfo, type ReactNode } from 'react';
import { ArrowDownCircle, FolderKanban, Home as HomeIcon, Plus, Settings as SettingsIcon, X } from 'lucide-react';
import { useApp } from './store/app';
import { subscribeAll } from './store/events';
import { useSubscription } from './lib/effects';
import { api } from './lib/api';
import { firstComponent } from './lib/logic';
import Home from './views/Home';
import CreateProject from './views/CreateProject';
import SettingsView from './views/Settings';
import Workspace from './views/workspace/Workspace';
import ApprovalDialog from './views/ApprovalDialog';
import UpdateDialog from './views/UpdateDialog';
import { GeniusAvatar } from './components/GeniusAvatar';
import { useWorkspace } from './store/workspace';

class ErrorBoundary extends Component<{ children: ReactNode; view: string }, { error: Error | null; componentStack: string; logPath: string | null }> {
  state = { error: null as Error | null, componentStack: '', logPath: null as string | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: ErrorInfo) {
    const componentStack = info.componentStack ?? '';
    console.error('UI error', error, componentStack);
    this.setState({ componentStack });
    // Persist (redacted) so the failing component can be identified without devtools.
    api
      .recordUiError({ view: this.props.view, message: String(error?.message ?? error), stack: String(error?.stack ?? ''), componentStack })
      .then((logPath) => this.setState({ logPath }))
      .catch((e) => console.error('could not record UI error', e));
  }
  details() {
    const { error, componentStack } = this.state;
    return `View: ${this.props.view}\nError: ${error?.message}\n\nStack:\n${error?.stack ?? ''}\n\nComponents:${componentStack}`;
  }
  render() {
    const { error, componentStack, logPath } = this.state;
    if (error) {
      const failing = firstComponent(componentStack);
      return (
        <div className="empty" role="alert">
          <h3>Something went wrong in this view</h3>
          <p className="mono small">{error.message}</p>
          {failing && <p className="small">Failing component: <span className="mono">{failing}</span></p>}
          <p>Your projects and files are safe — this is a display error only.</p>
          {logPath && <p className="small muted">Details were saved to <span className="mono">{logPath}</span></p>}
          <div className="row" style={{ justifyContent: 'center' }}>
            <button className="btn" onClick={() => this.setState({ error: null, componentStack: '', logPath: null })}>
              Try again
            </button>
            <button className="btn ghost" onClick={() => void navigator.clipboard?.writeText(this.details())}>
              Copy details
            </button>
          </div>
        </div>
      );
    }
    return this.props.children;
  }
}

function Toasts() {
  const toasts = useApp((s) => s.toasts);
  const dismiss = useApp((s) => s.dismissToast);
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.tone}`}>
          <div className="grow" style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>
            {t.text}
          </div>
          <button className="icon-btn" style={{ width: 20, height: 20 }} aria-label="Dismiss" onClick={() => dismiss(t.id)}>
            <X size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}

export default function App() {
  const route = useApp((s) => s.route);
  const navigate = useApp((s) => s.navigate);
  const loadSettings = useApp((s) => s.loadSettings);
  const refreshApprovals = useApp((s) => s.refreshApprovals);
  const approvals = useApp((s) => s.approvals);
  const openProjectId = useWorkspace((s) => s.projectId);
  const openProjectName = useWorkspace((s) => s.project?.name);
  const update = useApp((s) => s.update);
  const showUpdate = useApp((s) => s.showUpdate);
  const setShowUpdate = useApp((s) => s.setShowUpdate);
  const busy = useWorkspace((s) => s.tasks.some((t) => t.kind !== 'verify'));
  const skipped = useApp((s) => s.skippedVersion);

  // Backend events: released even if the app unmounts before `listen` resolves (StrictMode
  // mounts twice in development; the old code leaked the first set and delivered every event twice).
  useSubscription('backend events', subscribeAll, []);
  useEffect(() => {
    void loadSettings();
    void refreshApprovals();
  }, [loadSettings, refreshApprovals]);

  // Long sessions: look for new releases every 6 hours as well.
  useEffect(() => {
    const t = setInterval(() => {
      if (useApp.getState().settings?.autoCheckUpdates) void useApp.getState().checkUpdate(false);
    }, 6 * 60 * 60 * 1000);
    return () => clearInterval(t);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key === ',') {
        e.preventDefault();
        navigate({ name: 'settings' });
      } else if (mod && e.shiftKey && (e.key === 'N' || e.key === 'n')) {
        e.preventDefault();
        navigate({ name: 'create' });
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [navigate]);

  return (
    <div className="shell">
      <nav className="rail" aria-label="Main navigation">
        <button className="logo" onClick={() => navigate({ name: 'home' })} title="Genius" aria-label="Genius home">
          <GeniusAvatar size={34} mood={busy ? 'working' : 'idle'} />
        </button>
        <button className={route.name === 'home' ? 'on' : ''} onClick={() => navigate({ name: 'home' })} title="Home" aria-label="Home">
          <HomeIcon size={19} />
        </button>
        <button className={route.name === 'create' ? 'on' : ''} onClick={() => navigate({ name: 'create' })} title="New project (Ctrl+Shift+N)" aria-label="New project">
          <Plus size={19} />
        </button>
        {openProjectId && (
          <button className={route.name === 'project' ? 'on' : ''} onClick={() => navigate({ name: 'project', id: openProjectId })} title={`Workspace: ${openProjectName ?? ''}`} aria-label="Open workspace">
            <FolderKanban size={19} />
            {approvals.length > 0 && <span className="count">{approvals.length}</span>}
          </button>
        )}
        <div className="spacer" />
        {update?.available && update.latestVersion !== skipped && (
          <button className="update-dot" onClick={() => setShowUpdate(true)} title={`Update available: ${update.latestVersion}`} aria-label="Update available">
            <ArrowDownCircle size={19} />
            <span className="count ok" />
          </button>
        )}
        <button className={route.name === 'settings' ? 'on' : ''} onClick={() => navigate({ name: 'settings' })} title="Settings (Ctrl+,)" aria-label="Settings">
          <SettingsIcon size={19} />
        </button>
      </nav>
      <main className="main">
        <ErrorBoundary key={route.name} view={route.name}>
          {route.name === 'home' && <Home />}
          {route.name === 'create' && <CreateProject />}
          {route.name === 'settings' && <SettingsView initialTab={route.tab} />}
          {route.name === 'project' && <Workspace projectId={route.id} />}
        </ErrorBoundary>
      </main>
      {approvals.length > 0 && <ApprovalDialog request={approvals[0]} remaining={approvals.length - 1} />}
      {showUpdate && <UpdateDialog onClose={() => setShowUpdate(false)} />}
      <Toasts />
    </div>
  );
}
