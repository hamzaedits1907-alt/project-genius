import { useEffect, useState } from 'react';
import { ArrowUp, Check, FolderOpen, KeyRound, Plus, SlidersHorizontal, Sparkles, Trash2, Wrench, X, FileArchive } from 'lucide-react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { Github } from '../components/icons';
import { api, errorMessage } from '../lib/api';
import { timeAgo } from '../lib/logic';
import type { Project, RepoInfo, ToolStatus } from '../lib/types';
import { useApp } from '../store/app';
import { Badge, Button, Callout, Empty, Modal, Spinner, StatusBadge } from '../components/ui';
import { GeniusAvatar } from '../components/GeniusAvatar';
import { markAutoBuild } from '../store/workspace';

const IDEAS = [
  { label: 'Portfolio website', text: 'A personal portfolio website with a bold hero, project gallery with filters, about section, testimonials and a contact form. Modern, elegant, responsive.' },
  { label: 'Landing page', text: 'A landing page for a specialty coffee shop: hero with call to action, menu highlights, story section, opening hours, location and newsletter signup. Warm, inviting design.' },
  { label: 'Task manager', text: 'A modern task manager with projects, a kanban board with drag and drop, search, due dates and a clean responsive interface. Data saved in the browser.' },
  { label: 'Finance tracker', text: 'A personal finance tracker: add income and expenses, categories, monthly charts, and data saved in the browser.' },
  { label: 'Recipe book', text: 'A recipe book where I can add recipes with ingredients and steps, tag them, and filter by tag. Beautiful food-magazine style.' },
];

/** A short project name from a description: "A landing page for a coffee shop" → "Coffee Shop Landing". */
export function nameFromIdea(text: string): string {
  const stop = new Set(['a', 'an', 'the', 'for', 'with', 'and', 'my', 'me', 'build', 'make', 'create', 'i', 'want', 'to', 'of', 'that', 'where', 'can', 'app', 'application', 'website', 'site', 'page', 'please', 'in', 'on', 'which', 'is', 'it']);
  const words = text
    .replace(/[^\p{L}\p{N}\s-]/gu, ' ')
    .split(/\s+/)
    .filter((w) => w && !stop.has(w.toLowerCase()))
    .slice(0, 3);
  const name = words.map((w) => w[0].toUpperCase() + w.slice(1).toLowerCase()).join(' ');
  return name || 'My App';
}

function greeting(): string {
  const h = new Date().getHours();
  return h < 5 ? 'Up late?' : h < 12 ? 'Good morning!' : h < 18 ? 'Good afternoon!' : 'Good evening!';
}

function stageLabel(p: Project): { text: string; tone: 'ok' | 'warn' | 'accent' | 'idle' } {
  switch (p.status.stage) {
    case 'planned':
      return { text: 'Plan ready', tone: 'accent' };
    case 'generated':
      return { text: 'Code generated', tone: 'ok' };
    case 'imported':
      return { text: 'Imported', tone: 'idle' };
    default:
      return { text: 'New', tone: 'idle' };
  }
}

