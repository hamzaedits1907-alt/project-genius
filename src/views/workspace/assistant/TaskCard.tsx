import { useState, type ReactNode } from 'react';
import { Bot, Brain, Check, ChevronDown, ChevronRight, Circle, CircleDot, Eye, FilePen, FilePlus, FileSearch, FileText, Files, FolderTree, GitBranch, Globe, Hammer, ListChecks, Package, Search, Square, Wrench } from 'lucide-react';
import { api, errorMessage } from '../../../lib/api';
import { bytes } from '../../../lib/logic';
import { AGENTS, groupSteps, latestNote, latestTodo, stepLabel, type StepView } from '../../../lib/agent';
import type { AgentStep, TaskInfo } from '../../../lib/types';
import { useApp } from '../../../store/app';
import { useWorkspace } from '../../../store/workspace';
import { Badge, Button } from '../../../components/ui';
import { GeniusAvatar } from '../../../components/GeniusAvatar';
import { Thinking } from './Thinking';

const TOOL_ICON: Record<string, ReactNode> = {
  read_file: <FileText size={12} />,
  read_reference: <FileText size={12} />,
  list_files: <FolderTree size={12} />,
  project_facts: <FolderTree size={12} />,
  search: <Search size={12} />,
  search_references: <FileSearch size={12} />,
  web_search: <Globe size={12} />,
  fetch_url: <Globe size={12} />,
  write_file: <FilePlus size={12} />,
  write_files: <Files size={12} />,
  edit_file: <FilePen size={12} />,
  run_check: <Hammer size={12} />,
  last_run_output: <Hammer size={12} />,
  preview_errors: <Eye size={12} />,
  add_dependency: <Package size={12} />,
  delegate: <Bot size={12} />,
  remember: <Brain size={12} />,
  memory: <Brain size={12} />,
  git_status: <GitBranch size={12} />,
  git_diff: <GitBranch size={12} />,
  recent_changes: <GitBranch size={12} />,
  todo: <ListChecks size={12} />,
};

function AgentBadge({ agent }: { agent: string }) {
  const a = AGENTS[agent] ?? { label: agent, hint: '' };
  return (
    <span className={`agent-badge agent-${agent}`} title={a.hint}>
      {a.label}
    </span>
  );
}

function StepRow({ v }: { v: StepView }) {
  const s = v.step;
  return (
    <li className={`run-step ${s.ok ? '' : 'bad-text'}`} title={JSON.stringify(s.args)}>
      <span className="run-step-icon" aria-hidden>
        {TOOL_ICON[s.tool] ?? <Wrench size={12} />}
      </span>
      <AgentBadge agent={s.agent} /> <span className="grow">{stepLabel(s)}</span>
      {v.retries > 0 && (
        <span className="faint tiny" title="Earlier attempts failed and were retried">
          retried{v.retries > 1 ? ` ×${v.retries}` : ''}
        </span>
      )}
    </li>
  );
}

/** Real tool calls, grouped by model turn, with failed-then-retried attempts folded away. */
export function StepList({ steps }: { steps: AgentStep[] | undefined }) {
  const groups = groupSteps(steps);
  return (
    <ol className="run-steps" aria-label="Steps">
      {groups.map((g) =>
        g.items.length === 1 ? (
          <StepRow key={g.items[0].step.id} v={g.items[0]} />
        ) : (
          <li key={g.items[0].step.id} className="run-step-group">
            <div className="faint tiny">{g.items.length} actions at once</div>
            <ul>
              {g.items.map((v) => (
                <StepRow key={v.step.id} v={v} />
              ))}
            </ul>
          </li>
        ),
      )}
    </ol>
  );
}

