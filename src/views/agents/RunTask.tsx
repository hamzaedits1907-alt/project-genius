// "Give this agent a task": project + task text -> run_agent_task, with the live result.
import { useState } from 'react';
import { ArrowUpRight, Play } from 'lucide-react';
import { MISSION_STATUS_LABEL, TASK_STATUS_LABEL, missionTone, taskTone } from '../../lib/agents';
import { useAgents } from '../../store/agents';
import { Badge, Button, Field } from '../../components/ui';

export function LiveMissionResult({ missionId }: { missionId: string }) {
  const mission = useAgents((s) => s.missions.find((m) => m.id === missionId));
  const set = useAgents((s) => s.set);
  if (!mission) return null;
  return (
    <div className="card pad col" style={{ gap: 8 }} aria-label="Task result" role="region">
      <div className="row">
        <b className="grow ellipsis">{mission.title}</b>
        <Badge tone={missionTone(mission.status)}>{MISSION_STATUS_LABEL[mission.status]}</Badge>
      </div>
      {mission.tasks.length === 0 && <div className="small muted">{mission.status === 'planning' ? 'Preparing the task…' : 'No task details reported yet.'}</div>}
      {mission.tasks.map((t) => (
        <div key={t.id} className="col" style={{ gap: 4 }}>
          <div className="row">
            <span className="grow">{t.title}</span>
            <Badge tone={taskTone(t.status)}>{TASK_STATUS_LABEL[t.status]}</Badge>
          </div>
          {t.blocker && <div className="small" style={{ color: 'var(--warn)' }}>Blocked: {t.blocker}</div>}
          {t.result && <pre className="ag-result">{t.result}</pre>}
        </div>
      ))}
      {mission.summary && <div className="small">{mission.summary}</div>}
      <div className="row">
        <Button size="sm" icon={<ArrowUpRight size={13} />} onClick={() => set({ tab: 'missions', selectedMissionId: mission.id, selectedTaskId: null })}>
          Open in Mission Control
        </Button>
      </div>
    </div>
  );
}

export default function RunTask({ agentId, agentName }: { agentId: string; agentName: string }) {
  const projects = useAgents((s) => s.projects);
  const runTask = useAgents((s) => s.runTask);
  const [projectId, setProjectId] = useState('');
  const [task, setTask] = useState('');
  const [busy, setBusy] = useState(false);
  const [missionId, setMissionId] = useState<string | null>(null);
  const pid = projectId || projects[0]?.id || '';
  const ready = !!pid && task.trim().length >= 3;

  const run = async () => {
    if (!ready) return;
    setBusy(true);
    const m = await runTask(agentId, pid, task.trim());
    setBusy(false);
    if (m) {
      setMissionId(m.id);
      setTask('');
    }
  };

  return (
    <div className="col" style={{ gap: 10 }}>
      {projects.length === 0 ? (
        <div className="small muted">Create or import a project first, then give {agentName} a task in it.</div>
      ) : (
        <>
          <Field label="Project" htmlFor={`rt-project-${agentId}`}>
            <select id={`rt-project-${agentId}`} className="select" value={pid} onChange={(e) => setProjectId(e.target.value)}>
              {projects.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </Field>
          <Field label="Task" htmlFor={`rt-task-${agentId}`} hint={`Describe what ${agentName} should do. Changes are never applied without your review (unless this agent is set to auto).`}>
            <textarea
              id={`rt-task-${agentId}`}
              className="textarea"
              rows={3}
              value={task}
              placeholder="e.g. Review the sign-in form for accessibility problems"
              onChange={(e) => setTask(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) void run();
              }}
            />
          </Field>
          <div className="row">
            <Button variant="primary" icon={<Play size={13} />} loading={busy} disabled={!ready} onClick={() => void run()}>
              Run task
            </Button>
            <span className="tiny faint">Ctrl+Enter</span>
          </div>
        </>
      )}
      {missionId && <LiveMissionResult missionId={missionId} />}
    </div>
  );
}