export default function Home() {
  const navigate = useApp((s) => s.navigate);
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const toast = useApp((s) => s.toast);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [deleting, setDeleting] = useState<Project | null>(null);
  const [cloning, setCloning] = useState(false);
  const [idea, setIdea] = useState('');
  const [starting, setStarting] = useState(false);

  const load = () => api.listProjects().then(setProjects).catch((e) => toast('bad', errorMessage(e)));
  useEffect(() => {
    void load();
    void api.detectTools().then(setTools);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const importFolder = async () => {
    const dir = await openDialog({ directory: true, title: 'Open an existing project folder' });
    if (typeof dir !== 'string') return;
    try {
      const p = await api.importProject(dir);
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const importZip = async () => {
    const file = await openDialog({ title: 'Import a project from a ZIP archive', filters: [{ name: 'ZIP archive', extensions: ['zip'] }] });
    if (typeof file !== 'string') return;
    try {
      const p = await api.importZipProject(file);
      toast('ok', 'Extracted safely — no scripts were run. Review package.json scripts before installing.');
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const ready = !!keyStatus?.stored && !!settings?.model;
  const buildIdea = async () => {
    const text = idea.trim();
    if (text.length < 10 || !ready) return;
    setStarting(true);
    try {
      const p = await api.createProject(nameFromIdea(text), text);
      markAutoBuild(p.id);
      await api.startPlan(p.id, text, '');
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const missingTools = tools?.filter((t) => !t.found) ?? [];
  const setup = [
    { done: !!keyStatus?.stored, label: keyStatus?.stored ? 'NVIDIA API key saved' : 'Save your NVIDIA API key', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <KeyRound size={14} /> },
    { done: !!settings?.model, label: settings?.model ? `Model: ${settings.model}` : 'Choose a model', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <Sparkles size={14} /> },
    {
      done: tools ? missingTools.filter((t) => t.name === 'node' || t.name === 'npm').length === 0 : false,
      label: tools ? (missingTools.some((t) => t.name === 'node' || t.name === 'npm') ? 'Node.js / npm not found' : 'Node.js and npm available') : 'Checking tools…',
      action: () => navigate({ name: 'settings', tab: 'workspace' }),
      icon: <Wrench size={14} />,
    },
  ];
  const setupDone = setup.every((s) => s.done);

  return (
    <div className="page">
      <div className="page-inner">
        <div className="home-hero">
          <GeniusAvatar size={96} mood={starting ? 'working' : idea.trim() ? 'listening' : 'idle'} />
          <div className="home-greet">{greeting()} I'm Genius.</div>
          <h1>What should we build today?</h1>
          <div className="prompt-box">
            <textarea
              aria-label="Describe your app"
              placeholder="Describe your app or website… e.g. “A portfolio site for a photographer with a dark, elegant look”"
              value={idea}
              rows={3}
              onChange={(e) => setIdea(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  void buildIdea();
                }
              }}
              autoFocus
            />
            <div className="row prompt-tools">
              <button className="btn ghost sm" onClick={() => navigate({ name: 'create' })} title="Name, tech and design preferences, and review the plan before code is written">
                <SlidersHorizontal size={13} /> Detailed setup
              </button>
              <span className="spacer" />
              <span className="faint tiny">{ready ? 'Genius plans it, then writes the code' : 'Finish setup below first'}</span>
              <button className="send-btn lg" aria-label="Build it" title="Build it (Enter)" disabled={idea.trim().length < 10 || !ready || starting} onClick={() => void buildIdea()}>
                {starting ? <span className="spinner" /> : <ArrowUp size={18} />}
              </button>
            </div>
          </div>
          <div className="suggestions center-row">
            {IDEAS.map((x) => (
              <button key={x.label} onClick={() => setIdea(x.text)}>
                {x.label}
              </button>
            ))}
          </div>
          <div className="row home-actions">
            <Button variant="ghost" icon={<Plus size={15} />} onClick={() => navigate({ name: 'create' })}>
              New application
            </Button>
            <Button variant="ghost" icon={<FolderOpen size={15} />} onClick={() => void importFolder()}>
              Open folder
            </Button>
            <Button variant="ghost" icon={<FileArchive size={15} />} onClick={() => void importZip()}>
              Import ZIP
            </Button>
            <Button variant="ghost" icon={<Github size={15} />} onClick={() => setCloning(true)}>
              Clone from GitHub
            </Button>
          </div>
        </div>

        {!setupDone && (
          <div className="card pad" style={{ marginTop: 16 }}>
            <div style={{ fontWeight: 650, marginBottom: 8 }}>Finish setup</div>
            <div className="col" style={{ gap: 6 }}>
              {setup.map((s) => (
                <button key={s.label} className="row btn ghost" style={{ justifyContent: 'flex-start', height: 30 }} onClick={s.action}>
                  {s.done ? <Check size={15} color="var(--ok)" /> : <X size={15} color="var(--warn)" />}
                  {s.icon}
                  <span>{s.label}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="section-title">
          <h2>Recent projects</h2>
          {projects && <Badge>{projects.length}</Badge>}
        </div>
        {projects === null ? (
          <div className="row muted" style={{ padding: 20 }}>
            <Spinner /> Loading projects…
          </div>
        ) : projects.length === 0 ? (
          <div className="card" style={{ marginTop: 14 }}>
            <Empty icon={<Sparkles size={20} />} title="No projects yet" action={<Button variant="primary" onClick={() => navigate({ name: 'create' })}>Create your first app</Button>}>
              Projects you create or open appear here, with their real build status.
            </Empty>
          </div>
        ) : (
          <div className="project-grid">
            {projects.map((p) => {
              const st = stageLabel(p);
              return (
                <div key={p.id} className="project-card" role="button" tabIndex={0} onClick={() => navigate({ name: 'project', id: p.id })} onKeyDown={(e) => e.key === 'Enter' && navigate({ name: 'project', id: p.id })}>
                  <div className="row">
                    <h3 className="grow ellipsis">{p.name}</h3>
                    <button
                      className="icon-btn"
                      aria-label={`Delete ${p.name}`}
                      title="Delete project"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleting(p);
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                  <p>{p.description || p.spec?.purpose || p.path}</p>
                  <div className="row wrap">
                    <Badge tone={st.tone}>{st.text}</Badge>
                    {p.status.lastBuild ? (
                      <StatusBadge status={p.status.lastBuild.status} label={`build ${p.status.lastBuild.status}`} />
                    ) : (
                      <Badge>not built</Badge>
                    )}
                    {p.status.publish?.step === 'pushed' && <Badge tone="ok">on GitHub</Badge>}
                    <span className="spacer" />
                    <span className="faint tiny">{timeAgo(p.lastOpenedAt ?? p.updatedAt)}</span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
      {deleting && (
        <DeleteDialog
          project={deleting}
          onClose={() => setDeleting(null)}
          onDone={() => {
            setDeleting(null);
            void load();
          }}
        />
      )}
      {cloning && <CloneDialog onClose={() => setCloning(false)} />}
    </div>
  );
}

function DeleteDialog({ project, onClose, onDone }: { project: Project; onClose: () => void; onDone: () => void }) {
  const toast = useApp((s) => s.toast);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      const r = await api.deleteProject(project.id, deleteFiles);
      toast('ok', r.filesDeleted ? `Deleted ${project.name} and its files` : `Removed ${project.name} from Genius (files kept)`);
      onDone();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={`Delete “${project.name}”?`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="danger" loading={busy} disabled={deleteFiles && confirm !== project.name} onClick={() => void run()}>
            {deleteFiles ? 'Delete project and files' : 'Remove from Genius'}
          </Button>
        </>
      }
    >
      <p className="muted" style={{ margin: 0 }}>
        This removes the project, its history and activity from Genius. Running processes are stopped.
      </p>
      <label className="checkbox">
        <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
        Also permanently delete the project folder
      </label>
      <div className="mono small faint">{project.path}</div>
      {deleteFiles && (
        <>
          <Callout tone="bad">The folder and all its files will be deleted. This cannot be undone. Folders outside the Genius workspace are never deleted.</Callout>
          <input className="input" placeholder={`Type “${project.name}” to confirm`} value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        </>
      )}
    </Modal>
  );
}

function CloneDialog({ onClose }: { onClose: () => void }) {
  const navigate = useApp((s) => s.navigate);
  const toast = useApp((s) => s.toast);
  const [repos, setRepos] = useState<RepoInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    api
      .githubStatus()
      .then((s) => {
        if (!s.connected) throw new Error(s.detail || 'Not connected to GitHub. Connect in Settings → GitHub.');
        return api.githubRepos();
      })
      .then(setRepos)
      .catch((e) => setError(errorMessage(e)));
  }, []);
  const clone = async (r: RepoInfo) => {
    setBusy(r.nameWithOwner);
    try {
      const p = await api.githubClone(r.nameWithOwner);
      toast('ok', `Cloned ${r.nameWithOwner}`);
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const shown = (repos ?? []).filter((r) => r.nameWithOwner.toLowerCase().includes(filter.toLowerCase()));
  return (
    <Modal title="Clone a GitHub repository" onClose={onClose} wide>
      {error ? (
        <>
          <Callout tone="warn">{error}</Callout>
          <div>
            <Button onClick={() => navigate({ name: 'settings', tab: 'github' })}>Open GitHub settings</Button>
          </div>
        </>
      ) : repos === null ? (
        <div className="row muted">
          <Spinner /> Loading repositories you can access…
        </div>
      ) : (
        <>
          <input className="input" placeholder="Filter repositories" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter repositories" />
          <div className="model-list">
            {shown.map((r) => (
              <div key={r.nameWithOwner} className="model-row" style={{ cursor: 'default' }}>
                <div className="col" style={{ gap: 0 }}>
                  <span>{r.nameWithOwner}</span>
                  {r.description && <span className="faint tiny ellipsis">{r.description}</span>}
                </div>
                <Badge tone={r.visibility === 'public' ? 'warn' : 'idle'}>{r.visibility}</Badge>
                <Button size="sm" loading={busy === r.nameWithOwner} disabled={!!busy} onClick={() => void clone(r)}>
                  Clone
                </Button>
              </div>
            ))}
            {shown.length === 0 && <div className="faint small" style={{ padding: 12 }}>No repositories match.</div>}
          </div>
        </>
      )}
    </Modal>
  );
}
