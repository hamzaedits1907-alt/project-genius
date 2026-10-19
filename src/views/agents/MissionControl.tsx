// Mission Control: missions per project, the coordinator's real task graph, live updates,
// task details and the controls (start/pause/resume/cancel, retry, reprioritize, approve/reject).
import { useMemo, useState } from 'react';
import { AlertTriangle, Check, Pause, Play, Plus, RotateCcw, Target, X } from 'lucide-react';
import {
  MISSION_STATUS_LABEL,
  TASK_STATUS_LABEL,
  canCancelMission,
  canCancelTask,
  canPauseMission,
  canReprioritizeTask,
  canResumeMission,
  canRetryTask,
  canStartMission,
  missionTone,
  missionTotals,
  taskTone,
  tokenText,
} from '../../lib/agents';
import { timeAgo } from '../../lib/logic';
import type { Mission, MissionTask } from '../../lib/types';
import { useAgents } from '../../store/agents';
import { Badge, Button, Callout, Empty, Field, Modal, Segmented, Spinner } from '../../components/ui';
import ReviewView from '../workspace/ReviewView';
import { TaskBoard, TaskGraph } from './MissionGraph';

function useAgentName() {
  const agents = useAgents((s) => s.agents);
  return (id: string | null) => (id ? (agents.find((a) => a.id === id)?.name ?? 'Unknown agent') : 'Unassigned');
}

function NewMissionDialog({ onClose }: { onClose: () => void }) {
  const projects = useAgents((s) => s.projects);
  const teams = useAgents((s) => s.teams);
  const createMission = useAgents((s) => s.createMission);
  const [projectId, setProjectId] = useState(projects[0]?.id ?? '');
  const [teamId, setTeamId] = useState('');
  const [goal, setGoal] = useState('');
  const [busy, setBusy] = useState(false);
  const [touched, setTouched] = useState(false);
  const goalError = goal.trim().length < 8 ? 'Describe the goal in at least a sentence (8+ characters).' : null;
  const projectError = projectId ? null : 'Choose a project.';

  const submit = async () => {
    setTouched(true);
    if (goalError || projectError) return;
    setBusy(true);
    const m = await createMission(projectId, goal.trim(), teamId || null);
    setBusy(false);
    if (m) onClose();
  };

  return (
    <Modal
      title="New mission"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} onClick={() => void submit()}>Plan mission</Button>
        </>
      }
    >
      <div className="col" style={{ gap: 14 }}>
        <Field label="Goal" htmlFor="nm-goal" hint="What should the team get done? A coordinator agent splits it into tasks you can review before anything starts.">
          <textarea id="nm-goal" className="textarea" rows={4} value={goal} onChange={(e) => setGoal(e.target.value)} aria-invalid={touched && goalError ? true : undefined} placeholder="e.g. Add a settings page with dark mode and tests" />
          {touched && goalError && <div className="field-error" role="alert">{goalError}</div>}
        </Field>
        <Field label="Project" htmlFor="nm-project">
          <select id="nm-project" className="select" value={projectId} onChange={(e) => setProjectId(e.target.value)} aria-invalid={touched && projectError ? true : undefined}>
            <option value="">Choose a project…</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
          {touched && projectError && <div className="field-error" role="alert">{projectError}</div>}
        </Field>
        <Field label="Team" htmlFor="nm-team" hint={teams.length ? 'The team’s coordinator plans the work and its members carry it out.' : 'No teams yet: create one on the Teams tab, or let Genius choose agents.'}>
          <select id="nm-team" className="select" value={teamId} onChange={(e) => setTeamId(e.target.value)}>
            <option value="">No team — let Genius choose</option>
            {teams.map((t) => (
              <option key={t.id} value={t.id}>{t.name}</option>
            ))}
          </select>
        </Field>
      </div>
    </Modal>
  );
}

const verifyTone = (v: string) => (v === 'passed' ? 'ok' : v === 'failed' ? 'bad' : 'idle');

