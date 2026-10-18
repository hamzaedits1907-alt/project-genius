// Top-level "Agents" route: Agent Workspace, Mission Control and Teams, with a friendly
// Simple-mode home (templates, create an agent, run a task).
import { useEffect, useState } from 'react';
import { Bot, Copy, Pencil, Plus, RefreshCw, Sparkles } from 'lucide-react';
import { blankAgent, duplicateAgent, fromTemplate, memoryPolicyLabel } from '../../lib/agents';
import type { AgentDraft, CustomAgent } from '../../lib/types';
import { runtimeOf, useAgents, type AgentsTab } from '../../store/agents';
import { useApp } from '../../store/app';
import { Badge, Button, Callout, Modal } from '../../components/ui';
import AgentCreator from './AgentCreator';
import AgentWorkspace from './AgentWorkspace';
import AgentsGlance from './AgentsGlance';
import MissionControl from './MissionControl';
import RunTask from './RunTask';
import TeamsManager from './TeamsManager';
import { AgentAvatar, AgentStatusBadge } from './parts';

function NewAgentChooser({ onPick, onClose }: { onPick: (d: AgentDraft) => void; onClose: () => void }) {
  const templates = useAgents((s) => s.templates);
  const agents = useAgents((s) => s.agents);
  return (
    <Modal title="Create an agent" onClose={onClose}>
      <div className="col" style={{ gap: 14 }}>
        <button type="button" className="ag-template" onClick={() => onPick(blankAgent())}>
          <span className="ag-emoji" style={{ fontSize: 24 }}>＋</span>
          <span className="col" style={{ gap: 2, textAlign: 'left' }}>
            <b>Blank agent</b>
            <span className="small muted">Start from scratch with safe defaults.</span>
          </span>
        </button>
        {templates.length > 0 && <div className="panel-title">Start from a template</div>}
        <div className="ag-templates">
          {templates.map((t) => (
            <button key={t.key} type="button" className="ag-template" onClick={() => onPick(fromTemplate(t))}>
              <AgentAvatar avatar={t.avatar} size={36} />
              <span className="col" style={{ gap: 2, textAlign: 'left', minWidth: 0 }}>
                <b>{t.name}</b>
                <span className="small muted">{t.description}</span>
              </span>
            </button>
          ))}
        </div>
        {agents.some((a) => a.builtin) && <div className="small muted">Tip: built-in agents can be duplicated from the Agents list to customise them.</div>}
      </div>
    </Modal>
  );
}

function SimpleAgents({ onCreate, onPick, onEdit, onDuplicate }: { onCreate: () => void; onPick: (d: AgentDraft) => void; onEdit: (a: CustomAgent) => void; onDuplicate: (a: CustomAgent) => void }) {
  const agents = useAgents((s) => s.agents);
  const templates = useAgents((s) => s.templates);
  const runtimes = useAgents((s) => s.runtimes);
  const set = useAgents((s) => s.set);
  const setUiMode = useApp((s) => s.setUiMode);
  const [running, setRunning] = useState<string | null>(null);
  return (
    <div className="page-inner">
      <div className="hero">
        <div className="col grow" style={{ gap: 6 }}>
          <h1>Your agents</h1>
          <p className="lead">Agents are specialists — a designer, a tester, a researcher — that you can give tasks to. They show you what they changed before anything is applied.</p>
          <div className="row wrap" style={{ marginTop: 8 }}>
            <Button variant="primary" size="lg" icon={<Plus size={15} />} onClick={onCreate}>Create an agent</Button>
            <Button size="lg" onClick={() => set({ tab: 'missions' })}>Missions</Button>
          </div>
        </div>
      </div>

      <div style={{ marginTop: 18 }}><AgentsGlance onSelect={(id) => setRunning(id)} /></div>

      {agents.length > 0 && (
        <>
          <div className="section-title"><h2>Run a task</h2></div>
          <ul className="ag-cards" aria-label="Your agents">
            {agents.map((a) => {
              const rt = runtimeOf(runtimes, a.id);
              return (
                <li key={a.id} className="card pad col" style={{ gap: 10 }}>
                  <div className="row">
                    <AgentAvatar avatar={a.avatar} status={rt.status} size={44} />
                    <div className="grow col" style={{ gap: 2, minWidth: 0 }}>
                      <b className="ellipsis">{a.name}</b>
                      <span className="small muted ellipsis">{a.role}</span>
                    </div>
                    <AgentStatusBadge status={rt.status} />
                  </div>
                  {a.description && <p className="small muted" style={{ margin: 0 }}>{a.description}</p>}
                  <div className="small faint">{memoryPolicyLabel(a.memoryPolicy)}{a.builtin ? ' · built-in' : ''}</div>
                  <div className="row wrap">
                    <Button size="sm" variant="primary" aria-expanded={running === a.id} onClick={() => setRunning(running === a.id ? null : a.id)}>
                      {running === a.id ? 'Hide' : 'Give a task'}
                    </Button>
                    {a.builtin ? (
                      <Button size="sm" icon={<Copy size={12} />} onClick={() => onDuplicate(a)}>Customize a copy</Button>
                    ) : (
                      <Button size="sm" icon={<Pencil size={12} />} onClick={() => onEdit(a)}>Edit</Button>
                    )}
                  </div>
                  {running === a.id && <RunTask agentId={a.id} agentName={a.name} />}
                </li>
              );
            })}
          </ul>
        </>
      )}

      {templates.length > 0 && (
        <>
          <div className="section-title"><h2>Start from a template</h2></div>
          <div className="ag-templates">
            {templates.map((t) => (
              <button key={t.key} type="button" className="ag-template" onClick={() => onPick(fromTemplate(t))}>
                <AgentAvatar avatar={t.avatar} size={36} />
                <span className="col" style={{ gap: 2, textAlign: 'left', minWidth: 0 }}>
                  <b>{t.name}</b>
                  <span className="small muted">{t.description}</span>
                </span>
              </button>
            ))}
          </div>
        </>
      )}
      <p className="small muted" style={{ marginTop: 24 }}>
        Want budgets, permissions, logs and the full Mission Control?{' '}
        <button type="button" className="link-btn" onClick={() => void setUiMode('advanced')}>Switch to Advanced mode</button>.
      </p>
    </div>
  );
}

