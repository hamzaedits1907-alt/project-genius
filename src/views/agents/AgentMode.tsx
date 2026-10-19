// Agent Mode: describe a goal, Genius plans it, runs it with real tools in an isolated copy of the
// workspace, asks before anything lands, and reports only what actually happened. Everything shown
// here is the runtime state of a mission (store/agents.ts); nothing is simulated.
import { useEffect, useMemo, useRef, useState } from 'react';
import { AlertTriangle, ArrowUp, Ban, Check, CheckCircle2, ChevronDown, CircleDashed, FileDiff, Loader2, Pause, Play, RotateCcw, ShieldCheck, ShieldQuestion, Square, Target, X, XCircle } from 'lucide-react';
import { MISSION_STATUS_LABEL, TASK_STATUS_LABEL, canCancelMission, canCancelTask, canPauseMission, canResumeMission, canRetryTask, isTerminalMission, missionOutcome, missionTotals, outcomeLabel, taskOutcome, tokenText, type Outcome } from '../../lib/agents';
import { timeAgo } from '../../lib/logic';
import type { AgentTaskLog, Mission, MissionTask } from '../../lib/types';
import { useAgents } from '../../store/agents';
import { useApp } from '../../store/app';
import { api, errorMessage } from '../../lib/api';
import { Badge, Button, Callout, Modal } from '../../components/ui';
import ReviewView from '../workspace/ReviewView';

const WORKSPACE_NAME = 'Agent workspace';
const EXAMPLES = [
  'Create docs/overview.md that lists every file in this workspace with one line about each.',
  'Read the project and write a short README.md that explains what it is and how to run it.',
  'Find TODO comments in the code and collect them into docs/todo.md grouped by file.',
];

function OutcomeBadge({ outcome }: { outcome: Outcome }) {
  const tone = outcome === 'verified' ? 'ok' : outcome === 'checks_failed' || outcome === 'failed' ? 'bad' : outcome === 'running' || outcome === 'planning' ? 'run' : outcome === 'waiting' || outcome === 'completed_unverified' || outcome === 'blocked' ? 'warn' : 'idle';
  const Icon = outcome === 'verified' ? ShieldCheck : outcome === 'completed_unverified' ? ShieldQuestion : outcome === 'failed' || outcome === 'checks_failed' ? XCircle : outcome === 'cancelled' ? Ban : outcome === 'running' || outcome === 'planning' ? Loader2 : outcome === 'waiting' || outcome === 'blocked' ? AlertTriangle : CircleDashed;
  return (
    <Badge tone={tone} title={outcome === 'completed_unverified' ? 'The work finished, but no build or test confirmed it.' : undefined}>
      <Icon size={11} className={outcome === 'running' || outcome === 'planning' ? 'spin' : undefined} /> {outcomeLabel(outcome)}
    </Badge>
  );
}

function StepIcon({ t }: { t: MissionTask }) {
  const o = taskOutcome(t);
  if (o === 'running') return <Loader2 size={15} className="spin" color="var(--run)" aria-label="Running" />;
  if (o === 'verified' || o === 'completed_unverified') return <CheckCircle2 size={15} color={o === 'verified' ? 'var(--ok)' : 'var(--warn)'} aria-label={outcomeLabel(o)} />;
  if (o === 'failed' || o === 'checks_failed') return <XCircle size={15} color="var(--bad)" aria-label="Failed" />;
  if (o === 'waiting' || o === 'blocked') return <AlertTriangle size={15} color="var(--warn)" aria-label={outcomeLabel(o)} />;
  if (o === 'cancelled') return <Ban size={15} color="var(--text-3)" aria-label="Cancelled" />;
  return <CircleDashed size={15} color="var(--text-3)" aria-label={TASK_STATUS_LABEL[t.status]} />;
}

const LOG_LABEL: Record<AgentTaskLog['kind'], string> = { message: 'Said', tool: 'Tool', status: 'Status', result: 'Result', error: 'Error' };

function Activity({ mission }: { mission: Mission }) {
  const logs = useAgents((s) => s.logs);
  const loadLogs = useAgents((s) => s.loadLogs);
  const agentIds = useMemo(() => [...new Set(mission.tasks.map((t) => t.assigneeAgentId).filter((x): x is string => !!x))], [mission.tasks]);
  useEffect(() => {
    for (const id of agentIds) void loadLogs(id);
  }, [agentIds, loadLogs]);
  const mine = useMemo(() => agentIds.flatMap((id) => logs[id] ?? []).filter((l) => l.missionId === mission.id).sort((a, b) => a.at.localeCompare(b.at)), [agentIds, logs, mission.id]);
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => {
    end.current?.scrollIntoView?.({ block: 'nearest' });
  }, [mine.length]);
  if (!mine.length) return <p className="small faint" style={{ margin: 0 }}>{isTerminalMission(mission.status) ? 'No activity was recorded.' : 'Activity appears here as the agents work.'}</p>;
  return (
    <ol className="am-log" aria-label="Activity log">
      {mine.slice(-60).map((l) => (
        <li key={l.id} className={l.kind}>
          <span className="am-log-k">{LOG_LABEL[l.kind]}</span>
          <span className="am-log-t">{l.text}</span>
          <time className="faint tiny" dateTime={l.at}>{timeAgo(l.at)}</time>
        </li>
      ))}
      <div ref={end} />
    </ol>
  );
}