function TaskPanel({ mission, task }: { mission: Mission; task: MissionTask }) {
  const taskAction = useAgents((s) => s.taskAction);
  const agentName = useAgentName();
  const [priority, setPriority] = useState(String(task.priority));
  const [busy, setBusy] = useState<string | null>(null);
  const [review, setReview] = useState(false);
  const deps = task.dependsOn.map((d) => mission.tasks.find((t) => t.id === d)).filter((t): t is MissionTask => !!t);

  const run = async (kind: 'cancel' | 'retry' | 'approve' | 'reject' | 'reprioritize', p?: number) => {
    setBusy(kind);
    await taskAction(kind, task.id, p);
    setBusy(null);
  };
  const prio = Number(priority);
  const prioValid = priority.trim() !== '' && Number.isInteger(prio) && prio >= 0 && prio <= 100;

  return (
    <aside className="mg-task card pad col" aria-label="Task details" style={{ gap: 12 }}>
      <div className="row wrap">
        <h3 style={{ margin: 0, fontSize: 15 }} className="grow">{task.title}</h3>
        <Badge tone={taskTone(task.status)}>{TASK_STATUS_LABEL[task.status]}</Badge>
      </div>
      {task.description && <p className="muted" style={{ margin: 0 }}>{task.description}</p>}
      <dl className="mg-facts">
        <dt>Assigned to</dt><dd>{agentName(task.assigneeAgentId)}</dd>
        <dt>Depends on</dt><dd>{deps.length ? deps.map((d) => d.title).join(', ') : 'nothing'}</dd>
        <dt>Priority</dt><dd>{task.priority}</dd>
        <dt>Usage</dt><dd>{task.usage.tokens.toLocaleString('en-US')} tokens · {task.usage.calls} calls</dd>
        {task.startedAt && (<><dt>Started</dt><dd>{timeAgo(task.startedAt)}</dd></>)}
        {task.endedAt && (<><dt>Ended</dt><dd>{timeAgo(task.endedAt)}</dd></>)}
        {task.workspace && (<><dt>Isolated copy</dt><dd className="mono small">{task.workspace}</dd></>)}
      </dl>

      {task.blocker && (
        <Callout tone="warn" title="Blocked">{task.blocker}</Callout>
      )}
      {task.result && (
        <div>
          <div className="panel-title" style={{ marginBottom: 4 }}>Result</div>
          <pre className="ag-result">{task.result}</pre>
        </div>
      )}
      <div>
        <div className="panel-title" style={{ marginBottom: 4 }}>Verification</div>
        <div className="row wrap">
          <Badge tone={verifyTone(task.verification.build)}>Build: {task.verification.build}</Badge>
          <Badge tone={verifyTone(task.verification.tests)}>Tests: {task.verification.tests}</Badge>
        </div>
        {task.verification.notes && <div className="small muted" style={{ marginTop: 4 }}>{task.verification.notes}</div>}
        {task.verification.build === 'unknown' && task.verification.tests === 'unknown' && <div className="tiny faint">Nothing has been verified for this task yet.</div>}
      </div>

      {task.changesetId && (
        <div className="row wrap">
          <Button size="sm" onClick={() => setReview(true)}>Review the isolated changes</Button>
        </div>
      )}

      {task.status === 'awaiting_approval' && (
        <Callout tone="warn" title="This task is waiting for your decision">
          The agent proposed changes in its own isolated copy of the project. Nothing in your project changes until you approve.
          <div className="row" style={{ marginTop: 8 }}>
            <Button variant="ok" icon={<Check size={13} />} loading={busy === 'approve'} disabled={!!busy} onClick={() => void run('approve')}>Approve</Button>
            <Button variant="danger" icon={<X size={13} />} loading={busy === 'reject'} disabled={!!busy} onClick={() => void run('reject')}>Reject</Button>
          </div>
        </Callout>
      )}

      <div className="row wrap">
        {canRetryTask(task) && <Button size="sm" icon={<RotateCcw size={13} />} loading={busy === 'retry'} disabled={!!busy} onClick={() => void run('retry')}>Retry task</Button>}
        {canCancelTask(task) && <Button size="sm" variant="danger" loading={busy === 'cancel'} disabled={!!busy} onClick={() => void run('cancel')}>Cancel task</Button>}
      </div>
      {canReprioritizeTask(task) && (
        <div className="row">
          <label className="small muted" htmlFor="mg-prio">Priority (0–100)</label>
          <input id="mg-prio" type="number" min={0} max={100} className="input" style={{ width: 80 }} value={priority} onChange={(e) => setPriority(e.target.value)} aria-invalid={prioValid ? undefined : true} />
          <Button size="sm" disabled={!prioValid || prio === task.priority || !!busy} loading={busy === 'reprioritize'} onClick={() => void run('reprioritize', prio)}>Set priority</Button>
        </div>
      )}

      {review && task.changesetId && (
        <Modal wide title={`Changes: ${task.title}`} onClose={() => setReview(false)}>
          <div style={{ height: '62vh', minHeight: 320 }}>
            <ReviewView changesetId={task.changesetId} />
          </div>
        </Modal>
      )}
    </aside>
  );
}

