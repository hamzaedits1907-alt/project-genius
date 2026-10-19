// Agent Mode in the browser: a goal is planned into steps by the model, and each step runs the same tool
// loop as Chat/Research (web search, page reading, exact calculation). A browser has no project folder,
// terminal or build tools, so browser missions can never change files or be "verified" by a build or
// test; they finish as "completed, not verified" and say so. Statuses change only on real transitions,
// and the summary is built from recorded results, never written by a model.

import type { AgentTaskLog, CustomAgent, Mission, MissionTask, Project } from '../lib/types';

type AgentLogKind = AgentTaskLog['kind'];
import { runTask } from './engine/chat';
import { lenientJson } from './engine/toolsyntax';
import { ProviderError, complete } from './engine/providers';
import type { WebRuntime } from './runtime';
import { DESKTOP_ONLY, WORKSPACE_ID } from './runtime';

const MISSION_CMDS = new Set(['list_agents', 'get_agent', 'agent_templates', 'agent_runtimes', 'agent_logs', 'list_teams', 'list_missions', 'get_mission', 'create_project', 'create_mission', 'start_mission', 'pause_mission', 'resume_mission', 'cancel_mission', 'cancel_mission_task', 'retry_mission_task', 'approve_mission_task', 'reprioritize_task', 'run_agent_task', 'save_agent', 'delete_agent', 'save_team', 'delete_team']);
const VIRTUAL_PROJECT_ID = 'browser-workspace';
const MAX_STEPS = 5;

const BUILTIN: CustomAgent[] = [
  { id: 'web-researcher', name: 'Web researcher', avatar: '🔍', role: 'Researcher', description: 'Searches the web, reads pages and reports what they say, with the addresses it read.', instructions: 'Use web_search and fetch_url; report only what the pages say.', specialization: ['research'], knowledgeSources: [], modelPolicy: 'auto', model: null, tools: ['web_search', 'fetch_url', 'calculate'], scope: { projects: 'any', paths: [] }, network: true, memoryPolicy: 'none', budget: { maxSteps: 30, maxMinutes: 20, maxTokens: 200_000, maxModelCalls: 40 }, retryLimit: 1, approval: 'ask', acceptanceCriteria: '', outputRequirements: '', builtin: true, createdAt: '2026-10-19T00:00:00Z', updatedAt: '2026-10-19T00:00:00Z' },
  { id: 'analyst', name: 'Analyst', avatar: '📊', role: 'Analyst', description: 'Works through numbers and attached tables with exact tools, and explains the result.', instructions: 'Use calculate and data_summary; never do arithmetic in your head.', specialization: ['analysis'], knowledgeSources: [], modelPolicy: 'auto', model: null, tools: ['calculate', 'data_summary', 'web_search', 'fetch_url'], scope: { projects: 'any', paths: [] }, network: true, memoryPolicy: 'none', budget: { maxSteps: 30, maxMinutes: 20, maxTokens: 200_000, maxModelCalls: 40 }, retryLimit: 1, approval: 'ask', acceptanceCriteria: '', outputRequirements: '', builtin: true, createdAt: '2026-10-19T00:00:00Z', updatedAt: '2026-10-19T00:00:00Z' },
];

const COORDINATOR = `You are the coordinator of Genius Web Agent Mode. Split the user's goal into 1 to ${MAX_STEPS} steps that can be done in a web browser with ONLY these tools: web search, reading public web pages, exact calculation, and reading files the user attached. There is no project folder, terminal, build or file editing here.
Reply with JSON only, no prose:
{"summary": "one sentence plan", "tasks": [{"id": "t1", "title": "short title", "description": "what to do and what to report", "dependsOn": []}]}
Use dependsOn only when a step needs an earlier step's result. If the goal REQUIRES editing files, running commands, builds or tests, reply {"summary": "", "unsupported": "why this needs Genius Desktop", "tasks": []}.`;

const iso = () => new Date().toISOString();
const uid = () => crypto.randomUUID();

interface Live {
  abort: AbortController;
  convId: string | null;
  taskAborts: Map<string, AbortController>;
}