function ApprovalCard({ mission, task }: { mission: Mission; task: MissionTask }) {
  const taskAction = useAgents((s) => s.taskAction);
  const [busy, setBusy] = useState<string | null>(null);
  const [review, setReview] = useState(false);
  const act = async (kind: 'approve' | 'reject') => {
    setBusy(kind);
    await taskAction(kind, task.id);
    setBusy(null);
  };
  return (
    <div className="am-approval" role="region" aria-label="Needs your decision">
      <div className="row">
        <AlertTriangle size={16} color="var(--warn)" />
        <b className="grow">“{task.title}” is waiting for your decision</b>
      </div>
      <p className="small muted" style={{ margin: '4px 0 8px' }}>
        The agent worked in an isolated copy. Nothing in your workspace has changed yet.
        {task.changedPaths?.length ? <> It wants to change: <span className="mono">{task.changedPaths.join(', ')}</span>.</> : null}
      </p>
      <div className="row wrap">
        {task.changesetId && <Button size="sm" icon={<FileDiff size={13} />} onClick={() => setReview(true)}>Review changes</Button>}
        <Button size="sm" variant="ok" icon={<Check size={13} />} loading={busy === 'approve'} disabled={!!busy} onClick={() => void act('approve')}>Approve and apply</Button>
        <Button size="sm" variant="danger" icon={<X size={13} />} loading={busy === 'reject'} disabled={!!busy} onClick={() => void act('reject')}>Reject</Button>
      </div>
      {review && task.changesetId && (
        <Modal wide title={`Changes: ${task.title}`} onClose={() => setReview(false)} footer={<Button onClick={() => setReview(false)}>Close</Button>}>
          <div style={{ height: '60vh', minHeight: 300 }}>
            <ReviewView changesetId={task.changesetId} />
          </div>
        </Modal>
      )}
      <span className="sr-only">{mission.title}</span>
    </div>
  );
}

function Report({ mission }: { mission: Mission }) {
  const done = mission.tasks.filter((t) => t.status === 'done');
  const files = [...new Set(done.flatMap((t) => t.changedPaths ?? []))];
  const outcome = missionOutcome(mission);
  const notes = [...new Set(mission.tasks.map((t) => t.verification.notes).filter(Boolean))];
  return (
    <section className="am-report" aria-label="Final report">
      <div className="row wrap">
        <h3 style={{ margin: 0 }} className="grow">Report</h3>
        <OutcomeBadge outcome={outcome} />
      </div>
      {outcome === 'completed_unverified' && <Callout tone="warn" title="Finished, but not verified">The agents report the work as done, but no build or test confirmed it. Check the changed files yourself before relying on them.</Callout>}
      {outcome === 'verified' && <Callout tone="ok" title="Verified">A real build or test passed on the result.</Callout>}
      {(outcome === 'failed' || outcome === 'checks_failed') && <Callout tone="bad" title={outcome === 'checks_failed' ? 'Checks failed' : 'The mission did not complete'}>{mission.summary ?? 'See the failed step below.'}</Callout>}
      <dl className="am-facts">
        <dt>Steps</dt>
        <dd>{done.length} of {mission.tasks.length} completed</dd>
        <dt>Files changed</dt>
        <dd>{files.length ? <span className="mono">{files.join(', ')}</span> : 'none'}</dd>
        <dt>Build</dt>
        <dd>{[...new Set(mission.tasks.map((t) => t.verification.build))].join(', ') || 'unknown'}</dd>
        <dt>Tests</dt>
        <dd>{[...new Set(mission.tasks.map((t) => t.verification.tests))].join(', ') || 'unknown'}</dd>
      </dl>
      {notes.length > 0 && <p className="small muted" style={{ margin: 0 }}>{notes.join(' ')}</p>}
      {mission.summary && (
        <details className="am-raw">
          <summary>Full summary from the runtime</summary>
          <pre className="ag-result">{mission.summary}</pre>
        </details>
      )}
    </section>
  );
}

