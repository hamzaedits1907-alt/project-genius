import { useEffect, useMemo, useState } from 'react';
import { ArrowDownCircle, Bot, Code2, Command, FolderKanban, FolderOpen, Globe2, MessageSquare, PanelLeftClose, PanelLeftOpen, Paperclip, Pencil, Search, Settings as SettingsIcon, SquarePen, Trash2 } from 'lucide-react';
import { api } from '../lib/api';
import { groupByRecency } from '../lib/chat';
import type { ChatConversation, ChatMode, Project } from '../lib/types';
import { pendingApprovalTasks } from '../lib/agents';
import { useAgents } from '../store/agents';
import { useApp } from '../store/app';
import { useChat } from '../store/chat';
import { useWorkspace } from '../store/workspace';
import { GeniusAvatar } from './GeniusAvatar';
import { openCommandPalette, shortcutLabel } from './CommandPalette';
import { FilesDialog } from '../views/chat/FilesDialog';
import { capabilities } from '../lib/platform';
import { AccountBlock } from './AccountBlock';

/** Start a blank conversation in `mode` and show it (nothing is created until the first message is sent). */
export function startNewChat(mode: ChatMode = 'chat') {
  useChat.getState().fresh(mode);
  useApp.getState().navigate({ name: mode });
}

/**
 * The app's left sidebar: new chat, workspaces (Chat, Research, Code, Agents), searchable history
 * grouped by date, recent projects, saved files, and settings. Collapses to an icon rail.
 */