export class MissionRuntime {
  private missions = new Map<string, Mission>();
  private loaded = false;
  private live = new Map<string, Live>();
  private logs = new Map<string, import('../lib/types').AgentTaskLog[]>();

  constructor(private rt: WebRuntime) {}

  handles(cmd: string) {
    return MISSION_CMDS.has(cmd);
  }

  private async ensure() {
    if (this.loaded) return;
    this.loaded = true;
    for (const m of await this.rt.opts.store.listMissions().catch(() => [])) {
      // a mission that was running when the page closed did not finish: say so instead of showing it as running
      if (['planning', 'running', 'integrating', 'verifying'].includes(m.status)) {
        m.status = 'failed';
        m.summary = 'Interrupted: the page was closed or reloaded while this mission was running. Nothing was lost, but the remaining steps did not run.';
        m.tasks = m.tasks.map((t) => (['running', 'reviewing', 'ready', 'pending'].includes(t.status) ? { ...t, status: 'cancelled' as const } : t));
      }
      this.missions.set(m.id, m);
    }
  }

  private put(m: Mission) {
    m.updatedAt = iso();
    this.missions.set(m.id, m);
    void this.rt.opts.store.saveMission(m).catch(() => undefined);
    this.rt.emit('genius://mission-update', structuredClone(m));
  }

  private log(m: Mission, t: MissionTask | null, kind: AgentLogKind, text: string, data: unknown = null) {
    const agentId = t?.assigneeAgentId ?? 'web-researcher';
    const entry = { id: uid(), agentId, missionId: m.id, taskId: t?.id ?? null, projectId: VIRTUAL_PROJECT_ID, kind, text, data, at: iso() };
    this.logs.set(agentId, [...(this.logs.get(agentId) ?? []), entry].slice(-300));
    this.rt.emit('genius://agent-log', entry);
  }

  private status(agentId: string, status: 'idle' | 'working' | 'planning' | 'completed' | 'failed', taskId: string | null) {
    this.rt.emit('genius://agent-status', { agentId, status, currentTaskId: taskId, lastActivityAt: iso(), usage: { tokens: 0, calls: 0, tasksDone: 0, tasksFailed: 0 } });
  }

  async invoke(cmd: string, a: Record<string, any>): Promise<unknown> { // eslint-disable-line @typescript-eslint/no-explicit-any
    await this.ensure();
    switch (cmd) {
      case 'list_agents':
        return BUILTIN;
      case 'get_agent':
        return BUILTIN.find((x) => x.id === a.agentId) ?? Promise.reject({ kind: 'not_found', message: 'Unknown agent.' });
      case 'agent_templates':
      case 'list_teams':
        return [];
      case 'agent_runtimes':
        return BUILTIN.map((x) => ({ agentId: x.id, status: 'idle', currentTaskId: null, lastActivityAt: null, usage: { tokens: 0, calls: 0, tasksDone: 0, tasksFailed: 0 } }));
      case 'agent_logs':
        return (this.logs.get(a.agentId) ?? []).slice(-(a.limit ?? 100));
      case 'save_agent':
      case 'delete_agent':
      case 'save_team':
      case 'delete_team':
        throw DESKTOP_ONLY('Creating custom agents and teams');
      case 'create_project': {
        const p: Project = { id: VIRTUAL_PROJECT_ID, name: a.name, description: 'Genius Web has no project folder: browser missions use web tools only.', path: '', kind: 'browser', spec: null, notes: '', status: {}, createdAt: iso(), updatedAt: iso(), lastOpenedAt: null };
        return p;
      }
      case 'list_missions':
        return [...this.missions.values()].sort((x, y) => y.updatedAt.localeCompare(x.updatedAt));
      case 'get_mission':
        return this.get(a.missionId);
      case 'create_mission':
        return this.create(String(a.goal), a.teamId ?? null);
      case 'run_agent_task':
        return this.create(String(a.task), null, String(a.agentId));
      case 'start_mission':
        return this.start(a.missionId);
      case 'pause_mission':
      case 'resume_mission':
        throw { kind: 'unsupported', message: 'Pausing is not available in the browser. You can stop the mission and start it again.' };
      case 'cancel_mission':
        return this.cancel(a.missionId);
      case 'cancel_mission_task':
        return this.cancelStep(a.taskId);
      case 'retry_mission_task':
        return this.retry(a.taskId);
      case 'approve_mission_task':
      case 'reprioritize_task':
        throw { kind: 'unsupported', message: 'Browser missions change no files, so there is nothing to approve.' };
      default:
        throw DESKTOP_ONLY(cmd);
    }
  }

