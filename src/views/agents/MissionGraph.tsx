// The mission's task graph, drawn from real tasks: an SVG DAG (nodes = tasks, edges = dependsOn,
// colour = real status) and a status board. Only running/reviewing tasks animate.
import { useMemo, type KeyboardEvent } from 'react';
import { BOARD_COLUMNS, NODE_H, NODE_W, TASK_STATUS_LABEL, layoutGraph, taskTone } from '../../lib/agents';
import type { MissionTask } from '../../lib/types';
import { Badge } from '../../components/ui';

const isLive = (t: MissionTask) => t.status === 'running' || t.status === 'reviewing';
const clip = (s: string, n: number) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

interface Props {
  tasks: MissionTask[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  agentName: (id: string | null) => string;
}

export function TaskGraph({ tasks, selectedId, onSelect, agentName }: Props) {
  const g = useMemo(() => layoutGraph(tasks), [tasks]);
  const onKey = (id: string) => (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onSelect(id);
    }
  };
  return (
    <div className="mg-graph scroll" role="group" aria-label="Task dependency graph">
      <svg width={g.width} height={g.height} viewBox={`0 0 ${g.width} ${g.height}`} role="presentation">
        <defs>
          <marker id="mg-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" className="mg-arrow" />
          </marker>
        </defs>
        {g.edges.map((e) => (
          <path key={`${e.from}>${e.to}`} d={e.path} className="mg-edge" markerEnd="url(#mg-arrow)" data-from={e.from} data-to={e.to} />
        ))}
        {g.nodes.map(({ task: t, x, y }) => (
          <g
            key={t.id}
            transform={`translate(${x},${y})`}
            className={`mg-node s-${t.status} ${selectedId === t.id ? 'sel' : ''} ${isLive(t) ? 'live' : ''}`}
            role="button"
            tabIndex={0}
            aria-pressed={selectedId === t.id}
            aria-label={`Task ${t.title}, ${TASK_STATUS_LABEL[t.status]}${t.assigneeAgentId ? `, assigned to ${agentName(t.assigneeAgentId)}` : ''}`}
            data-task-id={t.id}
            data-status={t.status}
            onClick={() => onSelect(t.id)}
            onKeyDown={onKey(t.id)}
          >
            <rect width={NODE_W} height={NODE_H} rx={10} className="mg-box" />
            <rect width={5} height={NODE_H} rx={2} className="mg-stripe" />
            <text x={14} y={22} className="mg-title">{clip(t.title, 24)}</text>
            <text x={14} y={42} className="mg-sub">{clip(`${agentName(t.assigneeAgentId)} · ${TASK_STATUS_LABEL[t.status]}`, 30)}</text>
          </g>
        ))}
      </svg>
    </div>
  );
}

export function TaskBoard({ tasks, selectedId, onSelect, agentName }: Props) {
  return (
    <div className="mg-board" role="group" aria-label="Task board">
      {BOARD_COLUMNS.map((col) => {
        const items = tasks.filter((t) => col.statuses.includes(t.status)).sort((a, b) => b.priority - a.priority);
        return (
          <section key={col.key} className="mg-col" aria-label={col.label}>
            <h4>{col.label} <span className="faint">{items.length}</span></h4>
            {items.length === 0 && <div className="tiny faint">Nothing here</div>}
            {items.map((t) => (
              <button key={t.id} type="button" className={`mg-card s-${t.status} ${selectedId === t.id ? 'sel' : ''} ${isLive(t) ? 'live' : ''}`} aria-pressed={selectedId === t.id} onClick={() => onSelect(t.id)} data-task-id={t.id}>
                <span className="ellipsis" style={{ fontWeight: 600 }}>{t.title}</span>
                <span className="row small muted">
                  <span className="grow ellipsis">{agentName(t.assigneeAgentId)}</span>
                  <Badge tone={taskTone(t.status)}>{TASK_STATUS_LABEL[t.status]}</Badge>
                </span>
                {t.blocker && <span className="small" style={{ color: 'var(--warn)' }}>{t.blocker}</span>}
              </button>
            ))}
          </section>
        );
      })}
    </div>
  );
}