/** Live view of a running task: what Genius says it's doing, its checklist, real tool calls and model output. */
export function TaskCard({ task, expanded, name }: { task: TaskInfo; expanded: boolean; name: string }) {
  const steps = useWorkspace((s) => s.agentSteps[task.id]) as AgentStep[] | undefined;
  const activity = useWorkspace((s) => s.activity);
  const stream = useWorkspace((s) => s.aiStream[task.id]);
  const thinking = useWorkspace((s) => !!s.thinking[task.id]);
  const drafts = useWorkspace((s) => s.drafts);
  const openTab = useWorkspace((s) => s.openTab);
  const setWs = useWorkspace((s) => s.set);
  const toast = useApp((s) => s.toast);
  const [showRaw, setShowRaw] = useState(false);
  const [showSteps, setShowSteps] = useState(expanded);
  const events = activity.filter((e) => e.taskId === task.id && (e.kind === 'ai' || e.kind === 'approval' || e.kind === 'build' || e.kind === 'test' || e.kind === 'install' || (e.kind === 'task' && e.title.includes('subagent'))));
  const writing = Object.values(drafts).filter((d) => d.taskId === task.id);
  const current = writing.find((d) => d.status === 'streaming');
  const lastAi = events.filter((e) => e.kind === 'ai').slice(-1)[0];
  const todo = latestTodo(steps);
  const note = latestNote(steps);
  const groups = groupSteps(steps);
  const count = groups.reduce((n, g) => n + g.items.length, 0);
  const last = groups[groups.length - 1]?.items.slice(-1)[0]?.step;
  const title = task.kind === 'plan' ? `${name} is planning your app` : task.kind === 'generate' ? `${name} is writing your app` : task.title.replace(/^Genius: /, '');
  return (
    <div className="msg-row fade-in">
      <GeniusAvatar size={26} mood={current ? 'working' : 'thinking'} className="msg-avatar" />
      <div className="task-card live" aria-label={`Running: ${task.title}`}>
        <div className="row">
          <strong className="grow ellipsis">{title}</strong>
          <Button size="sm" variant="ghost" icon={<Square size={11} />} onClick={() => void api.cancelTask(task.id).catch((e) => toast('bad', errorMessage(e)))}>
            Stop
          </Button>
        </div>
        <Thinking taskId={task.id} live />
        {note ? <div className="task-note">{note}</div> : !thinking && lastAi && lastAi.status === 'running' && <div className="small faint thinking-dots">Thinking</div>}
        {todo.length > 0 && (
          <ul className="todo" aria-label="Genius's plan">
            {todo.map((t, i) => (
              <li key={i} className={t.status}>
                {t.status === 'done' ? <Check size={13} /> : t.status === 'active' ? <CircleDot size={13} /> : <Circle size={13} />}
                <span>{t.text}</span>
              </li>
            ))}
          </ul>
        )}
        {count > 0 && last && (
          <>
            <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowSteps(!showSteps)} aria-expanded={showSteps}>
              {showSteps ? <ChevronDown size={12} /> : <ChevronRight size={12} />} {count} step{count === 1 ? '' : 's'} · last: {last.tool.replace(/_/g, ' ')}
            </button>
            {showSteps && <StepList steps={steps} />}
          </>
        )}
        {events
          .filter((e) => e.kind !== 'ai')
          .slice(-3)
          .map((e) => (
            <div key={e.id} className="small row" style={{ gap: 6 }}>
              <Badge tone={e.status === 'completed' ? 'ok' : e.status === 'failed' ? 'bad' : 'run'}>{e.status.replace('_', ' ')}</Badge>
              <span className="ellipsis">{e.title}</span>
            </div>
          ))}
        {writing.length > 0 && (
          <div className="files-written">
            {writing.map((d) => (
              <button
                key={d.path}
                className={`file-pill ${d.status}`}
                onClick={() => {
                  openTab(`draft:${d.path}`);
                  setWs({ stage: 'code' });
                }}
                title={d.status === 'streaming' ? 'Being written now (not saved yet)' : d.path}
              >
                {d.status === 'streaming' ? <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} /> : <Check size={11} />}
                {d.path.split('/').pop()}
              </button>
            ))}
          </div>
        )}
        {stream && expanded && (
          <>
            <button className="btn ghost sm" style={{ alignSelf: 'flex-start' }} onClick={() => setShowRaw(!showRaw)}>
              {showRaw ? <ChevronDown size={12} /> : <ChevronRight size={12} />} Raw model output ({bytes(stream.length)})
            </button>
            {showRaw && <pre className="stream">{stream.slice(-20000)}</pre>}
          </>
        )}
      </div>
    </div>
  );
}
