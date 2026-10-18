// Custom agents, Agent Workspace and Mission Control against the mocked Tauri IPC.
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { emit } from '@tauri-apps/api/event';
import { builtinAgent, calls, clearMocks, installIpc, missionFixture, settings } from './ipc';
import App from '../App';
import { useApp } from '../store/app';
import { useAgents } from '../store/agents';
import {
  BUDGET_DEFAULTS,
  DEFAULT_TOOLS,
  EDIT_TOOLS,
  blankAgent,
  isAnimatedStatus,
  isSafeRelPath,
  layoutGraph,
  moodForStatus,
  normalizeAgent,
  validateAgent,
} from '../lib/agents';
import type { AgentRuntime, Mission, MissionTask } from '../lib/types';

vi.mock('react-resizable-panels', async () => {
  const { forwardRef, createElement } = await import('react');
  const Panel = forwardRef<unknown, { children?: React.ReactNode }>(function Panel({ children }, _ref) {
    return createElement('div', { 'data-panel': '' }, children);
  });
  const PanelGroup = ({ children }: { children?: React.ReactNode }) => createElement('div', null, children);
  return { Panel, PanelGroup, PanelResizeHandle: () => null };
});

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const click = async (el: Element) => {
  await act(async () => void fireEvent.click(el));
  await flush();
};
const type = async (el: Element, value: string) => {
  await act(async () => void fireEvent.change(el, { target: { value } }));
};
const send = async (channel: string, payload: unknown) => {
  await act(async () => {
    await emit(channel, payload);
  });
  await flush();
};
const lastCall = (cmd: string) => [...calls].reverse().find((c) => c.cmd === cmd);

const runtime = (over: Partial<AgentRuntime> = {}): AgentRuntime => ({
  agentId: 'agent-designer', status: 'idle', currentTaskId: null, lastActivityAt: null, usage: { tokens: 0, calls: 0, tasksDone: 0, tasksFailed: 0 }, ...over,
});
const mission = (over: Partial<Mission> = {}, tasks?: MissionTask[]): Mission => ({ ...missionFixture, ...over, tasks: tasks ?? missionFixture.tasks });

function resetStore() {
  useAgents.setState({ loaded: false, loading: false, loadError: null, agents: [], templates: [], runtimes: {}, teams: [], missions: [], projects: [], logs: {}, tab: 'agents', selectedAgentId: null, selectedMissionId: null, selectedTaskId: null });
}

async function openAgents(tab: 'agents' | 'missions' | 'teams' = 'agents') {
  const r = render(<StrictMode><App /></StrictMode>);
  await flush();
  await act(async () => useApp.getState().navigate({ name: 'agents', tab }));
  await flush(60);
  return r;
}