export function Sidebar() {
  const route = useApp((s) => s.route);
  const navigate = useApp((s) => s.navigate);
  const collapsed = useApp((s) => s.sidebarCollapsed);
  const setCollapsed = useApp((s) => s.setSidebarCollapsed);
  const update = useApp((s) => s.update);
  const skipped = useApp((s) => s.skippedVersion);
  const setShowUpdate = useApp((s) => s.setShowUpdate);
  const approvals = useApp((s) => s.approvals);
  const agentApprovals = useAgents((s) => pendingApprovalTasks(s.missions).length);
  const projectBusy = useWorkspace((s) => s.tasks.some((t) => t.kind !== 'verify'));
  const openProjectId = useWorkspace((s) => s.projectId);
  const all = useChat((s) => s.all);
  const query = useChat((s) => s.query);
  const active = useChat((s) => s.active);
  const running = useChat((s) => s.tasks.some((t) => t.kind === 'chat' || t.kind === 'research'));
  const [projects, setProjects] = useState<Project[]>([]);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [title, setTitle] = useState('');
  const [files, setFiles] = useState(false);

  useEffect(() => {
    void useChat.getState().loadAll();
  }, []);
  useEffect(() => {
    if (!capabilities.localProjects) return;
    api.listProjects().then((p) => setProjects([...p].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)).slice(0, 5))).catch(() => undefined);
  }, [route.name, openProjectId]);

  // Narrow windows start collapsed; the user's explicit choice still wins afterwards.
  useEffect(() => {
    const mq = window.matchMedia?.('(max-width: 820px)');
    if (mq?.matches && localStorageGet('genius.sidebarCollapsed') === null) setCollapsed(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const groups = useMemo(() => groupByRecency(all), [all]);
  const open = (c: ChatConversation) => {
    navigate(c.origin?.flavor === 'code' ? { name: 'code' } : { name: c.mode });
    void useChat.getState().select(c.mode, c.id);
  };
  const activeId = route.name === 'chat' ? active.chat : route.name === 'research' ? active.research : null;
  const mode = route.name === 'research' ? 'research' : 'chat';

  const nav = [
    { key: 'chat', label: 'Chat', icon: <MessageSquare size={16} />, on: route.name === 'chat', go: () => navigate({ name: 'chat' }) },
    { key: 'research', label: 'Research', icon: <Globe2 size={16} />, on: route.name === 'research', go: () => navigate({ name: 'research' }) },
    { key: 'code', label: 'Code', icon: <Code2 size={16} />, on: ['home', 'create', 'project', 'code'].includes(route.name), go: () => navigate(capabilities.localProjects ? { name: 'home' } : { name: 'code' }), badge: approvals.length },
    { key: 'agents', label: 'Agents', icon: <Bot size={16} />, on: route.name === 'agents', go: () => navigate({ name: 'agents' }), badge: agentApprovals },
    { key: 'projects', label: 'Projects', icon: <FolderKanban size={16} />, on: route.name === 'projects', go: () => navigate({ name: 'projects' }) },
  ];
  const updateReady = !!update?.available && update.latestVersion !== skipped;

  if (collapsed) {
    return (
      <aside className="gx-side collapsed" aria-label="Sidebar">
        <button className="gx-logo" onClick={() => setCollapsed(false)} aria-label="Expand sidebar" title="Expand sidebar (Ctrl+B)">
          <GeniusAvatar size={30} mood={running || projectBusy ? 'working' : 'idle'} />
        </button>
        <button className="gx-rail-btn primary" onClick={() => startNewChat('chat')} aria-label="New chat" title="New chat (Ctrl+Shift+O)">
          <SquarePen size={17} />
        </button>
        {nav.map((n) => (
          <button key={n.key} className={`gx-rail-btn ${n.on ? 'on' : ''}`} onClick={n.go} aria-label={n.label} title={n.label} aria-current={n.on ? 'page' : undefined}>
            {n.icon}
            {!!n.badge && <span className="count">{n.badge}</span>}
          </button>
        ))}
        <span className="spacer" />
        <button className="gx-rail-btn" onClick={() => setCollapsed(false)} aria-label="Expand sidebar" title="Expand sidebar">
          <PanelLeftOpen size={17} />
        </button>
        <button className={`gx-rail-btn ${route.name === 'settings' ? 'on' : ''}`} onClick={() => navigate({ name: 'settings' })} aria-label="Settings" title="Settings (Ctrl+,)">
          <SettingsIcon size={17} />
        </button>
      </aside>
    );
  }

  return (
    <aside
      className="gx-side"
      aria-label="Sidebar"
      onClick={(e) => {
        // phones: choosing something closes the drawer (typing in search or the small row actions keep it open)
        const t = e.target as HTMLElement;
        if (window.matchMedia?.('(max-width: 640px)').matches && t.closest('.gx-nav button, .gx-new, li > button:not(.icon-btn), .gx-side-foot button') && !t.closest('input, textarea, .icon-btn')) setCollapsed(true);
      }}
    >
      <div className="gx-side-head">
        <GeniusAvatar size={26} mood={running || projectBusy ? 'working' : 'idle'} />
        <span className="gx-brand">Genius</span>
        <span className="spacer" />
        <button className="icon-btn" onClick={() => setCollapsed(true)} aria-label="Collapse sidebar" title="Collapse sidebar (Ctrl+B)">
          <PanelLeftClose size={16} />
        </button>
      </div>

      <button className="gx-new" onClick={() => startNewChat(mode)} aria-label="New chat" title="New chat (Ctrl+Shift+O)">
        <SquarePen size={15} /> New {mode === 'research' ? 'research' : 'chat'}
      </button>

      <nav className="gx-nav" aria-label="Workspaces">
        {nav.map((n) => (
          <button key={n.key} className={n.on ? 'on' : ''} onClick={n.go} aria-label={n.label} title={n.badge ? `${n.label}: ${n.badge} waiting for your approval` : undefined} aria-current={n.on ? 'page' : undefined}>
            {n.icon}
            <span>{n.label}</span>
            {!!n.badge && <span className="count" title="Waiting for your approval">{n.badge}</span>}
          </button>
        ))}
      </nav>

      <label className="gx-search">
        <Search size={13} />
        <input placeholder="Search conversations" aria-label="Search conversations" value={query} onChange={(e) => void useChat.getState().search(mode, e.target.value)} />
      </label>

      <div className="gx-scroll">
        <div className="gx-history" aria-label="Conversations">
          {groups.map((g) => (
            <section key={g.label} aria-label={g.label}>
              <h4>{g.label}</h4>
              <ul>
                {g.items.map((c) => (
                  <li key={c.id} className={c.id === activeId ? 'on' : ''}>
                    {renaming === c.id ? (
                      <input
                        className="input sm"
                        aria-label="Conversation title"
                        autoFocus
                        value={title}
                        onChange={(e) => setTitle(e.target.value)}
                        onBlur={() => setRenaming(null)}
                        onKeyDown={(e) => {
                          if (e.key === 'Escape') setRenaming(null);
                          if (e.key === 'Enter' && title.trim()) void useChat.getState().rename(c.mode, c.id, title).then(() => setRenaming(null));
                        }}
                      />
                    ) : (
                      <button className="gx-conv" onClick={() => open(c)} title={c.snippet || c.title} aria-current={c.id === activeId ? 'true' : undefined}>
                        {c.mode === 'research' && <Globe2 size={12} aria-label="Research" />}
                        <span className="ellipsis">{c.title}</span>
                      </button>
                    )}
                    <button className="icon-btn" aria-label={`Rename ${c.title}`} onClick={() => { setTitle(c.title); setRenaming(c.id); }}>
                      <Pencil size={12} />
                    </button>
                    <button
                      className="icon-btn"
                      aria-label={`Delete ${c.title}`}
                      disabled={running}
                      onClick={() => {
                        if (window.confirm(`Delete “${c.title}”? Its messages${c.mode === 'research' ? ' and sources ledger' : ''} are removed.`)) void useChat.getState().remove(c.mode, c.id);
                      }}
                    >
                      <Trash2 size={12} />
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
          {groups.length === 0 && <div className="faint small gx-empty">{query ? 'No conversation matches.' : 'Your conversations will appear here.'}</div>}
        </div>

        {projects.length > 0 && (
          <section className="gx-projects" aria-label="Projects">
            <h4>Projects</h4>
            <ul>
              {projects.map((p) => (
                <li key={p.id}>
                  <button className={`gx-conv ${route.name === 'project' && openProjectId === p.id ? 'on' : ''}`} onClick={() => navigate({ name: 'project', id: p.id })} title={p.description || p.name}>
                    <FolderOpen size={12} />
                    <span className="ellipsis">{p.name}</span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>

      <div className="gx-account-row"><AccountBlock /></div>
      <div className="gx-side-foot">
        <button onClick={() => setFiles(true)} aria-label="Saved files" title="Files you attached in Chat and Research">
          <Paperclip size={15} /> <span>Files</span>
        </button>
        <button onClick={openCommandPalette} title={`Command palette (${shortcutLabel('Ctrl+K')})`} aria-label="Command palette">
          <Command size={15} />
        </button>
        {updateReady && (
          <button className="update" onClick={() => setShowUpdate(true)} title={`Update available: ${update?.latestVersion}`} aria-label="Update available">
            <ArrowDownCircle size={15} />
          </button>
        )}
        <span className="spacer" />
        <button className={route.name === 'settings' ? 'on' : ''} onClick={() => navigate({ name: 'settings' })} aria-label="Settings" title="Settings (Ctrl+,)">
          <SettingsIcon size={15} /> <span>Settings</span>
        </button>
      </div>
      {files && <FilesDialog onClose={() => setFiles(false)} />}
    </aside>
  );
}

function localStorageGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