  private get(id: string): Mission {
    const m = this.missions.get(id);
    if (!m) throw { kind: 'not_found', message: 'Unknown mission.' };
    return m;
  }

  private async create(goal: string, teamId: string | null, agentId?: string): Promise<Mission> {
    if (goal.trim().length < 3) throw { kind: 'validation', message: 'Describe the goal in at least a sentence.' };
    const m: Mission = { id: uid(), projectId: VIRTUAL_PROJECT_ID, title: goal.trim().split('\n')[0].slice(0, 80), goal: goal.trim(), teamId, status: 'planning', tasks: [], summary: null, createdAt: iso(), updatedAt: iso() };
    this.put(m);
    void this.plan(m, agentId);
    return structuredClone(m);
  }

  private async plan(m: Mission, forcedAgent?: string) {
    const abort = new AbortController();
    this.live.set(m.id, { abort, convId: null, taskAborts: new Map() });
    this.status('web-researcher', 'planning', null);
    try {
      this.rt.providerConfig(); // clear error without a key
      const models = await this.rt.resolveModels('agent', 'max');
      if (!models.length) throw new Error('No model is available. Add your provider key and choose a model in Settings.');
      let plan: { summary?: string; unsupported?: string; tasks?: { id?: string; title?: string; description?: string; dependsOn?: string[] }[] } = {};
      if (forcedAgent) plan = { tasks: [{ id: 't1', title: m.title, description: m.goal, dependsOn: [] }] };
      else {
        const fn = this.rt.opts.completeFn ?? complete;
        const res = await fn(this.rt.providerConfig(), [{ role: 'system', content: COORDINATOR }, { role: 'user', content: m.goal }], { model: models[0], signal: abort.signal, temperature: 0, maxTokens: 1500, retries: this.rt.cfg.limits.maxProviderRetries });
        try {
          plan = lenientJson(res.content) as typeof plan;
        } catch {
          plan = { tasks: [{ id: 't1', title: m.title, description: m.goal, dependsOn: [] }], summary: 'The plan could not be read, so the goal runs as a single step.' };
        }
      }
      if (plan.unsupported) {
        m.status = 'failed';
        m.summary = `This goal needs Genius Desktop: ${plan.unsupported}`;
        this.put(m);
        return;
      }
      const raw = (plan.tasks ?? []).slice(0, MAX_STEPS);
      const ids = new Set(raw.map((t, i) => t.id ?? `t${i + 1}`));
      const tasks: MissionTask[] = (raw.length ? raw : [{ id: 't1', title: m.title, description: m.goal, dependsOn: [] }]).map((t, i) => ({
        id: uid(), missionId: m.id, title: (t.title ?? `Step ${i + 1}`).slice(0, 120), description: t.description ?? '', assigneeAgentId: forcedAgent ?? 'web-researcher',
        dependsOn: [], priority: 50, status: 'ready', result: null, blocker: null, changesetId: null, workspace: null, verification: { build: 'unknown', tests: 'unknown', notes: '' },
        startedAt: null, endedAt: null, usage: { tokens: 0, calls: 0 }, changedPaths: [], attempts: 0, _plan: { key: t.id ?? `t${i + 1}`, deps: (t.dependsOn ?? []).filter((d) => ids.has(d) && d !== (t.id ?? `t${i + 1}`)) },
      } as MissionTask));
      const keyToId = new Map(tasks.map((t) => [(t as unknown as { _plan: { key: string } })._plan.key, t.id]));
      for (const t of tasks) {
        const p = (t as unknown as { _plan: { deps: string[] } })._plan;
        t.dependsOn = p.deps.map((d) => keyToId.get(d)!).filter(Boolean);
        t.status = t.dependsOn.length ? 'pending' : 'ready';
        delete (t as unknown as { _plan?: unknown })._plan;
      }
      m.tasks = tasks;
      m.status = 'ready';
      (m as Mission & { meta?: unknown }).meta = { plan: plan.summary ?? '' };
      this.put(m);
    } catch (e) {
      m.status = 'failed';
      m.summary = abort.signal.aborted ? 'Cancelled while planning.' : `Planning failed: ${e instanceof Error ? e.message : (e as { message?: string })?.message ?? String(e)}`;
      if (abort.signal.aborted) m.status = 'cancelled';
      this.put(m);
    } finally {
      this.status('web-researcher', 'idle', null);
    }
  }

