// Command palette (Ctrl/Cmd+K or Ctrl/Cmd+Shift+P): a keyboard-first launcher for every
// action in Genius. Global commands live here; views add their own via lib/commands.
import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ArrowDownCircle, Bell, BellOff, FileCode2, FolderOpen, Home as HomeIcon, Layers, LayoutPanelLeft, Moon, Plus, Search, Settings as SettingsIcon, Sun } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { rankCommands, useRegisteredCommands, type PaletteCommand } from '../lib/commands';
import { notificationsEnabled, setNotificationsEnabled } from '../lib/notify';
import type { Project } from '../lib/types';
import { useApp, type SettingsTab } from '../store/app';
import { useWorkspace } from '../store/workspace';

/** Section order when nothing is typed. */
const GROUP_ORDER = ['Project', 'Navigation', 'Recent projects', 'Files', 'Preferences', 'Settings'];

const SETTINGS_TABS: { tab: SettingsTab; label: string; keywords: string[] }[] = [
  { tab: 'ai', label: 'AI provider', keywords: ['model', 'api key', 'nvidia'] },
  { tab: 'github', label: 'GitHub', keywords: ['account', 'login'] },
  { tab: 'integrations', label: 'Integrations', keywords: ['supabase', 'database'] },
  { tab: 'approvals', label: 'Approvals', keywords: ['permissions', 'policy'] },
  { tab: 'workspace', label: 'Workspace', keywords: ['folder', 'tools', 'node'] },
  { tab: 'appearance', label: 'Appearance', keywords: ['theme', 'look'] },
  { tab: 'updates', label: 'Updates', keywords: ['version', 'release'] },
];

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
/** "Ctrl+K" → "⌘K" on macOS. */
export function shortcutLabel(s: string): string {
  return isMac ? s.replace(/Ctrl\+/g, '⌘').replace(/Shift\+/g, '⇧').replace(/Alt\+/g, '⌥') : s;
}

export function isPaletteShortcut(e: KeyboardEvent): boolean {
  const mod = e.ctrlKey || e.metaKey;
  if (!mod || e.altKey) return false;
  const k = e.key.toLowerCase();
  return (k === 'k' && !e.shiftKey) || (k === 'p' && e.shiftKey);
}

/** Mounted once in App: owns the global shortcut and the open state. */
export default function CommandPalette() {
  const [open, setOpen] = useState(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isPaletteShortcut(e)) return;
      e.preventDefault();
      e.stopPropagation();
      setOpen((o) => !o);
    };
    const onOpen = () => setOpen(true);
    // Capture phase: the editor and other panels must not swallow the shortcut.
    window.addEventListener('keydown', onKey, true);
    window.addEventListener(OPEN_EVENT, onOpen);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener(OPEN_EVENT, onOpen);
    };
  }, []);
  return open ? <Palette onClose={() => setOpen(false)} /> : null;
}

const OPEN_EVENT = 'genius:open-command-palette';
/** Opens the palette from anywhere (e.g. a toolbar button). */
export function openCommandPalette() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

