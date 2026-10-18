// "Agents at a glance": one node per agent with its real status and current task.
// Driven only by agent_runtimes + live events; nothing here is simulated.
import { runtimeOf, useAgents } from '../../store/agents';
import { AGENT_STATUS_LABEL, isAnimatedStatus } from '../../lib/agents';
import { AgentAvatar } from './parts';

export default function AgentsGlance({ onSelect }: { onSelect?: (agentId: string) => void }) {
  const agents = useAgents((s) => s.agents);
  const runtimes = useAgents((s) => s.runtimes);
  const missions = useAgents((s) => s.missions);
  if (agents.length === 0) return null;
  const active = agents.filter((a) => isAnimatedStatus(runtimeOf(runtimes, a.id).status)).length;
  return (
    <section aria-label="Agents at a glance" className="glance">
      <div className="row" style={{ marginBottom: 8 }}>
        <span className="panel-title grow">Agents at a glance</span>
        <span className="small muted" aria-live="polite">{active === 0 ? 'All quiet' : `${active} active`}</span>
      </div>
      <ul className="glance-grid">
        {agents.map((a) => {
          const rt = runtimeOf(runtimes, a.id);
          const task = rt.currentTaskId ? missions.flatMap((m) => m.tasks).find((t) => t.id === rt.currentTaskId) : undefined;
          return (
            <li key={a.id}>
              <button type="button" className={`glance-node tone-${rt.status}`} onClick={() => onSelect?.(a.id)} data-status={rt.status} aria-label={`${a.name}: ${AGENT_STATUS_LABEL[rt.status]}${task ? `, ${task.title}` : ''}`}>
                <AgentAvatar avatar={a.avatar} status={rt.status} size={34} />
                <span className="col" style={{ gap: 0, minWidth: 0, textAlign: 'left' }}>
                  <b className="ellipsis">{a.name}</b>
                  <span className="tiny muted ellipsis">{task ? task.title : AGENT_STATUS_LABEL[rt.status]}</span>
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