describe('agent logic', () => {
  it('validates the agent form with clear messages', () => {
    const e = validateAgent(blankAgent());
    expect(e.name).toMatch(/name/i);
    expect(e.role).toBeTruthy();
    expect(e.instructions).toMatch(/at least 20/);
    expect(validateAgent({ ...blankAgent(), name: 'A', role: 'R', instructions: 'x'.repeat(30) })).toEqual({});

    const base = { ...blankAgent(), name: 'A', role: 'R', instructions: 'x'.repeat(30) };
    expect(validateAgent({ ...base, modelPolicy: 'manual', model: null }).model).toBeTruthy();
    expect(validateAgent({ ...base, tools: [] }).tools).toBeTruthy();
    expect(validateAgent({ ...base, approval: 'plan_only', tools: ['read_file', EDIT_TOOLS[0]] }).tools).toMatch(/plan-only/);
    expect(validateAgent({ ...base, tools: ['web_search'], network: false }).tools).toMatch(/Network/);
    expect(validateAgent({ ...base, tools: ['web_search'], network: true })).toEqual({});
    expect(validateAgent({ ...base, budget: { ...BUDGET_DEFAULTS, maxSteps: 101 } })['budget.maxSteps']).toMatch(/limited to 100/);
    expect(validateAgent({ ...base, budget: { ...BUDGET_DEFAULTS, maxTokens: 0 } })['budget.maxTokens']).toBeTruthy();
    expect(validateAgent({ ...base, retryLimit: 9 }).retryLimit).toBeTruthy();
    expect(validateAgent({ ...base, knowledgeSources: ['http://insecure.example'] }).knowledgeSources).toBeTruthy();
    expect(validateAgent({ ...base, knowledgeSources: ['docs/a.md', 'https://example.com/x'] })).toEqual({});
    expect(validateAgent({ ...base, scope: { projects: 'current', paths: ['../etc'] } }).scope).toBeTruthy();
  });

  it('accepts only project-relative paths', () => {
    for (const ok of ['src', 'src/components', 'docs/a.md']) expect(isSafeRelPath(ok)).toBe(true);
    for (const bad of ['/etc/passwd', '../x', 'a/../../b', 'C:\\x', '~/x', 'file://x', '']) expect(isSafeRelPath(bad)).toBe(false);
  });

  it('normalizes the payload (manual model only for the manual policy)', () => {
    const n = normalizeAgent({ ...blankAgent(), name: '  Pixel ', model: 'm', specialization: ['ui', 'ui', ' '], modelPolicy: 'auto' });
    expect(n.name).toBe('Pixel');
    expect(n.model).toBeNull();
    expect(n.specialization).toEqual(['ui']);
  });

  it('lays tasks out in dependency layers and tolerates cycles / unknown ids', () => {
    const g = layoutGraph(missionFixture.tasks);
    expect(g.nodes.map((n) => [n.task.id, n.layer])).toEqual([['t-a', 0], ['t-b', 1], ['t-c', 2]]);
    expect(g.edges.map((e) => `${e.from}>${e.to}`)).toEqual(['t-a>t-b', 't-b>t-c']);
    const cyc = missionFixture.tasks.map((t, i) => (i === 0 ? { ...t, dependsOn: ['t-c', 'ghost'] } : t));
    expect(() => layoutGraph(cyc)).not.toThrow();
  });

  it('animates only working, planning and reviewing', () => {
    const all = ['idle', 'planning', 'working', 'blocked', 'awaiting_approval', 'reviewing', 'completed', 'failed', 'paused'] as const;
    expect(all.filter(isAnimatedStatus)).toEqual(['planning', 'working', 'reviewing']);
    expect(moodForStatus('working')).toBe('working');
    expect(moodForStatus('failed')).toBe('sad');
    expect(moodForStatus('idle')).toBe('idle');
  });
});

