import { useCallback, useEffect, useState } from 'react';
import { Trash2 } from 'lucide-react';
import type { Memory } from '../../lib/types';
import { api, errorMessage } from '../../lib/api';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button } from '../../components/ui';

/** Project memory: decisions and conventions that are included in every AI request for this project. */
export default function MemoryPanel() {
  const project = useWorkspace((s) => s.project);
  const refresh = useWorkspace((s) => s.refreshProject);
  const toast = useApp((s) => s.toast);
  const [notes, setNotes] = useState(project?.notes ?? '');
  const [busy, setBusy] = useState(false);
  const [memories, setMemories] = useState<Memory[]>([]);
  const [draft, setDraft] = useState('');
  const [kind, setKind] = useState('decision');
  const tasksDone = useWorkspace((s) => s.activity.filter((e) => e.kind === 'task' && e.status !== 'running').length);
  useEffect(() => {
    setNotes(project?.notes ?? '');
  }, [project?.notes]);
  const projectId = project?.id;
  const loadMemories = useCallback(() => {
    if (projectId) api.listMemories(projectId).then(setMemories).catch((e) => toast('bad', errorMessage(e)));
  }, [projectId, toast]);
  useEffect(() => {
    loadMemories();
  }, [loadMemories, tasksDone]);
  if (!project) return null;
  const addMemory = async () => {
    try {
      await api.addMemory(project.id, kind, draft);
      setDraft('');
      loadMemories();
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const removeMemory = async (id: string) => {
    try {
      await api.deleteMemory(project.id, id);
      loadMemories();
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const SOURCE: Record<string, { label: string; tone: 'ok' | 'accent' | 'warn' }> = {
    user: { label: 'stated by you', tone: 'ok' },
    repo: { label: 'verified in repo', tone: 'accent' },
    agent: { label: 'agent inference', tone: 'warn' },
  };
  const dirty = notes !== project.notes;
  const save = async () => {
    setBusy(true);
    try {
      await api.updateProjectMeta(project.id, project.name, project.description, notes);
      await refresh();
      toast('ok', 'Project memory saved');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">Project memory</span>
        <Button size="sm" variant="primary" disabled={!dirty} loading={busy} onClick={() => void save()}>
          Save
        </Button>
      </div>
      <div className="col grow scroll" style={{ padding: 10, minHeight: 0 }}>
        <div className="faint small">
          Architecture decisions, conventions and preferences. Genius includes this in every planning, coding and repair request for this project so it stays consistent.
        </div>
        <textarea className="textarea mono" style={{ minHeight: 140, fontSize: 12 }} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder={'e.g.\n- Use CSS modules, no Tailwind\n- Keep all state in src/store.ts\n- Dates are always ISO strings'} aria-label="Project memory" />
        <div className="panel-title" style={{ marginTop: 8 }}>
          Remembered facts ({memories.length})
        </div>
        <div className="faint tiny">Each item shows where it came from. Agent inferences may be stale — delete anything wrong.</div>
        {memories.map((m) => (
          <div key={m.id} className="mem-row">
            <div className="row" style={{ gap: 6 }}>
              <Badge>{m.kind}</Badge>
              <Badge tone={SOURCE[m.source]?.tone}>{SOURCE[m.source]?.label ?? m.source}</Badge>
              <span className="faint tiny grow">{m.updatedAt.slice(0, 10)}</span>
              <button className="icon-btn" aria-label="Delete memory" onClick={() => void removeMemory(m.id)}>
                <Trash2 size={12} />
              </button>
            </div>
            <div className="small">{m.content}</div>
          </div>
        ))}
        <div className="row">
          <select className="input sm" value={kind} onChange={(e) => setKind(e.target.value)} aria-label="Memory kind">
            {['decision', 'convention', 'constraint', 'problem', 'integration'].map((k) => (
              <option key={k}>{k}</option>
            ))}
          </select>
          <input className="input sm grow" value={draft} onChange={(e) => setDraft(e.target.value)} placeholder="Add something Genius should remember" aria-label="New memory" />
          <Button size="sm" disabled={!draft.trim()} onClick={() => void addMemory()}>
            Add
          </Button>
        </div>
        <div className="faint tiny mono ellipsis" title={project.path}>
          {project.path}
        </div>
      </div>
    </>
  );
}