  private start(id: string): Mission {
    const m = this.get(id);
    if (m.status !== 'ready') throw { kind: 'validation', message: `The mission is ${m.status}; only a planned mission can be started.` };
    m.status = 'running';
    this.put(m);
    void this.run(m);
    return structuredClone(m);
  }

  private async run(m: Mission) {
    const live = this.live.get(m.id) ?? { abort: new AbortController(), convId: null, taskAborts: new Map() };
    if (live.abort.signal.aborted) live.abort = new AbortController();
    this.live.set(m.id, live);
    const store = this.rt.opts.store;
    try {
      const conv = await store.createConversation('agent', m.title);
      live.convId = conv.id;
      void store.recordUsage('agent');
      for (;;) {
        if (live.abort.signal.aborted) break;
        const next = m.tasks.find((t) => t.status === 'ready' && t.dependsOn.every((d) => m.tasks.find((x) => x.id === d)?.status === 'done'));
        if (!next) break;
        const ta = new AbortController();
        live.taskAborts.set(next.id, ta);
        live.abort.signal.addEventListener('abort', () => ta.abort(), { once: true });
        next.status = 'running';
        next.startedAt = iso();
        next.attempts = (next.attempts ?? 0) + 1;
        this.put(m);
        this.status(next.assigneeAgentId ?? 'web-researcher', 'working', next.id);
        this.log(m, next, 'status', `Started: ${next.title}`);
        const prior = m.tasks.filter((t) => t.status === 'done' && t.result).map((t) => `- ${t.title}: ${t.result}`).join('\n');
        const idx = m.tasks.indexOf(next) + 1;
        const message = `MISSION GOAL: ${m.goal}\n\nYOUR STEP (${idx} of ${m.tasks.length}): ${next.title}\n${next.description}\n${prior ? `\nRESULTS OF EARLIER STEPS:\n${prior}\n` : ''}\nDo this step now with your tools. Finish with <final> containing the result of this step only, and name the web addresses you actually read. Report only what tools actually returned; if you could not do something, say so.`;
        try {
          const env = this.rt.engineEnvFor();
          const origEmit = env.emit;
          env.emit = (c, p) => {
            if (c === 'genius://agent-step') {
              const st = p as import('../lib/types').AgentStep;
              this.log(m, next, 'tool', `${st.ok ? '' : 'Failed: '}${st.summary}`, { tool: st.tool, args: st.args });
            }
            if (c !== 'genius://stream') origEmit(c, p);
          };
          const r = await runTask(env, { taskId: `${m.id}:${next.id}:${next.attempts}`, conv, role: 'chat', message, attachments: [], effort: 'balanced' }, ta.signal);
          const report = r.meta.report as { usage?: { totalTokens: number; calls: number }; lifecycle?: string; warnings?: string[] };
          next.usage = { tokens: report.usage?.totalTokens ?? 0, calls: report.usage?.calls ?? 0 };
          next.result = r.answer;
          next.verification = { build: 'unknown', tests: 'unknown', notes: 'Not verified: browser missions cannot run builds or tests, and this step changed no files.' };
          next.acceptance = { checked: false, met: false, verdict: 'not checked' };
          if (report.lifecycle === 'blocked') {
            next.status = 'failed';
            next.blocker = report.warnings?.[0] ?? 'A Genius limit stopped this step.';
            this.log(m, next, 'error', next.blocker);
          } else {
            next.status = 'done';
            this.log(m, next, 'result', r.answer.slice(0, 400));
          }
        } catch (e) {
          next.status = live.abort.signal.aborted || ta.signal.aborted ? 'cancelled' : 'failed';
          if (next.status === 'failed') {
            next.blocker = e instanceof Error ? e.message : (e as { message?: string })?.message ?? String(e);
            this.log(m, next, 'error', next.blocker);
          }
          if (e instanceof ProviderError && e.kind === 'auth') live.abort.abort();
        }
        next.endedAt = iso();
        live.taskAborts.delete(next.id);
        this.status(next.assigneeAgentId ?? 'web-researcher', next.status === 'done' ? 'completed' : 'failed', null);
        // steps that depended on a step that did not finish cannot run
        for (const t of m.tasks) if ((t.status === 'pending' || t.status === 'ready') && t.dependsOn.some((d) => ['failed', 'cancelled'].includes(m.tasks.find((x) => x.id === d)?.status ?? ''))) {
          t.status = 'blocked';
          t.blocker = 'An earlier step it depends on did not finish.';
        }
        for (const t of m.tasks) if (t.status === 'pending' && t.dependsOn.every((d) => m.tasks.find((x) => x.id === d)?.status === 'done')) t.status = 'ready';
        this.put(m);
      }
    } catch (e) {
      m.summary = `The mission stopped: ${e instanceof Error ? e.message : (e as { message?: string })?.message ?? String(e)}`;
    }
    const done = m.tasks.filter((t) => t.status === 'done');
    const cancelled = live.abort.signal.aborted;
    m.status = cancelled ? 'cancelled' : done.length === m.tasks.length && m.tasks.length > 0 ? 'completed' : 'failed';
    for (const t of m.tasks) if (['ready', 'pending', 'running'].includes(t.status)) t.status = 'cancelled';
    m.summary = [
      m.summary,
      `Mission “${m.title}”: ${done.length} of ${m.tasks.length} step(s) done.`,
      ...m.tasks.map((t, i) => `${i + 1}. ${t.title} — ${t.status === 'done' ? 'done' : t.status}${t.status === 'done' ? `. ${(t.result ?? '').replace(/\s+/g, ' ').slice(0, 220)}` : t.blocker ? `. ${t.blocker}` : ''}`),
      'Verification: not verified. Browser missions cannot run builds or tests and change no files; check the results and the web addresses they name yourself.',
    ].filter(Boolean).join('\n');
    this.put(m);
    this.live.delete(m.id);
  }

