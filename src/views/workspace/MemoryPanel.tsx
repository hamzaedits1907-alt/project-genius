import { useEffect, useState } from 'react';
import { api, errorMessage } from '../../lib/api';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Button } from '../../components/ui';

/** Project memory: decisions and conventions that are included in every AI request for this project. */
export default function MemoryPanel() {
  const project = useWorkspace((s) => s.project);
  const refresh = useWorkspace((s) => s.refreshProject);
  const toast = useApp((s) => s.toast);
  const [notes, setNotes] = useState(project?.notes ?? '');
  const [busy, setBusy] = useState(false);
  useEffect(() => setNotes(project?.notes ?? ''), [project?.notes]);
  if (!project) return null;
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
      <div className="col grow" style={{ padding: 10, minHeight: 0 }}>
        <div className="faint small">
          Architecture decisions, conventions and preferences. Genius includes this in every planning, coding and repair request for this project so it stays consistent.
        </div>
        <textarea className="textarea mono grow" style={{ minHeight: 200, fontSize: 12 }} value={notes} onChange={(e) => setNotes(e.target.value)} placeholder={'e.g.\n- Use CSS modules, no Tailwind\n- Keep all state in src/store.ts\n- Dates are always ISO strings'} aria-label="Project memory" />
        <div className="faint tiny mono ellipsis" title={project.path}>
          {project.path}
        </div>
      </div>
    </>
  );
}