describe('agents UI', () => {
  beforeEach(() => {
    resetStore();
    installIpc();
    useApp.setState({ route: { name: 'home' } });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('creator validates inline and saves the exact payload shape', async () => {
    await openAgents();
    await click(screen.getByRole('button', { name: 'New' }));
    await click(screen.getByText('Blank agent'));
    expect(screen.getByRole('dialog', { name: 'Create an agent' })).toBeInTheDocument();

    // every advanced section is present in Advanced mode
    const dialog = within(screen.getByRole('dialog'));
    for (const t of ['Model', 'Tools', 'Permissions', 'Budgets', 'Quality bar']) expect(dialog.getByRole('heading', { name: t })).toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: /read_file/ })).toBeChecked();
    expect(screen.getByRole('checkbox', { name: /^write_file —/ })).not.toBeChecked();

    await click(screen.getByRole('button', { name: 'Create agent' }));
    expect(screen.getByText('Give the agent a name.')).toBeInTheDocument();
    expect(screen.getByText(/at least 20 characters/)).toBeInTheDocument();
    expect(lastCall('save_agent')).toBeUndefined();

    await type(screen.getByLabelText('Name'), 'Pixel');
    await type(screen.getByLabelText('Role'), 'Tester');
    await click(screen.getByRole('button', { name: 'Insert template' }));
    expect((screen.getByLabelText('Instructions') as HTMLTextAreaElement).value).toContain('You are a careful Tester.');
    await act(async () => {
      fireEvent.change(screen.getByRole('textbox', { name: 'Specializations' }), { target: { value: 'unit tests' } });
      fireEvent.keyDown(screen.getByRole('textbox', { name: 'Specializations' }), { key: 'Enter' });
    });
    await click(screen.getByRole('radio', { name: 'Emoji 🧪' }));
    await click(screen.getByRole('checkbox', { name: /run_check/ }));

    await click(screen.getByRole('button', { name: 'Create agent' }));
    const saved = lastCall('save_agent')!.args.agent as Record<string, unknown>;
    expect(saved).toMatchObject({
      name: 'Pixel', avatar: '🧪', role: 'Tester', specialization: ['unit tests'], knowledgeSources: [], modelPolicy: 'auto', model: null,
      scope: { projects: 'current', paths: [] }, network: false, memoryPolicy: 'project', budget: BUDGET_DEFAULTS, retryLimit: 1, approval: 'ask',
      acceptanceCriteria: '', outputRequirements: '', builtin: false,
    });
    expect(saved.tools).toEqual([...DEFAULT_TOOLS, 'run_check']);
    expect('id' in saved).toBe(false);
    expect(saved.createdAt).toEqual(expect.any(String));
    // saved agent is in the list afterwards
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(useAgents.getState().agents.some((a) => a.id === 'agent-new')).toBe(true);
  });

  it('manual model policy needs a model; budgets above the maximum are rejected', async () => {
    installIpc({ list_models: () => ({ models: [{ id: 'nvidia/some-model', owner: null, created: null, inferredCategory: 'chat' }], fetchedAt: null, source: 'cache', stale: false, baseUrl: '', probes: {} }) });
    await openAgents();
    await click(screen.getByRole('button', { name: 'New' }));
    await click(screen.getByText('Blank agent'));
    await type(screen.getByLabelText('Name'), 'Pixel');
    await type(screen.getByLabelText('Role'), 'Tester');
    await type(screen.getByLabelText('Instructions'), 'Always test the thing carefully and report.');
    await click(screen.getByRole('radio', { name: 'Manual' }));
    await type(screen.getByLabelText(/^Steps/), '500');
    await click(screen.getByRole('button', { name: 'Create agent' }));
    expect(screen.getByText(/Choose a model/, { selector: '.field-error' })).toBeInTheDocument();
    expect(screen.getByText(/Steps is limited to 100/)).toBeInTheDocument();
    expect(lastCall('save_agent')).toBeUndefined();

    await type(screen.getByLabelText(/^Steps/), '50');
    await type(screen.getByLabelText(/Model \(advanced\)/), 'nvidia/some-model');
    await click(screen.getByRole('button', { name: 'Create agent' }));
    expect(lastCall('save_agent')!.args.agent).toMatchObject({ modelPolicy: 'manual', model: 'nvidia/some-model', budget: { ...BUDGET_DEFAULTS, maxSteps: 50 } });
  });

  it('templates start prefilled and builtin agents are duplicated, not edited', async () => {
    await openAgents();
    expect(screen.getByRole('button', { name: 'Duplicate to customize' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Edit' })).toBeNull();
    await click(screen.getByRole('button', { name: 'Duplicate to customize' }));
    expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Designer (copy)');
    await click(screen.getByRole('button', { name: 'Create agent' }));
    const saved = lastCall('save_agent')!.args.agent as Record<string, unknown>;
    expect(saved.builtin).toBe(false);
    expect('id' in saved).toBe(false);
    await flush();

    await click(screen.getByRole('button', { name: 'New' }));
    await click(within(screen.getByRole('dialog')).getByText('Designer', { selector: 'b' }));
    expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('Designer');
    expect((screen.getByLabelText('Role') as HTMLInputElement).value).toBe('UI designer');
  });

  it('status badges follow genius://agent-status events and animation is off when idle', async () => {
    installIpc({ agent_runtimes: () => [runtime({ status: 'idle' })], list_missions: () => [missionFixture] });
    await openAgents();
    const badge = () => within(screen.getByRole('list', { name: 'Agents' })).getByTestId('agent-status');
    const avatar = () => document.querySelector('.ag-item .ag-avatar') as HTMLElement;
    expect(badge()).toHaveTextContent('Idle');
    expect(badge().dataset.animated).toBe('false');
    expect(avatar().dataset.animated).toBe('false');
    expect(avatar().querySelector('.genius-avatar.still')).not.toBeNull();
    expect(document.querySelector('.ag-item .spinner')).toBeNull();

    await send('genius://agent-status', runtime({ status: 'working', currentTaskId: 't-b' }));
    expect(badge()).toHaveTextContent('Working');
    expect(badge().dataset.animated).toBe('true');
    expect(avatar().dataset.animated).toBe('true');
    expect(avatar().querySelector('.genius-avatar.still')).toBeNull();
    expect(avatar().querySelector('.genius-avatar.mood-working')).not.toBeNull();
    expect(within(screen.getByRole('list', { name: 'Agents' })).getByText('Build the sidebar')).toBeInTheDocument();

    for (const [status, label, animated] of [['awaiting_approval', 'Awaiting approval', 'false'], ['planning', 'Planning', 'true'], ['failed', 'Failed', 'false'], ['reviewing', 'Reviewing', 'true'], ['paused', 'Paused', 'false'], ['completed', 'Completed', 'false']] as const) {
      await send('genius://agent-status', runtime({ status }));
      expect(badge()).toHaveTextContent(label);
      expect(badge().dataset.animated).toBe(animated);
    }
    await send('genius://agent-status', runtime({ status: 'idle' }));
    expect(avatar().dataset.animated).toBe('false');
  });

  it('shows the live activity log and usage against the budget', async () => {
    installIpc({
      agent_runtimes: () => [runtime({ status: 'working', currentTaskId: 't-b', usage: { tokens: 5000, calls: 7, tasksDone: 2, tasksFailed: 1 } })],
      list_missions: () => [missionFixture],
      agent_logs: () => [{ id: 'l1', agentId: 'agent-designer', missionId: 'm1', taskId: 't-b', projectId: 'p1', kind: 'tool', text: 'read_file src/App.tsx', data: null, at: '2026-10-01T00:00:00Z' }],
    });
    await openAgents();
    const log = screen.getByRole('log', { name: 'Activity log' });
    expect(within(log).getByText('read_file src/App.tsx')).toBeInTheDocument();
    await send('genius://agent-log', { id: 'l2', agentId: 'agent-designer', missionId: 'm1', taskId: 't-b', projectId: 'p1', kind: 'result', text: 'Sidebar drafted', data: null, at: '2026-10-01T00:00:05Z' });
    expect(within(log).getByText('Sidebar drafted')).toBeInTheDocument();
    const tokens = screen.getByRole('progressbar', { name: 'Tokens (this task)' });
    expect(tokens.getAttribute('aria-valuenow')).toBe('800');
    expect(tokens.getAttribute('aria-valuemax')).toBe(String(builtinAgent.budget.maxTokens));
    expect(screen.getByText(/5,000 tokens · 7 calls · 2 done · 1 failed/)).toBeInTheDocument();
  });

  it('gives an agent a task with the chosen project and shows the live result', async () => {
    installIpc({ run_agent_task: () => mission({ status: 'running' }, [{ ...missionFixture.tasks[0], status: 'running', result: null }]) });
    await openAgents();
    const run = screen.getByRole('button', { name: 'Run task' });
    expect(run).toBeDisabled();
    await type(screen.getByLabelText('Task'), 'Review the sign-in form');
    await click(screen.getByRole('button', { name: 'Run task' }));
    expect(lastCall('run_agent_task')!.args).toEqual({ agentId: 'agent-designer', projectId: 'p1', task: 'Review the sign-in form' });
    const result = screen.getByRole('region', { name: 'Task result' });
    expect(within(result).getByText('Design the layout')).toBeInTheDocument();
    expect(within(result).getAllByText('Running').length).toBeGreaterThan(0);
    await send('genius://mission-update', mission({ status: 'completed', updatedAt: '2026-10-01T00:00:09Z' }, [{ ...missionFixture.tasks[0], status: 'done', result: 'All good: no issues found.' }]));
    expect(within(result).getByText('All good: no issues found.')).toBeInTheDocument();
    expect(within(result).getAllByText('Completed').length).toBeGreaterThan(0);
  });

  it('tells the truth when the backend is not ready (inline error + toast)', async () => {
    installIpc({
      list_agents: () => { throw { kind: 'not_ready', message: 'agent runtime is not ready' }; },
      run_agent_task: () => { throw { kind: 'not_ready', message: 'agent runtime is not ready' }; },
    });
    await openAgents();
    expect(screen.getByText('Agents are not fully available')).toBeInTheDocument();
    expect(screen.getByText(/agent runtime is not ready/)).toBeInTheDocument();
    // a failing command shows an honest toast
    useAgents.setState({ agents: [builtinAgent], selectedAgentId: builtinAgent.id, projects: [{ id: 'p1', name: 'Dashboard' } as never] });
    await flush();
    await type(screen.getByLabelText('Task'), 'Do something useful');
    await click(screen.getByRole('button', { name: 'Run task' }));
    expect(useApp.getState().toasts.some((t) => t.tone === 'bad' && t.text.includes('Could not start the task: agent runtime is not ready'))).toBe(true);
  });

  it('simple mode shows a friendly page with templates, create and run a task', async () => {
    installIpc({ get_settings: () => ({ ...settings, uiMode: 'simple' }) });
    await openAgents();
    expect(screen.getByRole('heading', { name: 'Your agents' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Create an agent' })).toBeInTheDocument();
    expect(screen.getByText('Start from a template')).toBeInTheDocument();
    expect(screen.queryByRole('log')).toBeNull();
    await click(screen.getByRole('button', { name: 'Give a task' }));
    await type(screen.getByLabelText('Task'), 'Check my landing page');
    await click(screen.getByRole('button', { name: 'Run task' }));
    expect(lastCall('run_agent_task')!.args).toMatchObject({ agentId: 'agent-designer', task: 'Check my landing page' });
    // creator in simple mode hides the advanced sections but keeps safe defaults
    await click(screen.getByRole('button', { name: 'Customize a copy' }));
    expect(screen.queryByRole('heading', { name: 'Budgets' })).toBeNull();
    expect(screen.getByText('Safe defaults are on')).toBeInTheDocument();
  });

  it('mission board renders the dependency graph from the mission and follows live updates', async () => {
    installIpc({ list_missions: () => [missionFixture] });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    const graph = screen.getByRole('group', { name: 'Task dependency graph' });
    const nodes = Array.from(graph.querySelectorAll('[data-task-id]'));
    expect(nodes.map((n) => [n.getAttribute('data-task-id'), n.getAttribute('data-status')])).toEqual([['t-a', 'done'], ['t-b', 'awaiting_approval'], ['t-c', 'blocked']]);
    expect(graph.querySelectorAll('path.mg-edge')).toHaveLength(2);
    expect(graph.querySelector('path[data-from="t-a"][data-to="t-b"]')).not.toBeNull();
    // totals are computed from the real tasks
    const totals = screen.getByLabelText('Mission totals');
    expect(totals).toHaveTextContent('1/3 done');
    expect(totals).toHaveTextContent('1 blocked');
    expect(totals).toHaveTextContent('1 need approval');
    expect(totals).toHaveTextContent('2.0k tokens');
    expect(totals).toHaveTextContent('5 model calls');
    expect(screen.getByRole('region', { name: 'Needs your attention' })).toHaveTextContent('Waiting for the sidebar');
    // nothing is pulsing: no task is running
    expect(graph.querySelector('.mg-node.live')).toBeNull();

    // live update: sidebar approved and running, tests no longer blocked
    await send('genius://mission-update', mission({ updatedAt: '2026-10-01T00:00:09Z' }, [
      missionFixture.tasks[0],
      { ...missionFixture.tasks[1], status: 'running' },
      { ...missionFixture.tasks[2], status: 'pending', blocker: null },
    ]));
    expect(graph.querySelector('[data-task-id="t-b"]')!.getAttribute('data-status')).toBe('running');
    expect(graph.querySelector('[data-task-id="t-b"].live')).not.toBeNull();
    expect(screen.queryByRole('region', { name: 'Needs your attention' })).toBeNull();

    // an older snapshot never overwrites newer state
    await send('genius://mission-update', mission({ updatedAt: '2026-10-01T00:00:02Z' }));
    expect(graph.querySelector('[data-task-id="t-b"]')!.getAttribute('data-status')).toBe('running');

    // board view
    await click(screen.getByRole('radio', { name: 'Board' }));
    const board = screen.getByRole('group', { name: 'Task board' });
    expect(within(within(board).getByRole('region', { name: 'In progress' })).getByText('Build the sidebar')).toBeInTheDocument();
    expect(within(within(board).getByRole('region', { name: 'Done' })).getByText('Design the layout')).toBeInTheDocument();
  });

  it('graph nodes are keyboard accessible', async () => {
    installIpc({ list_missions: () => [missionFixture] });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    const node = screen.getByRole('button', { name: /Task Write tests, Blocked/ });
    await act(async () => void fireEvent.keyDown(node, { key: 'Enter' }));
    const panel = screen.getByRole('complementary', { name: 'Task details' });
    expect(within(panel).getByText('Waiting for the sidebar')).toBeInTheDocument();
    expect(within(panel).getByText('Build the sidebar', { selector: 'dd' })).toBeInTheDocument(); // depends on
  });

  it('mission controls call the matching commands', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    let current: Mission = mission({ status: 'ready' });
    installIpc({
      list_missions: () => [current],
      start_mission: () => (current = { ...current, status: 'running', updatedAt: '2026-10-01T00:01:00Z' }),
      pause_mission: () => (current = { ...current, status: 'paused', updatedAt: '2026-10-01T00:02:00Z' }),
      resume_mission: () => (current = { ...current, status: 'running', updatedAt: '2026-10-01T00:03:00Z' }),
      cancel_mission: () => (current = { ...current, status: 'cancelled', updatedAt: '2026-10-01T00:04:00Z' }),
    });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    expect(screen.queryByRole('button', { name: /Pause/ })).toBeNull();
    await click(screen.getByRole('button', { name: /Start mission/ }));
    expect(lastCall('start_mission')!.args).toEqual({ missionId: 'm1' });
    await click(screen.getByRole('button', { name: /Pause/ }));
    expect(lastCall('pause_mission')!.args).toEqual({ missionId: 'm1' });
    await click(screen.getByRole('button', { name: /Resume/ }));
    expect(lastCall('resume_mission')!.args).toEqual({ missionId: 'm1' });
    await click(screen.getByRole('button', { name: 'Cancel mission' }));
    expect(lastCall('cancel_mission')!.args).toEqual({ missionId: 'm1' });
    expect(screen.queryByRole('button', { name: 'Cancel mission' })).toBeNull(); // terminal: no more controls
    expect(screen.queryByRole('button', { name: /Start mission/ })).toBeNull();
  });

  it('task controls: retry, cancel and reprioritize', async () => {
    const tasks: MissionTask[] = [
      { ...missionFixture.tasks[0], id: 't-f', title: 'Broken task', status: 'failed', result: null, dependsOn: [] },
      { ...missionFixture.tasks[0], id: 't-p', title: 'Queued task', status: 'pending', priority: 2, result: null, dependsOn: [] },
    ];
    const same = () => mission({}, tasks);
    installIpc({ list_missions: () => [same()], retry_mission_task: same, cancel_mission_task: same, reprioritize_task: same });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    await click(screen.getByRole('button', { name: /Task Broken task, Failed/ }));
    await click(screen.getByRole('button', { name: 'Retry task' }));
    expect(lastCall('retry_mission_task')!.args).toEqual({ taskId: 't-f' });
    expect(screen.queryByLabelText(/Priority \(0/)).toBeNull(); // failed tasks are not reprioritized

    await click(screen.getByRole('button', { name: /Task Queued task, Waiting/ }));
    await type(screen.getByLabelText(/Priority \(0/), '9');
    await click(screen.getByRole('button', { name: 'Set priority' }));
    expect(lastCall('reprioritize_task')!.args).toEqual({ taskId: 't-p', priority: 9 });
    await click(screen.getByRole('button', { name: 'Cancel task' }));
    expect(lastCall('cancel_mission_task')!.args).toEqual({ taskId: 't-p' });
  });

  it('approval flow: review the isolated changes, approve or reject', async () => {
    installIpc({ list_missions: () => [missionFixture] });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    // the attention banner jumps to the task that needs a decision
    await click(within(screen.getByRole('region', { name: 'Needs your attention' })).getByRole('button', { name: /Build the sidebar/ }));
    const panel = screen.getByRole('complementary', { name: 'Task details' });
    expect(within(panel).getByText('This task is waiting for your decision')).toBeInTheDocument();
    expect(within(panel).getByText('agent/t-b')).toBeInTheDocument(); // isolated workspace

    await click(within(panel).getByRole('button', { name: 'Review the isolated changes' }));
    expect(lastCall('get_changeset')!.args).toEqual({ changesetId: 'cs1' });
    expect(screen.getByRole('dialog', { name: 'Changes: Build the sidebar' })).toBeInTheDocument();
    await click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Close' }));

    await click(within(panel).getByRole('button', { name: 'Approve' }));
    expect(lastCall('approve_mission_task')!.args).toEqual({ taskId: 't-b', approved: true });
    await click(within(panel).getByRole('button', { name: 'Reject' }));
    expect(lastCall('approve_mission_task')!.args).toEqual({ taskId: 't-b', approved: false });
  });

  it('a failing mission command shows an honest error and keeps the state', async () => {
    installIpc({ list_missions: () => [mission({ status: 'running' })], pause_mission: () => { throw { kind: 'conflict', message: 'mission is already finishing' }; } });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: /Ship the dashboard/ }));
    await click(screen.getByRole('button', { name: /Pause/ }));
    expect(useApp.getState().toasts.some((t) => t.tone === 'bad' && t.text === 'Could not pause the mission: mission is already finishing')).toBe(true);
    expect(screen.getByRole('button', { name: /Pause/ })).toBeInTheDocument();
  });

  it('creates a mission from a goal, a project and a team and shows the planning state', async () => {
    installIpc({
      list_teams: () => [{ id: 'team1', name: 'Web team', description: '', coordinatorAgentId: 'agent-designer', agentIds: ['agent-designer'], createdAt: '2026-10-01T00:00:00Z' }],
      create_mission: () => mission({ id: 'm9', title: 'Add dark mode', goal: 'Add dark mode', status: 'planning', updatedAt: '2026-10-02T00:00:00Z' }, []),
    });
    await openAgents('missions');
    await click(screen.getByRole('button', { name: 'New' }));
    await click(screen.getByRole('button', { name: 'Plan mission' }));
    expect(screen.getByText(/at least a sentence/)).toBeInTheDocument();
    expect(lastCall('create_mission')).toBeUndefined();
    await type(screen.getByLabelText('Goal'), 'Add dark mode to the dashboard');
    await type(screen.getByLabelText('Project'), 'p1');
    await type(screen.getByLabelText('Team'), 'team1');
    await click(screen.getByRole('button', { name: 'Plan mission' }));
    expect(lastCall('create_mission')!.args).toEqual({ projectId: 'p1', goal: 'Add dark mode to the dashboard', teamId: 'team1' });
    expect(screen.getByText('The coordinator is planning the tasks…')).toBeInTheDocument();
    // the coordinator's plan arrives by event and the graph appears
    await send('genius://mission-update', mission({ id: 'm9', title: 'Add dark mode', status: 'ready', updatedAt: '2026-10-02T00:00:05Z' }));
    expect(screen.getByRole('group', { name: 'Task dependency graph' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Start mission/ })).toBeInTheDocument();
  });

  it('teams manager validates and saves a team with its coordinator among the members', async () => {
    installIpc({ list_agents: () => [builtinAgent, { ...builtinAgent, id: 'agent-tester', name: 'Tester', role: 'QA', builtin: false }] });
    await openAgents('teams');
    await click(screen.getByRole('button', { name: 'New team' }));
    await click(screen.getByRole('button', { name: 'Create team' }));
    expect(screen.getByText('Give the team a name.')).toBeInTheDocument();
    expect(screen.getByText(/Choose the coordinator/)).toBeInTheDocument();
    await type(screen.getByLabelText('Team name'), 'Web team');
    await type(screen.getByLabelText('Coordinator'), 'agent-designer');
    await click(screen.getByRole('button', { name: 'Create team' }));
    expect(screen.getByText(/at least one member/)).toBeInTheDocument();
    await click(screen.getByRole('checkbox', { name: /Tester/ }));
    await click(screen.getByRole('button', { name: 'Create team' }));
    expect(lastCall('save_team')!.args.team).toMatchObject({ name: 'Web team', coordinatorAgentId: 'agent-designer', agentIds: ['agent-designer', 'agent-tester'] });
    expect(screen.getByRole('list', { name: 'Teams' })).toHaveTextContent('Web team');
  });

  it('the rail button shows how many agent tasks wait for approval', async () => {
    render(<StrictMode><App /></StrictMode>);
    await flush();
    const rail = screen.getByRole('button', { name: 'Agents' });
    expect(rail.querySelector('.count')).toBeNull();
    await send('genius://mission-update', missionFixture);
    expect(screen.getByRole('button', { name: 'Agents' }).querySelector('.count')).toHaveTextContent('1');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Agents' })));
    await flush(60);
    expect(useApp.getState().route.name).toBe('agents');
  });
});