function MissionPanel({ mission }: { mission: Mission }) {
  const missionAction = useAgents((s) => s.missionAction);
  const taskAction = useAgents((s) => s.taskAction);
  const agents = useAgents((s) => s.agents);
  const [busy, setBusy] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);
  const totals = missionTotals(mission);
  const outcome = missionOutcome(mission);
  const waiting = mission.tasks.filter((t) => t.status === 'awaiting_approval');
  const act = async (kind: 'pause' | 'resume' | 'cancel') => {
    if (kind === 'cancel' && !window.confirm('Cancel this mission? Running steps are stopped; finished work is kept.')) return;
    setBusy(kind);
    await missionAction(kind, mission.id);
    setBusy(null);
  };
  const nameOf = (id: string | null) => (id ? (agents.find((a) => a.id === id)?.name ?? 'Agent') : 'Unassigned');

  return (
    <article className="am-mission" aria-label={`Mission: ${mission.title}`}>
      <header className="am-mhead">
        <div className="grow col" style={{ gap: 2, minWidth: 0 }}>
          <h2 className="ellipsis" style={{ margin: 0, fontSize: 17 }}>{mission.title}</h2>
          <span className="small muted">{MISSION_STATUS_LABEL[mission.status]} · updated {timeAgo(mission.updatedAt)} · {totals.calls} model calls · {tokenText(totals.tokens)} tokens</span>
        </div>
        <OutcomeBadge outcome={outcome} />
        {canPauseMission(mission) && <Button size="sm" icon={<Pause size={13} />} loading={busy === 'pause'} disabled={!!busy} onClick={() => void act('pause')}>Pause</Button>}
        {canResumeMission(mission) && <Button size="sm" variant="primary" icon={<Play size={13} />} loading={busy === 'resume'} disabled={!!busy} onClick={() => void act('resume')}>Resume</Button>}
        {canCancelMission(mission) && <Button size="sm" variant="danger" icon={<Square size={11} />} loading={busy === 'cancel'} disabled={!!busy} onClick={() => void act('cancel')}>Stop</Button>}
      </header>

      {waiting.map((t) => <ApprovalCard key={t.id} mission={mission} task={t} />)}

      <section aria-label="Steps">
        <h3 className="am-h">Steps</h3>
        {mission.tasks.length === 0 ? (
          <p className="small faint" style={{ margin: 0 }}>{mission.status === 'planning' ? 'The coordinator is planning the steps…' : mission.status === 'failed' ? 'Planning failed.' : 'No steps.'}</p>
        ) : (
          <ol className="am-steps">
            {mission.tasks.map((t) => (
              <li key={t.id} className={`am-step ${taskOutcome(t)}`}>
                <button type="button" className="am-step-head" aria-expanded={open === t.id} onClick={() => setOpen(open === t.id ? null : t.id)}>
                  <StepIcon t={t} />
                  <span className="grow ellipsis"><b>{t.title}</b> <span className="faint small">· {nameOf(t.assigneeAgentId)}</span></span>
                  <span className="small faint">{outcomeLabel(taskOutcome(t))}</span>
                  <ChevronDown size={13} style={{ transform: open === t.id ? 'rotate(180deg)' : undefined }} />
                </button>
                {open === t.id && (
                  <div className="am-step-body">
                    {t.description && <p className="small muted" style={{ margin: 0 }}>{t.description}</p>}
                    {t.blocker && <Callout tone="warn" title="Blocked">{t.blocker}</Callout>}
                    {t.result && <pre className="ag-result">{t.result}</pre>}
                    <div className="small faint">
                      {t.usage.calls} model calls · {tokenText(t.usage.tokens)} tokens
                      {t.changedPaths?.length ? ` · changed ${t.changedPaths.join(', ')}` : ''}
                    </div>
                    <div className="row wrap">
                      <Badge tone={t.verification.build === 'passed' ? 'ok' : t.verification.build === 'failed' ? 'bad' : 'idle'}>Build: {t.verification.build}</Badge>
                      <Badge tone={t.verification.tests === 'passed' ? 'ok' : t.verification.tests === 'failed' ? 'bad' : 'idle'}>Tests: {t.verification.tests}</Badge>
                      {t.acceptance?.checked && <Badge tone={t.acceptance.met ? 'ok' : 'warn'} title={t.acceptance.evidence}>Acceptance: {t.acceptance.met ? 'met' : 'not met'}</Badge>}
                    </div>
                    {t.verification.notes && <div className="small muted">{t.verification.notes}</div>}
                    <div className="row wrap">
                      {canRetryTask(t) && <Button size="sm" icon={<RotateCcw size={12} />} onClick={() => void taskAction('retry', t.id)}>Retry step</Button>}
                      {canCancelTask(t) && <Button size="sm" variant="ghost" onClick={() => void taskAction('cancel', t.id)}>Cancel step</Button>}
                    </div>
                  </div>
                )}
              </li>
            ))}
          </ol>
        )}
      </section>

      <section aria-label="Activity">
        <h3 className="am-h">Activity</h3>
        <Activity mission={mission} />
      </section>

      {isTerminalMission(mission.status) && <Report mission={mission} />}
    </article>
  );
}