  private cancel(id: string): Mission {
    const m = this.get(id);
    this.live.get(id)?.abort.abort();
    if (['planning', 'ready'].includes(m.status)) {
      m.status = 'cancelled';
      m.summary = 'Cancelled before it started.';
      for (const t of m.tasks) t.status = 'cancelled';
      this.put(m);
    }
    return structuredClone(this.get(id));
  }

  cancelTask(taskKey: string) {
    for (const l of this.live.values()) l.taskAborts.get(taskKey)?.abort();
  }
  private cancelStep(taskId: string): Mission {
    for (const m of this.missions.values()) {
      const t = m.tasks.find((x) => x.id === taskId);
      if (t) {
        this.live.get(m.id)?.taskAborts.get(taskId)?.abort();
        if (['pending', 'ready'].includes(t.status)) {
          t.status = 'cancelled';
          this.put(m);
        }
        return structuredClone(m);
      }
    }
    throw { kind: 'not_found', message: 'Unknown step.' };
  }
  private retry(taskId: string): Mission {
    for (const m of this.missions.values()) {
      const t = m.tasks.find((x) => x.id === taskId);
      if (t && ['failed', 'cancelled', 'blocked'].includes(t.status) && ['failed', 'cancelled', 'completed'].includes(m.status)) {
        t.status = 'ready';
        t.blocker = null;
        t.result = null;
        for (const o of m.tasks) if (o.status === 'blocked' || (o.status === 'cancelled' && o.dependsOn.includes(t.id))) o.status = o.dependsOn.every((d) => d === t.id || m.tasks.find((x) => x.id === d)?.status === 'done') ? 'pending' : o.status;
        m.status = 'running';
        m.summary = null;
        this.put(m);
        void this.run(m);
        return structuredClone(m);
      }
    }
    throw { kind: 'validation', message: 'That step cannot be retried now.' };
  }
}

export { VIRTUAL_PROJECT_ID, WORKSPACE_ID };
