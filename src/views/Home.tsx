import { useEffect, useState } from 'react';
import { ArrowRight, Check, FolderOpen, GitBranch, KeyRound, Pencil, PlugZap, Plus, Sparkles, Trash2, Wrench, X, FileArchive } from 'lucide-react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { Github } from '../components/icons';
import { api, errorMessage } from '../lib/api';
import { timeAgo } from '../lib/logic';
import type { Project, RepoInfo, ToolStatus } from '../lib/types';
import { useApp } from '../store/app';
import { Badge, Button, Callout, Empty, Modal, Spinner, StatusBadge } from '../components/ui';

const IDEAS = [
  'A modern personal dashboard with notes, a weather widget and quick links.',
  'A recipe book where I can add recipes, tag them and filter by tag.',
  'A habit tracker with daily check-ins, streaks and a calendar view.',
  'A landing page for my bakery with a menu, opening hours and a contact form.',
];

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

  const missingTools = tools?.filter((t) => !t.found) ?? [];
  const has = (n: string) => !!tools?.find((t) => t.name === n && t.found);
  const [conn, setConn] = useState<'idle' | 'testing' | 'ok' | 'bad'>('idle');
  const [connMsg, setConnMsg] = useState('');
  const testConn = async () => {
    setConn('testing');
    try {
      await api.testConnection();
      setConn('ok');
      setConnMsg('Connection verified with a real request.');
    } catch (e) {
      setConn('bad');
      setConnMsg(errorMessage(e));
    }
  };
  const setup = [
    { done: !!keyStatus?.stored, required: true, label: keyStatus?.stored ? 'NVIDIA API key saved' : 'Save your NVIDIA API key', hint: 'Settings → AI provider. Stored in your system credential store.', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <KeyRound size={14} /> },
    { done: !!settings?.model, required: true, label: settings?.model ? `Model: ${settings.model}` : 'Choose an AI model', hint: 'Settings → AI provider → Refresh models.', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <Sparkles size={14} /> },
    {
      done: tools ? has('node') && has('npm') : false,
      required: true,
      label: tools ? (has('node') && has('npm') ? 'Node.js and npm available' : 'Install Node.js (includes npm)') : 'Checking Node.js…',
      hint: missingTools.find((t) => t.name === 'node' || t.name === 'npm')?.hint ?? 'Needed to install and run your app. Get it from nodejs.org, then restart Genius.',
      action: () => navigate({ name: 'settings', tab: 'workspace' }),
      icon: <Wrench size={14} />,
    },
    { done: tools ? has('git') : false, required: false, label: tools ? (has('git') ? 'Git available' : 'Git not found (optional: history on GitHub)') : 'Checking Git…', hint: missingTools.find((t) => t.name === 'git')?.hint ?? '', action: () => navigate({ name: 'settings', tab: 'workspace' }), icon: <GitBranch size={14} /> },
  ];
  const ready = setup.filter((x) => x.required).every((x) => x.done);
  const [idea, setIdea] = useState('');
  const [reviewPlan, setReviewPlan] = useState(false);
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState<Project | null>(null);
  const create = async () => {
    setCreating(true);
    try {
      const r = await api.startCreateApp(null, idea, reviewPlan);
      navigate({ name: 'project', id: r.project.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setCreating(false);
    }
  };
  const missingReason = setup.find((x) => x.required && !x.done)?.label;

  return (
    <div className="page">
      <div className="page-inner">
        <div className="hero">
          <div style={{ width: '100%' }}>
            <h1>What do you want to build?</h1>
            <p className="lead">Describe it in your own words. Genius creates the project, writes the code, installs what it needs, checks it, and shows the running website. Then just tell it what to change.</p>
            <div className="card pad col" style={{ gap: 10, marginTop: 14 }}>
              <textarea
                className="textarea"
                rows={3}
                aria-label="Describe your app"
                placeholder="e.g. Create a modern personal dashboard with notes, a weather widget, and quick links."
                value={idea}
                onChange={(e) => setIdea(e.target.value)}
                onKeyDown={(e) => {
                  if ((e.ctrlKey || e.metaKey) && e.key === 'Enter' && idea.trim().length >= 10 && ready) void create();
                }}
              />
              <div className="examples" aria-label="Ideas">
                {IDEAS.map((ex) => (
                  <button key={ex} onClick={() => setIdea(ex)}>
                    {ex.split(' with')[0].split(' where')[0].slice(0, 44)}…
                  </button>
                ))}
              </div>
              <div className="row wrap">
                <label className="checkbox small">
                  <input type="checkbox" checked={reviewPlan} onChange={(e) => setReviewPlan(e.target.checked)} /> Let me review the plan before code is written
                </label>
                <span className="spacer" />
                <Button
                  variant="primary"
                  size="lg"
                  icon={<ArrowRight size={16} />}
                  loading={creating}
                  disabled={!ready || idea.trim().length < 10}
                  title={!ready ? `Finish setup first: ${missingReason}` : idea.trim().length < 10 ? 'Describe the app in at least a sentence' : undefined}
                  onClick={() => void create()}
                >
                  Create
                </Button>
              </div>
              {!ready && <div className="small warn-text">Before Genius can build: {missingReason}. See the checkup below.</div>}
            </div>
            <div className="row wrap" style={{ marginTop: 12 }}>
              <Button icon={<Plus size={15} />} onClick={() => navigate({ name: 'create' })}>
                Detailed form
              </Button>
              <Button icon={<FolderOpen size={15} />} onClick={() => void importFolder()}>
                Open folder
              </Button>
              <Button icon={<FileArchive size={15} />} onClick={() => void importZip()}>
                Import ZIP
              </Button>
              <Button icon={<Github size={15} />} onClick={() => setCloning(true)}>
                Clone from GitHub
              </Button>
            </div>
          </div>
        </div>

        <div className="card pad" style={{ marginTop: 16 }}>
          <div className="row" style={{ marginBottom: 8 }}>
            <div style={{ fontWeight: 650 }} className="grow">
              Checkup {ready ? <Badge tone="ok">ready to build</Badge> : <Badge tone="warn">action needed</Badge>}
            </div>
            <Button size="sm" variant="ghost" icon={<PlugZap size={13} />} loading={conn === 'testing'} disabled={!keyStatus?.stored || !settings?.model} title={!keyStatus?.stored || !settings?.model ? 'Save a key and choose a model first' : 'Send a small real request to the AI provider'} onClick={() => void testConn()}>
              Test AI connection
            </Button>
          </div>
          {conn !== 'idle' && conn !== 'testing' && <div className={`small ${conn === 'ok' ? 'ok-text' : 'bad-text'}`} style={{ marginBottom: 6 }}>{connMsg}</div>}
          <div className="col" style={{ gap: 4 }}>
            {setup.map((x) => (
              <button key={x.label} className="row btn ghost" style={{ justifyContent: 'flex-start', height: 'auto', minHeight: 30, textAlign: 'left' }} onClick={x.action} title={x.hint}>
                {x.done ? <Check size={15} color="var(--ok)" /> : <X size={15} color={x.required ? 'var(--warn)' : 'var(--text-3)'} />}
                {x.icon}
                <span className="grow">{x.label}</span>
                {!x.done && x.hint && <span className="tiny faint ellipsis" style={{ maxWidth: 360 }}>{x.hint}</span>}
              </button>
            ))}
          </div>
        </div>

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
            <Empty icon={<Sparkles size={20} />} title="No projects yet">
              Describe your first app above and press Create. Projects you create or open appear here.
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
                      aria-label={`Rename ${p.name}`}
                      title="Rename project"
                      onClick={(e) => {
                        e.stopPropagation();
                        setRenaming(p);
                      }}
                    >
                      <Pencil size={13} />
                    </button>
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
      {renaming && (
        <RenameDialog
          project={renaming}
          onClose={() => setRenaming(null)}
          onDone={() => {
            setRenaming(null);
            void load();
          }}
        />
      )}
    </div>
  );
}

function RenameDialog({ project, onClose, onDone }: { project: Project; onClose: () => void; onDone: () => void }) {
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState(project.name);
  const [busy, setBusy] = useState(false);
  const save = async () => {
    setBusy(true);
    try {
      await api.updateProjectMeta(project.id, name, project.description, project.notes);
      onDone();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title="Rename project"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} disabled={!name.trim() || name.trim() === project.name} onClick={() => void save()}>
            Rename
          </Button>
        </>
      }
    >
      <input className="input" aria-label="Project name" maxLength={80} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && name.trim() && void save()} />
      <div className="tiny faint">Only the name shown in Genius changes; the folder stays where it is.</div>
    </Modal>
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
