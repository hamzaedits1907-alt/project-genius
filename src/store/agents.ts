// State for custom agents, teams and missions. Everything shown comes from backend commands
// and `genius://agent-status` / `genius://agent-log` / `genius://mission-update` events.
import { create } from 'zustand';
import { api, errorMessage } from '../lib/api';
import type { AgentDraft, AgentRuntime, AgentTaskLog, AgentTemplate, CustomAgent, Mission, Project, Team } from '../lib/types';
import { useApp } from './app';

export type AgentsTab = 'agents' | 'missions' | 'teams';
const MAX_LOG = 300;

interface AgentsState {
  loaded: boolean;
  loading: boolean;
  /** Set when the backend could not be reached or a command failed; shown inline with a retry. */
  loadError: string | null;
  agents: CustomAgent[];
  templates: AgentTemplate[];
  runtimes: Record<string, AgentRuntime>;
  teams: Team[];
  missions: Mission[];
  projects: Project[];
  logs: Record<string, AgentTaskLog[]>;
  tab: AgentsTab;
  selectedAgentId: string | null;
  selectedMissionId: string | null;
  selectedTaskId: string | null;

  set: (p: Partial<AgentsState>) => void;
  load: () => Promise<void>;
  loadLogs: (agentId: string) => Promise<void>;
  // events
  onAgentStatus: (r: AgentRuntime) => void;
  onMission: (m: Mission) => void;
  onAgentLog: (l: AgentTaskLog) => void;
  // actions (each toasts an honest error and resolves to null on failure)
  saveAgent: (a: AgentDraft | CustomAgent) => Promise<CustomAgent | null>;
  deleteAgent: (id: string) => Promise<boolean>;
  saveTeam: (t: Team | Omit<Team, 'id'>) => Promise<Team | null>;
  deleteTeam: (id: string) => Promise<boolean>;
  runTask: (agentId: string, projectId: string, task: string) => Promise<Mission | null>;
  createMission: (projectId: string, goal: string, teamId: string | null) => Promise<Mission | null>;
  missionAction: (kind: 'start' | 'pause' | 'resume' | 'cancel', missionId: string) => Promise<Mission | null>;
  taskAction: (kind: 'cancel' | 'retry' | 'approve' | 'reject' | 'reprioritize', taskId: string, priority?: number) => Promise<Mission | null>;
}

const fail = (what: string, e: unknown) => useApp.getState().toast('bad', `${what}: ${errorMessage(e)}`);

export function upsertMission(list: Mission[], m: Mission): Mission[] {
  const i = list.findIndex((x) => x.id === m.id);
  if (i < 0) return [m, ...list];
  // An older snapshot (a slow command reply arriving after a newer event) must not overwrite newer state.
  if (list[i].updatedAt > m.updatedAt) return list;
  const next = list.slice();
  next[i] = m;
  return next;
}