function MissionDetail({ mission }: { mission: Mission }) {
  const missionAction = useAgents((s) => s.missionAction);
  const selectedTaskId = useAgents((s) => s.selectedTaskId);
  const set = useAgents((s) => s.set);
  const projects = useAgents((s) => s.projects);
  const teams = useAgents((s) => s.teams);
  const agentName = useAgentName();
  const [view, setView] = useState<'graph' | 'board'>('graph');
  const [busy, setBusy] = useState<string | null>(null);
  const totals = useMemo(() => missionTotals(mission), [mission]);
  const task = mission.tasks.find((t) => t.id === selectedTaskId) ?? null;
  const attention = mission.tasks.filter((t) => t.status === 'awaiting_approval' || t.status === 'blocked');
  const project = projects.find((p) => p.id === mission.projectId);
  const team = teams.find((t) => t.id === mission.teamId);

  const act = async (kind: 'start' | 'pause' | 'resume' | 'cancel') => {
    if (kind === 'cancel' && !window.confirm('Cancel this mission? Running tasks are stopped; finished work is kept.')) return;
    setBusy(kind);
    await missionAction(kind, mission.id);
    setBusy(null);
  };

  return (
    <div className="mg-detail">
      <div className="mg-head">
        <div className="row wrap">
          <h2 style={{ margin: 0, fontSize: 18 }} className="grow">{mission.title}</h2>
          <Badge tone={missionTone(mission.status)}>
            {(mission.status === 'planning' || mission.status === 'running') && <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} aria-hidden />}
            {MISSION_STATUS_LABEL[mission.status]}
          </Badge>
        </div>
        <p className="muted" style={{ margin: '4px 0 0' }}>{mission.goal}</p>
        <div className="small faint" style={{ marginTop: 4 }}>
          {project?.name ?? 'Unknown project'} · {team ? `team ${team.name}` : 'no team'} · updated {timeAgo(mission.updatedAt)}
        </div>
        <div className="mg-totals" aria-label="Mission totals">
          <span><b>{totals.done}</b>/{totals.total} done</span>
          <span><b>{totals.running}</b> running</span>
          <span className={totals.blocked ? 'warn' : ''}><b>{totals.blocked}</b> blocked</span>
          <span className={totals.awaiting ? 'warn' : ''}><b>{totals.awaiting}</b> need approval</span>
          <span className={totals.failed ? 'bad' : ''}><b>{totals.failed}</b> failed</span>
          <span><b>{tokenText(totals.tokens)}</b> tokens</span>
          <span><b>{totals.calls}</b> model calls</span>
        </div>
        <div className="row wrap" style={{ marginTop: 10 }}>
          {canStartMission(mission) && <Button variant="primary" icon={<Play size={13} />} loading={busy === 'start'} disabled={!!busy} onClick={() => void act('start')}>Start mission</Button>}
          {canPauseMission(mission) && <Button icon={<Pause size={13} />} loading={busy === 'pause'} disabled={!!busy} onClick={() => void act('pause')}>Pause</Button>}
          {canResumeMission(mission) && <Button variant="primary" icon={<Play size={13} />} loading={busy === 'resume'} disabled={!!busy} onClick={() => void act('resume')}>Resume</Button>}
          {canCancelMission(mission) && <Button variant="danger" loading={busy === 'cancel'} disabled={!!busy} onClick={() => void act('cancel')}>Cancel mission</Button>}
          <span className="grow" />
          <Segmented label="View" value={view} onChange={setView} options={[{ value: 'graph', label: 'Graph' }, { value: 'board', label: 'Board' }]} />
        </div>
      </div>

      {mission.summary && <Callout tone={mission.status === 'failed' ? 'bad' : 'ok'} title="Summary">{mission.summary}</Callout>}

      {attention.length > 0 && (
        <div className="mg-attention" role="region" aria-label="Needs your attention">
          <AlertTriangle size={15} color="var(--warn)" />
          <div className="grow col" style={{ gap: 4 }}>
            {attention.map((t) => (
              <button key={t.id} type="button" className="link-btn" onClick={() => set({ selectedTaskId: t.id })}>
                <b>{t.title}</b> — {t.status === 'awaiting_approval' ? 'waiting for your approval' : `blocked${t.blocker ? `: ${t.blocker}` : ''}`}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className={`mg-body ${task ? 'with-task' : ''}`}>
        <div className="mg-canvas">
          {mission.tasks.length === 0 ? (
            mission.status === 'planning' ? (
              <Empty icon={<Spinner />} title="The coordinator is planning the tasks…">The task graph appears here as soon as the plan is ready.</Empty>
            ) : (
              <Empty title="This mission has no tasks">{mission.status === 'failed' ? 'Planning failed. See the summary above.' : 'Nothing to show.'}</Empty>
            )
          ) : view === 'graph' ? (
            <TaskGraph tasks={mission.tasks} selectedId={selectedTaskId} onSelect={(id) => set({ selectedTaskId: id })} agentName={agentName} />
          ) : (
            <TaskBoard tasks={mission.tasks} selectedId={selectedTaskId} onSelect={(id) => set({ selectedTaskId: id })} agentName={agentName} />
          )}
        </div>
        {task && <TaskPanel key={task.id} mission={mission} task={task} />}
      </div>
    </div>
  );
}

export default function MissionControl() {
  const missions = useAgents((s) => s.missions);
  const projects = useAgents((s) => s.projects);
  const selectedId = useAgents((s) => s.selectedMissionId);
  const set = useAgents((s) => s.set);
  const loaded = useAgents((s) => s.loaded);
  const [projectFilter, setProjectFilter] = useState('');
  const [creating, setCreating] = useState(false);

  const list = useMemo(
    () => missions.filter((m) => !projectFilter || m.projectId === projectFilter).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
    [missions, projectFilter],
  );
  const mission = missions.find((m) => m.id === selectedId) ?? null;

  return (
    <div className="ag-split">
      <div className="ag-list">
        <div className="row" style={{ padding: '10px 12px' }}>
          <span className="panel-title grow">Missions</span>
          <Button size="sm" icon={<Plus size={13} />} onClick={() => setCreating(true)}>New</Button>
        </div>
        <div style={{ padding: '0 12px 8px' }}>
          <label className="sr-only" htmlFor="mc-filter">Filter by project</label>
          <select id="mc-filter" className="select" value={projectFilter} onChange={(e) => setProjectFilter(e.target.value)}>
            <option value="">All projects</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </div>
        {list.length === 0 ? (
          <div className="small muted" style={{ padding: '4px 12px' }}>{loaded ? 'No missions yet. Describe a goal and a team of agents will plan and do the work.' : 'Loading…'}</div>
        ) : (
          <ul className="ag-items" aria-label="Missions">
            {list.map((m) => {
              const t = missionTotals(m);
              return (
                <li key={m.id}>
                  <button type="button" className={`ag-item ${m.id === selectedId ? 'on' : ''}`} aria-current={m.id === selectedId ? 'true' : undefined} onClick={() => set({ selectedMissionId: m.id, selectedTaskId: null })}>
                    <span className="grow col" style={{ gap: 2, minWidth: 0, textAlign: 'left' }}>
                      <b className="ellipsis">{m.title}</b>
                      <span className="small muted ellipsis">{projects.find((p) => p.id === m.projectId)?.name ?? 'Unknown project'} · {t.done}/{t.total} done{t.awaiting ? ` · ${t.awaiting} to approve` : ''}</span>
                    </span>
                    <Badge tone={missionTone(m.status)}>{MISSION_STATUS_LABEL[m.status]}</Badge>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="ag-main scroll">
        {mission ? (
          <MissionDetail key={mission.id} mission={mission} />
        ) : (
          <Empty icon={<Target size={20} />} title="Select or create a mission" action={<Button variant="primary" onClick={() => setCreating(true)}>New mission</Button>}>
            A mission turns a goal into a dependency-aware task graph that several agents work through, with your approval before changes land.
          </Empty>
        )}
      </div>
      {creating && <NewMissionDialog onClose={() => setCreating(false)} />}
    </div>
  );
}
