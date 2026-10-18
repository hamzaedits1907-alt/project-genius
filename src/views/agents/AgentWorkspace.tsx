// The unified Agent Workspace: agent list with live status on the left, the selected agent's
// profile, history, activity log, usage and "give a task" box on the right.
import { useEffect, useMemo, useState } from 'react';
import { Bot, Copy, Pencil, Plus, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { APPROVAL_LABEL, BUDGET_LABELS, TASK_STATUS_LABEL, memoryPolicyLabel, taskTone, tokenText } from '../../lib/agents';
import { timeAgo } from '../../lib/logic';
import type { AgentTaskLog, CustomAgent, Memory } from '../../lib/types';
import { runtimeOf, useAgents } from '../../store/agents';
import { useApp } from '../../store/app';
import { Badge, Button, Callout, Empty } from '../../components/ui';
import { AgentAvatar, AgentStatusBadge, Section } from './parts';
import RunTask from './RunTask';

export function UsageBar({ label, used, max, unit }: { label: string; used: number; max: number; unit?: string }) {
  const pct = max > 0 ? Math.min(100, Math.round((used / max) * 100)) : 0;
  return (
    <div className="usage-bar">
      <div className="row small">
        <span className="grow">{label}</span>
        <span className="muted">{used.toLocaleString()} / {max.toLocaleString()} {unit}</span>
      </div>
      <div className="meter" role="progressbar" aria-label={label} aria-valuemin={0} aria-valuemax={max} aria-valuenow={Math.min(used, max)}>
        <span className={pct >= 90 ? 'hot' : ''} style={{ width: `${pct}%` }} />
      </div>
    </div>
  );
}

function AgentListItem({ agent }: { agent: CustomAgent }) {
  const runtimes = useAgents((s) => s.runtimes);
  const missions = useAgents((s) => s.missions);
  const selected = useAgents((s) => s.selectedAgentId === agent.id);
  const set = useAgents((s) => s.set);
  const rt = runtimeOf(runtimes, agent.id);
  const current = rt.currentTaskId ? missions.flatMap((m) => m.tasks).find((t) => t.id === rt.currentTaskId) : undefined;
  return (
    <li>
      <button type="button" className={`ag-item ${selected ? 'on' : ''}`} aria-current={selected ? 'true' : undefined} onClick={() => set({ selectedAgentId: agent.id })} data-agent-id={agent.id}>
        <AgentAvatar avatar={agent.avatar} status={rt.status} size={38} />
        <span className="grow col" style={{ gap: 2, minWidth: 0 }}>
          <span className="row" style={{ gap: 6 }}>
            <b className="ellipsis grow">{agent.name}</b>
            {agent.builtin && <Badge tone="idle">built-in</Badge>}
          </span>
          <span className="small muted ellipsis">{current ? current.title : agent.role}</span>
        </span>
        <AgentStatusBadge status={rt.status} />
      </button>
    </li>
  );
}

export function AgentList({ onCreate }: { onCreate: () => void }) {
  const agents = useAgents((s) => s.agents);
  return (
    <div className="ag-list">
      <div className="row" style={{ padding: '10px 12px' }}>
        <span className="panel-title grow">Agents</span>
        <Button size="sm" icon={<Plus size={13} />} onClick={onCreate}>New</Button>
      </div>
      {agents.length === 0 ? (
        <div className="small muted" style={{ padding: 12 }}>No agents yet. Create one or start from a template.</div>
      ) : (
        <ul className="ag-items" aria-label="Agents">
          {agents.map((a) => (
            <AgentListItem key={a.id} agent={a} />
          ))}
        </ul>
      )}
    </div>
  );
}

function ActivityLog({ agentId }: { agentId: string }) {
  const rows = useAgents((s) => s.logs[agentId]) as AgentTaskLog[] | undefined;
  const loadLogs = useAgents((s) => s.loadLogs);
  useEffect(() => {
    void loadLogs(agentId);
  }, [agentId, loadLogs]);
  if (!rows || rows.length === 0) return <div className="small muted">No activity recorded for this agent yet.</div>;
  return (
    <div className="ag-log" role="log" aria-label="Activity log" tabIndex={0}>
      {rows.map((l) => (
        <div key={l.id} className={`ag-log-row ${l.kind}`}>
          <span className="tiny faint">{new Date(l.at).toLocaleTimeString()}</span>
          <Badge tone={l.kind === 'error' ? 'bad' : l.kind === 'result' ? 'ok' : l.kind === 'tool' ? 'accent' : 'idle'}>{l.kind}</Badge>
          <span className="grow" style={{ overflowWrap: 'anywhere' }}>{l.text}</span>
        </div>
      ))}
    </div>
  );
}

function MemoryBlock({ agent, projectId }: { agent: CustomAgent; projectId: string | null }) {
  const navigate = useApp((s) => s.navigate);
  const [items, setItems] = useState<Memory[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (agent.memoryPolicy !== 'project' || !projectId) {
      setItems(null);
      return;
    }
    let alive = true;
    api
      .listMemories(projectId)
      .then((m) => alive && (setItems(m), setErr(null)))
      .catch((e) => alive && setErr(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [agent.memoryPolicy, projectId]);
  if (agent.memoryPolicy === 'none') return <div className="small muted">This agent does not keep any memory between tasks.</div>;
  if (agent.memoryPolicy === 'agent') return <div className="small muted">This agent keeps its own notes between tasks. They are managed by the agent runtime and are not shown here yet.</div>;
  if (!projectId) return <div className="small muted">This agent reads the shared project memory. Run a task in a project to see its memory here.</div>;
  return (
    <div className="col" style={{ gap: 6 }}>
      {err && <div className="small" style={{ color: 'var(--bad)' }}>Could not load project memory: {err}</div>}
      {items && items.length === 0 && <div className="small muted">The project has no saved memories yet.</div>}
      {items?.slice(0, 5).map((m) => (
        <div key={m.id} className="small"><Badge>{m.kind}</Badge> {m.content}</div>
      ))}
      <div className="row">
        <Button size="sm" onClick={() => navigate({ name: 'project', id: projectId })}>Open project (Memory panel)</Button>
      </div>
    </div>
  );
}

export function AgentDetail({ agent, advanced, onEdit, onDuplicate }: { agent: CustomAgent; advanced: boolean; onEdit: (a: CustomAgent) => void; onDuplicate: (a: CustomAgent) => void }) {
  const runtimes = useAgents((s) => s.runtimes);
  const missions = useAgents((s) => s.missions);
  const deleteAgent = useAgents((s) => s.deleteAgent);
  const set = useAgents((s) => s.set);
  const rt = runtimeOf(runtimes, agent.id);
  const allTasks = useMemo(() => missions.flatMap((m) => m.tasks.map((t) => ({ t, m }))), [missions]);
  const history = useMemo(
    () => allTasks.filter((x) => x.t.assigneeAgentId === agent.id).sort((a, b) => (b.t.startedAt ?? b.m.updatedAt).localeCompare(a.t.startedAt ?? a.m.updatedAt)),
    [allTasks, agent.id],
  );
  const current = rt.currentTaskId ? allTasks.find((x) => x.t.id === rt.currentTaskId)?.t : undefined;
  const lastProject = history[0]?.m.projectId ?? null;
  const [confirmDelete, setConfirmDelete] = useState(false);

  return (
    <div className="ag-detail">
      <div className="ag-head">
        <AgentAvatar avatar={agent.avatar} status={rt.status} size={64} label={`${agent.name} is ${rt.status.replace(/_/g, ' ')}`} />
        <div className="grow col" style={{ gap: 4 }}>
          <div className="row wrap">
            <h2 style={{ margin: 0, fontSize: 20 }}>{agent.name}</h2>
            <AgentStatusBadge status={rt.status} />
            {agent.builtin && <Badge tone="idle">built-in</Badge>}
          </div>
          <div className="muted">{agent.role}</div>
          {current ? <div className="small">Working on: <b>{current.title}</b></div> : <div className="small faint">{rt.lastActivityAt ? `Last active ${timeAgo(rt.lastActivityAt)}` : 'No activity yet'}</div>}
        </div>
        <div className="row wrap">
          {agent.builtin ? (
            <Button icon={<Copy size={13} />} onClick={() => onDuplicate(agent)}>Duplicate to customize</Button>
          ) : (
            <>
              <Button icon={<Pencil size={13} />} onClick={() => onEdit(agent)}>Edit</Button>
              <Button icon={<Copy size={13} />} onClick={() => onDuplicate(agent)}>Duplicate</Button>
              {confirmDelete ? (
                <>
                  <Button variant="danger" onClick={() => void deleteAgent(agent.id).then((ok) => ok && setConfirmDelete(false))}>Confirm delete</Button>
                  <Button variant="ghost" onClick={() => setConfirmDelete(false)}>Keep</Button>
                </>
              ) : (
                <Button variant="ghost" icon={<Trash2 size={13} />} onClick={() => setConfirmDelete(true)} aria-label={`Delete ${agent.name}`}>Delete</Button>
              )}
            </>
          )}
        </div>
      </div>

      <div className="ag-cols">
        <div className="col" style={{ gap: 18 }}>
          <Section title="Give this agent a task">
            <RunTask agentId={agent.id} agentName={agent.name} />
          </Section>

          <Section title="Task history">
            {history.length === 0 ? (
              <div className="small muted">{agent.name} has not worked on any task yet.</div>
            ) : (
              <ul className="ag-history" aria-label="Task history">
                {history.slice(0, 12).map(({ t, m }) => (
                  <li key={t.id}>
                    <button type="button" className="ag-hist-row" onClick={() => set({ tab: 'missions', selectedMissionId: m.id, selectedTaskId: t.id })}>
                      <span className="grow col" style={{ gap: 1, minWidth: 0, textAlign: 'left' }}>
                        <span className="ellipsis">{t.title}</span>
                        <span className="tiny faint ellipsis">{m.title}</span>
                      </span>
                      <span className="tiny faint">{tokenText(t.usage.tokens)} tok</span>
                      <Badge tone={taskTone(t.status)}>{TASK_STATUS_LABEL[t.status]}</Badge>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Section>
        </div>

        <div className="col" style={{ gap: 18 }}>
          <Section title="About">
            {agent.description && <p style={{ margin: 0 }}>{agent.description}</p>}
            {agent.specialization.length > 0 && (
              <ul className="chips" aria-label="Specializations">
                {agent.specialization.map((s) => (
                  <li key={s} className="chip">{s}</li>
                ))}
              </ul>
            )}
            <details>
              <summary>Instructions</summary>
              <pre className="ag-result">{agent.instructions}</pre>
            </details>
            <div className="small muted">
              Approval: {APPROVAL_LABEL[agent.approval]} · Model: {agent.modelPolicy === 'manual' ? agent.model : agent.modelPolicy === 'max' ? 'strongest available' : 'automatic'} · {memoryPolicyLabel(agent.memoryPolicy)}
            </div>
          </Section>

          {advanced && (
            <>
              <Section title="Usage" hint={current ? `Current task: ${current.title}` : 'Shown against the per-task budget while a task is running.'}>
                <UsageBar label="Tokens (this task)" used={current?.usage.tokens ?? 0} max={agent.budget.maxTokens} unit="tokens" />
                <UsageBar label="Model calls (this task)" used={current?.usage.calls ?? 0} max={agent.budget.maxModelCalls} unit="calls" />
                <div className="small muted">
                  All time: {rt.usage.tokens.toLocaleString()} tokens · {rt.usage.calls} calls · {rt.usage.tasksDone} done · {rt.usage.tasksFailed} failed
                </div>
              </Section>

              <Section title="Permissions">
                <div className="small col" style={{ gap: 4 }}>
                  <div>Tools: <span className="mono">{agent.tools.join(', ') || 'none'}</span></div>
                  <div>Scope: {agent.scope.projects === 'current' ? 'the task’s project' : 'any project'}{agent.scope.paths.length ? ` — ${agent.scope.paths.join(', ')}` : ' — whole project'}</div>
                  <div>Network: {agent.network ? 'allowed' : 'off'}</div>
                  <div>
                    Budget per task: {(Object.keys(BUDGET_LABELS) as (keyof typeof BUDGET_LABELS)[]).map((k) => `${BUDGET_LABELS[k].label} ${agent.budget[k].toLocaleString()}`).join(' · ')} · retries {agent.retryLimit}
                  </div>
                  {agent.knowledgeSources.length > 0 && <div>Knowledge: {agent.knowledgeSources.join(', ')}</div>}
                </div>
              </Section>
            </>
          )}

          <Section title="Memory">
            <MemoryBlock agent={agent} projectId={lastProject} />
          </Section>

          {advanced && (
            <Section title="Activity log">
              <ActivityLog agentId={agent.id} />
            </Section>
          )}
        </div>
      </div>
    </div>
  );
}

export default function AgentWorkspace({ advanced, onCreate, onEdit, onDuplicate }: { advanced: boolean; onCreate: () => void; onEdit: (a: CustomAgent) => void; onDuplicate: (a: CustomAgent) => void }) {
  const agents = useAgents((s) => s.agents);
  const selectedId = useAgents((s) => s.selectedAgentId);
  const loaded = useAgents((s) => s.loaded);
  const loadError = useAgents((s) => s.loadError);
  const load = useAgents((s) => s.load);
  const agent = agents.find((a) => a.id === selectedId) ?? agents[0];

  return (
    <div className="ag-split">
      <AgentList onCreate={onCreate} />
      <div className="ag-main scroll">
        {loadError && (
          <div style={{ padding: '12px 16px' }}>
            <Callout tone="bad" title="Agents are not fully available">
              {loadError}
              <div style={{ marginTop: 8 }}>
                <Button size="sm" onClick={() => void load()}>Retry</Button>
              </div>
            </Callout>
          </div>
        )}
        {agent ? (
          <AgentDetail key={agent.id} agent={agent} advanced={advanced} onEdit={onEdit} onDuplicate={onDuplicate} />
        ) : (
          <Empty icon={<Bot size={20} />} title={loaded ? 'No agents yet' : 'Loading agents…'} action={loaded ? <Button variant="primary" onClick={onCreate}>Create your first agent</Button> : undefined}>
            Agents are specialists you can give tasks to, like a designer, tester or researcher.
          </Empty>
        )}
      </div>
    </div>
  );
}