export const useAgents = create<AgentsState>((set) => {
  const putMission = (m: Mission) => set((s) => ({ missions: upsertMission(s.missions, m) }));
  return {
    loaded: false,
    loading: false,
    loadError: null,
    agents: [],
    templates: [],
    runtimes: {},
    teams: [],
    missions: [],
    projects: [],
    logs: {},
    tab: 'agents',
    selectedAgentId: null,
    selectedMissionId: null,
    selectedTaskId: null,

    set: (p) => set(p),
    load: async () => {
      set({ loading: true });
      const [agents, templates, runtimes, teams, missions, projects] = await Promise.allSettled([
        api.listAgents(),
        api.agentTemplates(),
        api.agentRuntimes(),
        api.listTeams(),
        api.listMissions(null),
        api.listProjects(),
      ]);
      const ok = <T,>(r: PromiseSettledResult<T>): T | undefined => (r.status === 'fulfilled' ? r.value : undefined);
      const firstError = [agents, templates, runtimes, teams, missions, projects].find((r) => r.status === 'rejected');
      const list = ok(agents);
      set((s) => ({
        loading: false,
        loaded: true,
        loadError: firstError && firstError.status === 'rejected' ? errorMessage(firstError.reason) : null,
        agents: list ?? s.agents,
        templates: ok(templates) ?? s.templates,
        runtimes: ok(runtimes) ? Object.fromEntries(ok(runtimes)!.map((r) => [r.agentId, r])) : s.runtimes,
        teams: ok(teams) ?? s.teams,
        // keep anything newer that arrived by event while the request was in flight
        missions: ok(missions) ? ok(missions)!.reduce(upsertMission, s.missions) : s.missions,
        projects: ok(projects) ?? s.projects,
        selectedAgentId: s.selectedAgentId && list?.some((a) => a.id === s.selectedAgentId) ? s.selectedAgentId : (list?.[0]?.id ?? null),
      }));
    },
    loadLogs: async (agentId) => {
      try {
        const rows = await api.agentLogs(agentId, 100);
        set((s) => {
          const live = s.logs[agentId] ?? [];
          const seen = new Set(rows.map((r) => r.id));
          return { logs: { ...s.logs, [agentId]: [...rows, ...live.filter((l) => !seen.has(l.id))].slice(-MAX_LOG) } };
        });
      } catch (e) {
        fail('Could not load the activity log', e);
      }
    },

    onAgentStatus: (r) => set((s) => ({ runtimes: { ...s.runtimes, [r.agentId]: r } })),
    onMission: putMission,
    onAgentLog: (l) =>
      set((s) => {
        const cur = s.logs[l.agentId] ?? [];
        if (cur.some((x) => x.id === l.id)) return s;
        return { logs: { ...s.logs, [l.agentId]: [...cur, l].slice(-MAX_LOG) } };
      }),

    saveAgent: async (a) => {
      try {
        const saved = await api.saveAgent(a);
        set((s) => ({ agents: s.agents.some((x) => x.id === saved.id) ? s.agents.map((x) => (x.id === saved.id ? saved : x)) : [...s.agents, saved], selectedAgentId: saved.id }));
        return saved;
      } catch (e) {
        fail('Could not save the agent', e);
        return null;
      }
    },
    deleteAgent: async (id) => {
      try {
        await api.deleteAgent(id);
        set((s) => {
          const agents = s.agents.filter((a) => a.id !== id);
          return { agents, selectedAgentId: s.selectedAgentId === id ? (agents[0]?.id ?? null) : s.selectedAgentId };
        });
        return true;
      } catch (e) {
        fail('Could not delete the agent', e);
        return false;
      }
    },
    saveTeam: async (t) => {
      try {
        const saved = await api.saveTeam(t);
        set((s) => ({ teams: s.teams.some((x) => x.id === saved.id) ? s.teams.map((x) => (x.id === saved.id ? saved : x)) : [...s.teams, saved] }));
        return saved;
      } catch (e) {
        fail('Could not save the team', e);
        return null;
      }
    },
    deleteTeam: async (id) => {
      try {
        await api.deleteTeam(id);
        set((s) => ({ teams: s.teams.filter((t) => t.id !== id) }));
        return true;
      } catch (e) {
        fail('Could not delete the team', e);
        return false;
      }
    },
    runTask: async (agentId, projectId, task) => {
      try {
        const m = await api.runAgentTask(agentId, projectId, task);
        putMission(m);
        return m;
      } catch (e) {
        fail('Could not start the task', e);
        return null;
      }
    },
    createMission: async (projectId, goal, teamId) => {
      try {
        const m = await api.createMission(projectId, goal, teamId);
        putMission(m);
        set({ selectedMissionId: m.id, selectedTaskId: null });
        return m;
      } catch (e) {
        fail('Could not create the mission', e);
        return null;
      }
    },
    missionAction: async (kind, missionId) => {
      const fn = { start: api.startMission, pause: api.pauseMission, resume: api.resumeMission, cancel: api.cancelMission }[kind];
      try {
        const m = await fn(missionId);
        putMission(m);
        return m;
      } catch (e) {
        fail(`Could not ${kind} the mission`, e);
        return null;
      }
    },
    taskAction: async (kind, taskId, priority) => {
      try {
        const m =
          kind === 'cancel'
            ? await api.cancelMissionTask(taskId)
            : kind === 'retry'
              ? await api.retryMissionTask(taskId)
              : kind === 'reprioritize'
                ? await api.reprioritizeTask(taskId, priority ?? 0)
                : await api.approveMissionTask(taskId, kind === 'approve');
        putMission(m);
        return m;
      } catch (e) {
        fail(`Could not ${kind === 'reprioritize' ? 'change the priority of' : kind} the task`, e);
        return null;
      }
    },
  };
});

/** Runtime of an agent (idle when the backend has not reported one). */
export function runtimeOf(runtimes: Record<string, AgentRuntime>, agentId: string): AgentRuntime {
  return (
    runtimes[agentId] ?? {
      agentId,
      status: 'idle',
      currentTaskId: null,
      lastActivityAt: null,
      usage: { tokens: 0, calls: 0, tasksDone: 0, tasksFailed: 0 },
    }
  );
}
