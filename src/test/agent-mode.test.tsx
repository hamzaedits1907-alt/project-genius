// Agent Mode: honest outcome states, the launch flow (plan → automatic start) and the approval card,
// against the mocked IPC. The backend mission engine itself is covered by the Rust mission tests and
// the opt-in live mission test.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, missionFixture, settings } from './ipc';
import AgentMode from '../views/agents/AgentMode';
import { useAgents } from '../store/agents';
import { useApp } from '../store/app';
import { missionOutcome, taskOutcome } from '../lib/agents';
import type { Mission, MissionTask } from '../lib/types';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const task = (over: Partial<MissionTask> = {}): MissionTask => ({ ...missionFixture.tasks[0], status: 'done', ...over });
const mission = (over: Partial<Mission> = {}, tasks: MissionTask[] = [task()]): Mission => ({ ...missionFixture, status: 'completed', summary: 'Mission done.', ...over, tasks });

describe('outcomes are never better than the evidence', () => {
  it('done without a passing check is completed-but-unverified, never verified', () => {
    expect(taskOutcome(task())).toBe('completed_unverified');
    expect(taskOutcome(task({ verification: { build: 'passed', tests: 'unknown', notes: '' } }))).toBe('verified');
    expect(taskOutcome(task({ verification: { build: 'passed', tests: 'failed', notes: '' } }))).toBe('checks_failed');
    expect(taskOutcome(task({ status: 'awaiting_approval' }))).toBe('waiting');
    expect(taskOutcome(task({ status: 'blocked' }))).toBe('blocked');
    expect(taskOutcome(task({ status: 'failed' }))).toBe('failed');
    expect(taskOutcome(task({ status: 'cancelled' }))).toBe('cancelled');
    expect(taskOutcome(task({ status: 'running' }))).toBe('running');
  });

  it('a mission is verified only when every finished step is', () => {
    const ok = task({ verification: { build: 'passed', tests: 'passed', notes: '' } });
    expect(missionOutcome(mission({}, [ok]))).toBe('verified');
    expect(missionOutcome(mission({}, [ok, task()]))).toBe('completed_unverified');
    expect(missionOutcome(mission({}, [ok, task({ verification: { build: 'failed', tests: 'unknown', notes: '' } })]))).toBe('checks_failed');
    expect(missionOutcome(mission({}, []))).toBe('completed_unverified');
    expect(missionOutcome(mission({ status: 'failed' }))).toBe('failed');
    expect(missionOutcome(mission({ status: 'running' }, [task({ status: 'awaiting_approval' })]))).toBe('waiting');
    expect(missionOutcome(mission({ status: 'planning' }, []))).toBe('planning');
  });
});

describe('Agent Mode screen', () => {
  beforeEach(() => {
    useAgents.setState({ loaded: true, loading: false, loadError: null, agents: [], templates: [], runtimes: {}, teams: [], missions: [], projects: [], logs: {}, tab: 'run', selectedAgentId: null, selectedMissionId: null, selectedTaskId: null });
    useApp.setState({ settings: { ...settings, modelMode: 'auto' }, keyStatus: { stored: true, hint: 'nvapi-…', backend: 'memory', error: null } });
    installIpc({ list_projects: () => [] });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('creates the scratch workspace once, plans, and starts the mission by itself when the plan is ready', async () => {
    const planning = { ...missionFixture, id: 'mx', status: 'planning' as const, tasks: [] };
    installIpc({
      list_projects: () => [],
      list_missions: () => [],
      create_project: () => ({ id: 'p-agent', name: 'Agent workspace', description: '', path: '/tmp/a', kind: 'vite-react', spec: null, notes: '', status: 'empty', createdAt: '', updatedAt: '', lastOpenedAt: null }),
      create_mission: () => planning,
      start_mission: () => ({ ...planning, status: 'running', tasks: [task({ status: 'running' })] }),
    });
    render(<AgentMode />);
    await flush();
    fireEvent.change(screen.getByLabelText('Mission'), { target: { value: 'Create docs/overview.md that lists every file' } });
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Start mission' })));
    await flush(60);
    expect(calls.some((c) => c.cmd === 'create_project' && c.args.name === 'Agent workspace')).toBe(true);
    const created = calls.find((c) => c.cmd === 'create_mission');
    expect(created?.args.projectId).toBe('p-agent');
    expect(created?.args.teamId).toBeNull();
    expect(calls.some((c) => c.cmd === 'start_mission'), 'not started before the plan exists').toBe(false);
    expect(screen.getByText(/coordinator is planning/i)).toBeTruthy();

    // the runtime reports the plan: Genius starts the mission without another click
    await act(async () => {
      // what the `genius://mission-update` listener in store/events.ts does with the event
      useAgents.getState().onMission({ ...planning, status: 'ready', updatedAt: '2026-10-01T00:00:09Z', tasks: [task({ status: 'ready' })] });
    });
    await flush(60);
    expect(calls.filter((c) => c.cmd === 'start_mission')).toHaveLength(1);
  });

  it('shows a finished-but-unchecked mission as such, with an honest report', async () => {
    useAgents.setState({ missions: [mission({}, [task({ changedPaths: ['docs/inventory.md'] })])], selectedMissionId: 'm1' });
    render(<AgentMode />);
    await flush();
    const report = screen.getByRole('region', { name: 'Final report' });
    expect(report.textContent).toMatch(/Completed, not verified/);
    expect(report.textContent).toMatch(/no build or test confirmed it/);
    expect(report.textContent).toMatch(/docs\/inventory\.md/);
    expect(report.textContent).not.toMatch(/^Verified/);
  });

  it('puts a step that waits for approval in front of the user with review, approve and reject', async () => {
    useAgents.setState({ missions: [mission({ status: 'running' }, [task({ status: 'awaiting_approval', changesetId: 'cs1', changedPaths: ['a.md'] })])], selectedMissionId: 'm1' });
    installIpc({ approve_mission_task: () => ({ ...missionFixture, status: 'running' }) });
    render(<AgentMode />);
    await flush();
    const card = screen.getByRole('region', { name: 'Needs your decision' });
    expect(card.textContent).toMatch(/Nothing in your workspace has changed yet/);
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Approve and apply/ })));
    await flush();
    expect(calls.find((c) => c.cmd === 'approve_mission_task')?.args).toMatchObject({ approved: true });
  });

  it('refuses to start without a provider and says why', async () => {
    useApp.setState({ keyStatus: { stored: false, hint: null, backend: 'memory', error: null } });
    render(<AgentMode />);
    await flush();
    expect(screen.getByText(/Connect a provider first/)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Start mission' }) as HTMLButtonElement).disabled).toBe(true);
  });
});