/** The Agent Mode page: a mission composer on top, the selected mission below, recent missions on the side. */
export default function AgentMode() {
  const missions = useAgents((s) => s.missions);
  const projects = useAgents((s) => s.projects);
  const selectedId = useAgents((s) => s.selectedMissionId);
  const launchMission = useAgents((s) => s.launchMission);
  const set = useAgents((s) => s.set);
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const toast = useApp((s) => s.toast);
  const [goal, setGoal] = useState('');
  const [projectId, setProjectId] = useState<string>('new');
  const [busy, setBusy] = useState(false);
  const connected = !!keyStatus?.stored;
  const hasModel = connected && (!!settings?.model || settings?.modelMode === 'auto');
  const list = useMemo(() => [...missions].sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)), [missions]);
  const mission = missions.find((m) => m.id === selectedId) ?? null;
  const box = useRef<HTMLTextAreaElement>(null);
  const canStart = goal.trim().length >= 8 && hasModel && !busy;

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [goal]);

  const start = async () => {
    if (!canStart) return;
    setBusy(true);
    try {
      let pid = projectId;
      if (pid === 'new') {
        const existing = projects.find((p) => p.name === WORKSPACE_NAME);
        pid = existing ? existing.id : (await api.createProject(WORKSPACE_NAME, 'The scratch workspace Agent Mode works in. You can import files into it or choose another project.')).id;
        await useAgents.getState().load();
      }
      const m = await launchMission(pid, goal.trim(), null);
      if (m) setGoal('');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="am-root">
      <div className="am-main scroll">
        <div className="am-col">
          <section className="am-compose" aria-label="New mission">
            <h1>What should the agents do?</h1>
            <p className="muted" style={{ margin: '0 0 12px' }}>Describe a goal. Genius plans the steps, runs them with real tools in an isolated copy of the workspace, and asks you before anything is applied.</p>
            <div className="gx-composer">
              <textarea
                ref={box}
                rows={2}
                aria-label="Mission"
                placeholder="e.g. Create docs/overview.md that lists every file with one line about each"
                value={goal}
                onChange={(e) => setGoal(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
                    e.preventDefault();
                    void start();
                  }
                }}
              />
              <div className="gx-tools">
                <label className="gx-pill" title="Where the agents work. A new workspace is created once and reused.">
                  <span className="sr-only">Workspace</span>
                  <select aria-label="Workspace" className="am-select" value={projectId} onChange={(e) => setProjectId(e.target.value)}>
                    <option value="new">{WORKSPACE_NAME} (scratch)</option>
                    {projects.filter((p) => p.name !== WORKSPACE_NAME).map((p) => (
                      <option key={p.id} value={p.id}>{p.name}</option>
                    ))}
                  </select>
                </label>
                <span className="small faint">Ctrl+Enter to start</span>
                <span className="spacer" />
                <button className="send-btn" aria-label="Start mission" title={hasModel ? 'Plan and start' : 'Connect a provider in Settings first'} disabled={!canStart} onClick={() => void start()}>
                  {busy ? <Loader2 size={16} className="spin" /> : <ArrowUp size={16} />}
                </button>
              </div>
            </div>
            {!hasModel && <Callout tone="warn" title="Connect a provider first">Agent Mode needs your own provider key. Add it in Settings → AI provider.</Callout>}
            {!mission && (
              <div className="gx-ideas" aria-label="Examples">
                {EXAMPLES.map((e) => (
                  <button key={e} className="gx-idea" onClick={() => setGoal(e)}>
                    <span className="gx-idea-ic"><Target size={15} /></span>
                    <span className="gx-idea-h" style={{ fontSize: 12.5, color: 'var(--text-2)' }}>{e}</span>
                  </button>
                ))}
              </div>
            )}
          </section>
          {mission && <MissionPanel key={mission.id} mission={mission} />}
        </div>
      </div>
      <aside className="am-side" aria-label="Recent missions">
        <h3 className="am-h">Missions</h3>
        {list.length === 0 && <p className="small faint">No missions yet.</p>}
        <ul>
          {list.map((m) => (
            <li key={m.id}>
              <button type="button" className={`am-mitem ${m.id === selectedId ? 'on' : ''}`} aria-current={m.id === selectedId ? 'true' : undefined} onClick={() => set({ selectedMissionId: m.id, selectedTaskId: null })}>
                <span className="grow col" style={{ gap: 2, minWidth: 0, textAlign: 'left' }}>
                  <b className="ellipsis">{m.title}</b>
                  <span className="tiny faint">{timeAgo(m.updatedAt)}</span>
                </span>
                <OutcomeBadge outcome={missionOutcome(m)} />
              </button>
            </li>
          ))}
        </ul>
      </aside>
    </div>
  );
}
