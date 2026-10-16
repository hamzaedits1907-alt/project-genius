import { useEffect, useState } from 'react';
import { Check, FolderOpen, KeyRound, Plus, Sparkles, Trash2, Wrench, X, FileArchive } from 'lucide-react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { Github } from '../components/icons';
import { api, errorMessage } from '../lib/api';
import { timeAgo } from '../lib/logic';
import type { Project, RepoInfo, ToolStatus } from '../lib/types';
import { useApp } from '../store/app';
import { Badge, Button, Callout, Empty, Modal, Spinner, StatusBadge } from '../components/ui';

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
        <div className="hero">
          <div>
            <h1>What do you want to build?</h1>
            <p className="lead">Describe an app. Genius plans it, writes real source code, runs it, and helps you fix and ship it.</p>
            <div className="row" style={{ marginTop: 16 }}>
              <Button variant="primary" size="lg" icon={<Plus size={16} />} onClick={() => navigate({ name: 'create' })}>
                New application
              </Button>
              <Button size="lg" icon={<FolderOpen size={16} />} onClick={() => void importFolder()}>
                Open folder
              </Button>
              <Button size="lg" icon={<FileArchive size={16} />} onClick={() => void importZip()}>
                Import ZIP
              </Button>
              <Button size="lg" icon={<Github size={16} />} onClick={() => setCloning(true)}>
                Clone from GitHub
              </Button>
            </div>
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