function useCoreCommands(projects: Project[]): PaletteCommand[] {
  const route = useApp((s) => s.route);
  const navigate = useApp((s) => s.navigate);
  const settings = useApp((s) => s.settings);
  const setUiMode = useApp((s) => s.setUiMode);
  const setTheme = useApp((s) => s.setTheme);
  const checkUpdate = useApp((s) => s.checkUpdate);
  const setShowUpdate = useApp((s) => s.setShowUpdate);
  const toast = useApp((s) => s.toast);
  const wsProject = useWorkspace((s) => s.project);
  const files = useWorkspace((s) => s.files);
  const openFile = useWorkspace((s) => s.openFile);
  const setWs = useWorkspace((s) => s.set);
  const [notify, setNotify] = useState(notificationsEnabled);

  return useMemo(() => {
    const out: PaletteCommand[] = [];
    const nav = (c: Omit<PaletteCommand, 'group'>) => out.push({ group: 'Navigation', ...c });
    nav({ id: 'nav.home', title: 'Go Home', keywords: ['start', 'projects', 'dashboard'], icon: <HomeIcon size={15} />, run: () => navigate({ name: 'home' }) });
    nav({ id: 'nav.new', title: 'New project', keywords: ['create', 'app', 'start'], shortcut: 'Ctrl+Shift+N', icon: <Plus size={15} />, run: () => navigate({ name: 'create' }) });
    nav({ id: 'nav.settings', title: 'Settings', keywords: ['preferences', 'options'], shortcut: 'Ctrl+,', icon: <SettingsIcon size={15} />, run: () => navigate({ name: 'settings' }) });
    if (wsProject && route.name !== 'project') {
      nav({ id: 'nav.workspace', title: `Back to ${wsProject.name}`, keywords: ['workspace', 'project'], icon: <Layers size={15} />, run: () => navigate({ name: 'project', id: wsProject.id }) });
    }

    for (const p of projects) {
      out.push({
        id: `project.${p.id}`,
        group: 'Recent projects',
        title: `Open project ${p.name}`,
        subtitle: p.description || p.path,
        keywords: [p.name, p.path],
        icon: <FolderOpen size={15} />,
        run: () => navigate({ name: 'project', id: p.id }),
      });
    }

    if (wsProject) {
      for (const f of files) {
        if (!f.isText) continue;
        out.push({
          id: `file.${f.path}`,
          group: 'Files',
          title: f.path,
          subtitle: 'Open file',
          keywords: [`open file ${f.path}`],
          searchOnly: true,
          icon: <FileCode2 size={15} />,
          run: () => {
            if (route.name !== 'project') navigate({ name: 'project', id: wsProject.id });
            if (settings?.uiMode !== 'advanced') setWs({ stage: 'code' });
            void openFile(f.path);
          },
        });
      }
    }

    const pref = (c: Omit<PaletteCommand, 'group'>) => out.push({ group: 'Preferences', ...c });
    if (settings) {
      const next = settings.uiMode === 'advanced' ? 'simple' : 'advanced';
      pref({
        id: 'pref.mode',
        title: next === 'advanced' ? 'Switch to Advanced mode' : 'Switch to Simple mode',
        subtitle: next === 'advanced' ? 'Files, terminal, Git and every tool' : 'Chat and preview only',
        keywords: ['interface', 'ide', 'mode', 'layout'],
        icon: <LayoutPanelLeft size={15} />,
        run: () => setUiMode(next),
      });
      const resolved = document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
      pref({
        id: 'pref.theme',
        title: 'Toggle theme',
        subtitle: `Switch to ${resolved === 'dark' ? 'light' : 'dark'}`,
        keywords: ['dark', 'light', 'appearance', 'colors'],
        icon: resolved === 'dark' ? <Sun size={15} /> : <Moon size={15} />,
        run: () => setTheme(resolved === 'dark' ? 'light' : 'dark'),
      });
    }
    pref({
      id: 'pref.updates',
      title: 'Check for updates',
      keywords: ['version', 'upgrade', 'release'],
      icon: <ArrowDownCircle size={15} />,
      run: async () => {
        const u = await checkUpdate(true);
        if (u?.available) setShowUpdate(true);
      },
    });
    pref({
      id: 'pref.notifications',
      title: notify ? 'Turn notifications off' : 'Turn notifications on',
      subtitle: 'When Genius finishes or needs you while in the background',
      keywords: ['notify', 'alerts', 'sound', 'done', 'finished'],
      icon: notify ? <BellOff size={15} /> : <Bell size={15} />,
      run: () => {
        setNotificationsEnabled(!notify);
        setNotify(!notify);
        toast('ok', notify ? 'Notifications are off.' : 'Notifications are on. Genius will let you know when long tasks finish while you are away.');
      },
    });

    for (const t of SETTINGS_TABS) {
      out.push({ id: `settings.${t.tab}`, group: 'Settings', title: `Settings: ${t.label}`, keywords: t.keywords, searchOnly: true, icon: <SettingsIcon size={15} />, run: () => navigate({ name: 'settings', tab: t.tab }) });
    }
    return out;
  }, [route.name, navigate, settings, setUiMode, setTheme, checkUpdate, setShowUpdate, toast, wsProject, files, openFile, setWs, notify, projects]);
}

interface Section {
  group: string;
  items: { command: PaletteCommand; indices: number[]; index: number }[];
}

