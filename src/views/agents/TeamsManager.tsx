// Teams: a coordinator plus members. Used when creating missions.
import { useState } from 'react';
import { Pencil, Plus, Trash2, Users } from 'lucide-react';
import type { Team } from '../../lib/types';
import { useAgents } from '../../store/agents';
import { Button, Empty, Field, Modal } from '../../components/ui';
import { AgentAvatar } from './parts';

type TeamDraft = Omit<Team, 'id'> & { id?: string };

export function validateTeam(t: TeamDraft): Record<string, string> {
  const e: Record<string, string> = {};
  if (!t.name.trim()) e.name = 'Give the team a name.';
  else if (t.name.trim().length > 60) e.name = 'Keep the name under 60 characters.';
  if (!t.coordinatorAgentId) e.coordinator = 'Choose the coordinator who plans the work.';
  if (t.agentIds.filter((id) => id !== t.coordinatorAgentId).length === 0) e.members = 'Add at least one member besides the coordinator.';
  return e;
}

function TeamEditor({ initial, onClose }: { initial: TeamDraft; onClose: () => void }) {
  const agents = useAgents((s) => s.agents);
  const saveTeam = useAgents((s) => s.saveTeam);
  const [team, setTeam] = useState<TeamDraft>(initial);
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const errors = validateTeam(team);
  const shown = (k: string) => (submitted ? errors[k] : undefined);

  const setCoordinator = (id: string) => setTeam((t) => ({ ...t, coordinatorAgentId: id, agentIds: id && !t.agentIds.includes(id) ? [...t.agentIds, id] : t.agentIds }));
  const toggle = (id: string, on: boolean) => setTeam((t) => ({ ...t, agentIds: on ? [...t.agentIds, id] : t.agentIds.filter((x) => x !== id) }));

  const save = async () => {
    setSubmitted(true);
    if (Object.keys(errors).length) return;
    setBusy(true);
    // The coordinator is always part of agentIds (it can also be given tasks).
    const ids = Array.from(new Set([team.coordinatorAgentId, ...team.agentIds]));
    const saved = await saveTeam({ ...team, name: team.name.trim(), description: team.description.trim(), agentIds: ids });
    setBusy(false);
    if (saved) onClose();
  };

  return (
    <Modal
      title={initial.id ? 'Edit team' : 'Create a team'}
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} onClick={() => void save()}>{initial.id ? 'Save team' : 'Create team'}</Button>
        </>
      }
    >
      <div className="col" style={{ gap: 14 }}>
        <Field label="Team name" htmlFor="tm-name">
          <input id="tm-name" className="input" value={team.name} onChange={(e) => setTeam({ ...team, name: e.target.value })} aria-invalid={shown('name') ? true : undefined} autoFocus />
          {shown('name') && <div className="field-error" role="alert">{shown('name')}</div>}
        </Field>
        <Field label="Description" htmlFor="tm-desc">
          <input id="tm-desc" className="input" value={team.description} onChange={(e) => setTeam({ ...team, description: e.target.value })} />
        </Field>
        <Field label="Coordinator" htmlFor="tm-coord" hint="Reads the goal, plans the task graph and assigns tasks to members.">
          <select id="tm-coord" className="select" value={team.coordinatorAgentId} onChange={(e) => setCoordinator(e.target.value)} aria-invalid={shown('coordinator') ? true : undefined}>
            <option value="">Choose an agent…</option>
            {agents.map((a) => (
              <option key={a.id} value={a.id}>{a.name} — {a.role}</option>
            ))}
          </select>
          {shown('coordinator') && <div className="field-error" role="alert">{shown('coordinator')}</div>}
        </Field>
        <fieldset className="tool-group">
          <legend>Members</legend>
          {agents.map((a) => (
            <label key={a.id} className="tool-row">
              <input type="checkbox" checked={team.agentIds.includes(a.id)} disabled={a.id === team.coordinatorAgentId} onChange={(e) => toggle(a.id, e.target.checked)} />
              <span><b>{a.name}</b><span className="muted"> — {a.role}{a.id === team.coordinatorAgentId ? ' (coordinator)' : ''}</span></span>
            </label>
          ))}
          {shown('members') && <div className="field-error" role="alert">{shown('members')}</div>}
        </fieldset>
      </div>
    </Modal>
  );
}

export default function TeamsManager() {
  const teams = useAgents((s) => s.teams);
  const agents = useAgents((s) => s.agents);
  const deleteTeam = useAgents((s) => s.deleteTeam);
  const [editing, setEditing] = useState<TeamDraft | null>(null);
  const name = (id: string) => agents.find((a) => a.id === id);

  return (
    <div className="page-inner" style={{ maxWidth: 900 }}>
      <div className="row">
        <div className="grow">
          <h2 style={{ margin: 0 }}>Teams</h2>
          <p className="muted" style={{ margin: '4px 0 0' }}>A team is a coordinator plus the agents it can assign work to.</p>
        </div>
        <Button variant="primary" icon={<Plus size={14} />} disabled={agents.length < 2} onClick={() => setEditing({ name: '', description: '', coordinatorAgentId: '', agentIds: [], createdAt: new Date().toISOString() })}>
          New team
        </Button>
      </div>
      {agents.length < 2 && <p className="small muted">You need at least two agents to form a team.</p>}
      {teams.length === 0 ? (
        <Empty icon={<Users size={20} />} title="No teams yet">Create a team to run missions with several agents.</Empty>
      ) : (
        <ul className="ag-teams" aria-label="Teams">
          {teams.map((t) => (
            <li key={t.id} className="card pad col" style={{ gap: 8 }}>
              <div className="row">
                <b className="grow">{t.name}</b>
                <Button size="sm" variant="ghost" icon={<Pencil size={13} />} onClick={() => setEditing(t)} aria-label={`Edit ${t.name}`}>Edit</Button>
                <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} onClick={() => window.confirm(`Delete the team “${t.name}”? Missions that used it are kept.`) && void deleteTeam(t.id)} aria-label={`Delete ${t.name}`}>Delete</Button>
              </div>
              {t.description && <div className="small muted">{t.description}</div>}
              <div className="row wrap">
                {t.agentIds.map((id) => {
                  const a = name(id);
                  return (
                    <span key={id} className="chip">
                      {a && <AgentAvatar avatar={a.avatar} size={18} />}
                      {a?.name ?? 'Unknown'}{id === t.coordinatorAgentId ? ' ★' : ''}
                    </span>
                  );
                })}
              </div>
              <div className="tiny faint">★ coordinator</div>
            </li>
          ))}
        </ul>
      )}
      {editing && <TeamEditor initial={editing} onClose={() => setEditing(null)} />}
    </div>
  );
}
