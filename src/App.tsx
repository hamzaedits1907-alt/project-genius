import { Component, useEffect, type ReactNode } from 'react';
import { FolderKanban, Home as HomeIcon, Plus, Settings as SettingsIcon, Sparkles, X } from 'lucide-react';
import { useApp } from './store/app';
import { subscribeAll } from './store/events';
import Home from './views/Home';
import CreateProject from './views/CreateProject';
import SettingsView from './views/Settings';
import Workspace from './views/workspace/Workspace';
import ApprovalDialog from './views/ApprovalDialog';
import { useWorkspace } from './store/workspace';

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error) {
    console.error('UI error', error);
  }
  render() {
    if (this.state.error) {
      return (
        <div className="empty">
          <h3>Something went wrong in this view</h3>
          <p className="mono small">{this.state.error.message}</p>
          <p>Your projects and files are safe — this is a display error only.</p>
          <button className="btn" onClick={() => this.setState({ error: null })}>
            Try again
          </button>
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

  useEffect(() => {
    let off: (() => void) | undefined;
    void subscribeAll().then((u) => (off = u));
    void loadSettings();
    void refreshApprovals();
    return () => off?.();
  }, [loadSettings, refreshApprovals]);

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
        <div className="logo" aria-hidden>
          <Sparkles size={17} />
        </div>
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
        <button className={route.name === 'settings' ? 'on' : ''} onClick={() => navigate({ name: 'settings' })} title="Settings (Ctrl+,)" aria-label="Settings">
          <SettingsIcon size={19} />
        </button>
      </nav>
      <main className="main">
        <ErrorBoundary key={route.name}>
          {route.name === 'home' && <Home />}
          {route.name === 'create' && <CreateProject />}
          {route.name === 'settings' && <SettingsView initialTab={route.tab} />}
          {route.name === 'project' && <Workspace projectId={route.id} />}
        </ErrorBoundary>
      </main>
      {approvals.length > 0 && <ApprovalDialog request={approvals[0]} remaining={approvals.length - 1} />}
      <Toasts />
    </div>
  );
}