function Palette({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [query, setQuery] = useState('');
  const [sel, setSel] = useState(0);
  const [projects, setProjects] = useState<Project[]>([]);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let alive = true;
    api
      .listProjects()
      .then((p) => {
        if (alive) setProjects(p);
      })
      .catch(() => {
        /* the palette works without the project list */
      });
    return () => {
      alive = false;
    };
  }, []);

  // Focus the search box; give focus back to where it was on close.
  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    inputRef.current?.focus();
    return () => prev?.focus?.();
  }, []);

  const registered = useRegisteredCommands();
  const core = useCoreCommands(projects);

  const sections = useMemo<Section[]>(() => {
    const ranked = rankCommands([...registered, ...core], query);
    const order = query.trim() ? [...new Set(ranked.map((r) => r.command.group ?? 'Commands'))] : [...GROUP_ORDER, ...new Set(ranked.map((r) => r.command.group ?? 'Commands'))];
    const byGroup = new Map<string, Section>();
    for (const r of ranked) {
      const g = r.command.group ?? 'Commands';
      if (!byGroup.has(g)) byGroup.set(g, { group: g, items: [] });
      byGroup.get(g)!.items.push({ command: r.command, indices: r.indices, index: 0 });
    }
    const out = [...new Set(order)].map((g) => byGroup.get(g)).filter((s): s is Section => !!s);
    let i = 0;
    for (const s of out) for (const it of s.items) it.index = i++;
    return out;
  }, [registered, core, query]);
  const flat = sections.flatMap((s) => s.items);
  const active = Math.min(sel, Math.max(flat.length - 1, 0));

  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const run = (c: PaletteCommand) => {
    onClose();
    // Run after the palette has closed so focus and dialogs opened by the command behave.
    setTimeout(() => {
      Promise.resolve()
        .then(() => c.run())
        .catch((e) => toast('bad', errorMessage(e)));
    }, 0);
  };

  const onKeyDown = (e: React.KeyboardEvent) => {
    const n = flat.length;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (n) setSel((active + 1) % n);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (n) setSel((active - 1 + n) % n);
    } else if (e.key === 'PageDown') {
      e.preventDefault();
      setSel(Math.min(active + 8, n - 1));
    } else if (e.key === 'PageUp') {
      e.preventDefault();
      setSel(Math.max(active - 8, 0));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (flat[active]) run(flat[active].command);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === 'Tab') {
      e.preventDefault(); // focus stays in the search box
    }
  };

  const optionId = (i: number) => `palette-opt-${i}`;
  return (
    <div className="backdrop palette-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="palette" role="dialog" aria-modal="true" aria-label="Command palette" onKeyDown={onKeyDown}>
        <div className="palette-search">
          <Search size={16} aria-hidden />
          <input
            ref={inputRef}
            role="combobox"
            aria-expanded="true"
            aria-controls="palette-list"
            aria-autocomplete="list"
            aria-activedescendant={flat.length ? optionId(active) : undefined}
            aria-label="Search commands"
            placeholder="Type a command, project or file…"
            value={query}
            spellCheck={false}
            autoComplete="off"
            onChange={(e) => {
              setQuery(e.target.value);
              setSel(0);
            }}
          />
          <kbd className="kbd">Esc</kbd>
        </div>
        <div className="palette-list" id="palette-list" role="listbox" aria-label="Commands" ref={listRef}>
          {flat.length === 0 && <div className="palette-empty">No commands match “{query}”.</div>}
          {sections.map((s) => (
            <div key={s.group} role="group" aria-labelledby={`palette-g-${slug(s.group)}`}>
              <div className="palette-group" id={`palette-g-${slug(s.group)}`} role="presentation">
                {s.group}
              </div>
              {s.items.map(({ command: c, indices, index }) => (
                <div
                  key={c.id}
                  id={optionId(index)}
                  data-index={index}
                  role="option"
                  aria-selected={index === active}
                  className={`palette-item ${index === active ? 'on' : ''}`}
                  onMouseMove={() => index !== active && setSel(index)}
                  onClick={() => run(c)}
                >
                  <span className="palette-icon" aria-hidden>
                    {c.icon}
                  </span>
                  <span className="palette-title ellipsis">
                    <Highlight text={c.title} indices={indices} />
                    {c.subtitle && <span className="palette-sub"> {c.subtitle}</span>}
                  </span>
                  {c.shortcut && <kbd className="kbd">{shortcutLabel(c.shortcut)}</kbd>}
                </div>
              ))}
            </div>
          ))}
        </div>
        <div className="palette-foot" aria-hidden>
          <span>
            <kbd className="kbd">↑</kbd>
            <kbd className="kbd">↓</kbd> navigate
          </span>
          <span>
            <kbd className="kbd">↵</kbd> run
          </span>
          <span>
            <kbd className="kbd">Esc</kbd> close
          </span>
          <span className="spacer" />
          <span>
            <kbd className="kbd">{shortcutLabel('Ctrl+K')}</kbd> anywhere
          </span>
        </div>
      </div>
    </div>
  );
}

function slug(s: string) {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

function Highlight({ text, indices }: { text: string; indices: number[] }) {
  if (!indices.length) return <>{text}</>;
  const set = new Set(indices);
  const parts: ReactNode[] = [];
  let buf = '';
  let marked = false;
  const flush = (i: number) => {
    if (!buf) return;
    parts.push(marked ? <mark key={i}>{buf}</mark> : <Fragment key={i}>{buf}</Fragment>);
    buf = '';
  };
  for (let i = 0; i < text.length; i++) {
    const m = set.has(i);
    if (m !== marked) {
      flush(i);
      marked = m;
    }
    buf += text[i];
  }
  flush(text.length);
  return <>{parts}</>;
}