export default function AgentsView({ initialTab }: { initialTab?: AgentsTab }) {
  const advanced = useApp((s) => (s.settings?.uiMode ?? 'simple') === 'advanced');
  const tab = useAgents((s) => s.tab);
  const set = useAgents((s) => s.set);
  const load = useAgents((s) => s.load);
  const loaded = useAgents((s) => s.loaded);
  const loadError = useAgents((s) => s.loadError);
  const loading = useAgents((s) => s.loading);
  const [creator, setCreator] = useState<AgentDraft | null>(null);
  const [choosing, setChoosing] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    if (initialTab) set({ tab: initialTab });
  }, [initialTab, set]);

  const edit = (a: CustomAgent) => setCreator(a);
  const duplicate = (a: CustomAgent) => setCreator(duplicateAgent(a));
  const tabs: { key: AgentsTab; label: string }[] = [
    { key: 'agents', label: 'Agents' },
    { key: 'missions', label: 'Mission Control' },
    { key: 'teams', label: 'Teams' },
  ];

  return (
    <div className="ag-root">
      <div className="ws-top" style={{ height: 'auto' }}>
        <Bot size={17} color="var(--accent-2)" />
        <b>Agents</b>
        <div className="tabs" role="tablist" aria-label="Agents sections" style={{ borderBottom: 0, height: 44 }}>
          {tabs.map((t) => (
            <button key={t.key} role="tab" id={`ag-tab-${t.key}`} aria-selected={tab === t.key} aria-controls="ag-panel" className={`tab ${tab === t.key ? 'on' : ''}`} onClick={() => set({ tab: t.key })}>
              {t.label}
            </button>
          ))}
        </div>
        <span className="grow" />
        {loading && <span className="small muted">Loading…</span>}
        <Button size="sm" variant="ghost" icon={<RefreshCw size={13} />} onClick={() => void load()} aria-label="Reload agents">Reload</Button>
        <Badge tone="accent"><Sparkles size={11} /> {advanced ? 'Advanced' : 'Simple'}</Badge>
      </div>
      <div className="ag-body" role="tabpanel" id="ag-panel" aria-labelledby={`ag-tab-${tab}`}>
        {loaded && loadError && tab !== 'agents' && (
          <div style={{ padding: '10px 16px' }}>
            <Callout tone="bad" title="Some data could not be loaded">{loadError}</Callout>
          </div>
        )}
        {tab === 'agents' &&
          (advanced ? (
            <AgentWorkspace advanced onCreate={() => setChoosing(true)} onEdit={edit} onDuplicate={duplicate} />
          ) : loaded && loadError ? (
            <div className="page-inner">
              <Callout tone="bad" title="Agents are not available right now">
                {loadError}
                <div style={{ marginTop: 8 }}><Button size="sm" onClick={() => void load()}>Retry</Button></div>
              </Callout>
            </div>
          ) : (
            <div className="page"><SimpleAgents onCreate={() => setChoosing(true)} onPick={setCreator} onEdit={edit} onDuplicate={duplicate} /></div>
          ))}
        {tab === 'missions' && <MissionControl />}
        {tab === 'teams' && <div className="page"><TeamsManager /></div>}
      </div>
      {choosing && (
        <NewAgentChooser
          onClose={() => setChoosing(false)}
          onPick={(d) => {
            setChoosing(false);
            setCreator(d);
          }}
        />
      )}
      {creator && <AgentCreator key={creator.id ?? 'new'} initial={creator} onClose={() => setCreator(null)} />}
    </div>
  );
}
