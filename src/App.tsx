import { Component, useEffect, useState, type ErrorInfo, type ReactNode } from 'react';
import { Menu, SquarePen, X } from 'lucide-react';
import ChatHome from './views/chat/ChatHome';
import ResearchView from './views/chat/ResearchView';
import { subscribeChat } from './store/chat';
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
import AgentsView from './views/agents/AgentsView';
import ProjectsView from './views/ProjectsView';
import { extraViews } from './lib/extraViews';
import { ShortcutsDialog } from './components/ShortcutsDialog';
import CommandPalette from './components/CommandPalette';
import { Sidebar, startNewChat } from './components/Sidebar';

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
  const collapsed = useApp((s) => s.sidebarCollapsed);
  const setCollapsed = useApp((s) => s.setSidebarCollapsed);
  const showUpdate = useApp((s) => s.showUpdate);
  const setShowUpdate = useApp((s) => s.setShowUpdate);
  const [shortcuts, setShortcuts] = useState(false);

  // Backend events: released even if the app unmounts before `listen` resolves (StrictMode
  // mounts twice in development; the old code leaked the first set and delivered every event twice).
  useSubscription('backend events', subscribeAll, []);
  useSubscription('chat events', subscribeChat, []);
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
      } else if (mod && e.shiftKey && (e.key === 'O' || e.key === 'o')) {
        e.preventDefault();
        startNewChat(useApp.getState().route.name === 'research' ? 'research' : 'chat');
      } else if (mod && e.key === '/') {
        e.preventDefault();
        setShortcuts((v) => !v);
      } else if (mod && !e.shiftKey && (e.key === 'b' || e.key === 'B')) {
        e.preventDefault();
        setCollapsed(!useApp.getState().sidebarCollapsed);
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [navigate, setCollapsed]);

  return (
    <div className={`shell gx-shell ${collapsed ? 'collapsed' : ''}`}>
      {/* phones: a top bar opens the sidebar as a drawer (hidden by CSS on larger screens) */}
      <div className="gx-mobilebar">
        <button aria-label="Open menu" onClick={() => setCollapsed(false)}><Menu size={20} /></button>
        <span className="brand">Genius</span>
        <button aria-label="New chat" onClick={() => startNewChat(useApp.getState().route.name === 'research' ? 'research' : 'chat')}><SquarePen size={19} /></button>
      </div>
      {!collapsed && <div className="gx-scrim" onClick={() => setCollapsed(true)} aria-hidden />}
      <Sidebar />
      <main className="main">
        <ErrorBoundary key={route.name} view={route.name}>
          {route.name === 'home' && <Home />}
          {route.name === 'create' && <CreateProject />}
          {route.name === 'chat' && <ChatHome />}
          {route.name === 'research' && <ResearchView />}
          {route.name === 'settings' && <SettingsView initialTab={route.tab} />}
          {route.name === 'project' && <Workspace projectId={route.id} />}
          {route.name === 'agents' && <AgentsView initialTab={route.tab} />}
          {route.name === 'projects' && <ProjectsView />}
          {extraViews[route.name] && (() => { const V = extraViews[route.name]; return <V />; })()}
        </ErrorBoundary>
      </main>
      {approvals.length > 0 && <ApprovalDialog request={approvals[0]} remaining={approvals.length - 1} />}
      {showUpdate && <UpdateDialog onClose={() => setShowUpdate(false)} />}
      <CommandPalette />
      {shortcuts && <ShortcutsDialog onClose={() => setShortcuts(false)} />}
      <Toasts />
    </div>
  );
}
