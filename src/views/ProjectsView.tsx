// Projects: the code projects Genius knows (open them, see what Project Brain understands about each)
// and the files you attached in conversations. In the browser there are no local project folders, so the
// project list is replaced by an honest explanation.
import { useEffect, useState } from 'react';
import { FileText, FolderOpen, Plus } from 'lucide-react';
import { api } from '../lib/api';
import { timeAgo } from '../lib/logic';
import { capabilities } from '../lib/platform';
import type { BrainOverview, Project } from '../lib/types';
import { useApp } from '../store/app';
import { Button, Callout, Empty } from '../components/ui';
import { FilesDialog } from './chat/FilesDialog';

function BrainLine({ projectId }: { projectId: string }) {
  const [o, setO] = useState<BrainOverview | null>(null);
  useEffect(() => {
    let alive = true;
    api.brainOverview(projectId).then((r) => alive && setO(r)).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [projectId]);
  if (!o || (!o.projectType.length && !o.languages.length)) return null;
  return (
    <div className="small faint">
      {o.projectType.join(', ') || 'Project'}
      {o.languages.length ? ` · ${o.languages.slice(0, 3).map(([l, n]) => `${l} ${n}`).join(', ')}` : ''}
    </div>
  );
}

export default function ProjectsView() {
  const navigate = useApp((s) => s.navigate);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [files, setFiles] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!capabilities.localProjects) return;
    api.listProjects().then((p) => setProjects([...p].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)))).catch((e) => setError(String(e?.message ?? e)));
  }, []);

  return (
    <div className="pj-root">
      <div className="pj-inner">
        <header className="pj-head">
          <div className="grow">
            <h1>Projects</h1>
            <p className="muted" style={{ margin: '4px 0 0' }}>Your code projects and the files you gave Genius.</p>
          </div>
          <Button icon={<FileText size={14} />} onClick={() => setFiles(true)}>Saved files</Button>
          {capabilities.localProjects && <Button variant="primary" icon={<Plus size={14} />} onClick={() => navigate({ name: 'create' })}>New project</Button>}
        </header>

        {!capabilities.localProjects ? (
          <Callout tone="info" title="Local projects live in the desktop app">
            A browser cannot open folders on your computer, so code projects, the terminal, Git and live previews are part of Genius Desktop. Your files attached in conversations are available under “Saved files”.
          </Callout>
        ) : error ? (
          <Callout tone="bad" title="Projects could not be loaded">{error}</Callout>
        ) : projects === null ? (
          <p className="faint">Loading…</p>
        ) : projects.length === 0 ? (
          <Empty icon={<FolderOpen size={20} />} title="No projects yet" action={<Button variant="primary" onClick={() => navigate({ name: 'create' })}>Start a project</Button>}>
            Describe what you want to build, or turn a conversation into a project from the menu in Chat.
          </Empty>
        ) : (
          <ul className="pj-grid" style={{ listStyle: 'none', margin: 0, padding: 0 }} aria-label="Projects">
            {projects.map((p) => (
              <li key={p.id}>
                <button className="pj-card" style={{ width: '100%' }} onClick={() => navigate({ name: 'project', id: p.id })}>
                  <h3 className="ellipsis">{p.name}</h3>
                  <p>{p.description || 'No description yet.'}</p>
                  <BrainLine projectId={p.id} />
                  <span className="tiny faint">Updated {timeAgo(p.updatedAt)}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      {files && <FilesDialog onClose={() => setFiles(false)} />}
    </div>
  );
}
