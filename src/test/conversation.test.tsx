// The project conversation UI against a mocked backend (labelled: no real model involved).
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc } from './ipc';
import AssistantPanel from '../views/workspace/AssistantPanel';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';
import { project, settings } from './ipc';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));

describe('project conversation', () => {
  beforeEach(() => {
    installIpc();
    useApp.setState({ settings });
    useWorkspace.setState({ projectId: 'p1', project, tasks: [], activity: [], agentSteps: {} });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('shows persisted history with kinds, live proposal status and an honest report', async () => {
    render(<StrictMode><AssistantPanel /></StrictMode>);
    await flush();
    expect(screen.getByText('Add a sidebar')).toBeInTheDocument();
    expect(screen.getByText('Proposed changes')).toBeInTheDocument();
    expect(screen.getByText('awaiting your review')).toBeInTheDocument();
    expect(screen.getByText(/Not verified: No build or test/)).toBeInTheDocument();
    expect(screen.getByText(/no successful build or test ran/)).toBeInTheDocument();
  });

  it('sends follow-ups to the agent in the selected conversation and mode', async () => {
    render(<AssistantPanel />);
    await flush();
    fireEvent.change(screen.getByLabelText('Change request'), { target: { value: 'Make the cards smaller' } });
    fireEvent.click(screen.getByRole('radio', { name: 'Plan only' }));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Send/ })));
    await flush();
    const call = calls.find((c) => c.cmd === 'start_agent');
    expect(call?.args).toEqual({ projectId: 'p1', request: { conversationId: 'c1', message: 'Make the cards smaller', mode: 'plan', attachmentIds: [], pinned: [] } });
  });

  it('creates a new conversation without losing the old one', async () => {
    render(<AssistantPanel />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'New conversation' })));
    await flush();
    const options = screen.getAllByRole('option').map((o) => o.textContent);
    expect(options).toEqual(['New conversation (0)', 'Project conversation (2)']);
  });

  it('renders live agent steps with the agent that ran them, and Stop cancels', async () => {
    useWorkspace.setState({
      tasks: [{ id: 't9', projectId: 'p1', kind: 'agent', title: 'Genius: fix mobile', startedAt: '2026-10-01T00:00:00Z' }],
      agentSteps: {
        t9: [
          { id: 's1', projectId: 'p1', taskId: 't9', conversationId: 'c1', agent: 'main', seq: 1, tool: 'read_file', args: { path: 'src/Sidebar.tsx' }, ok: true, summary: 'Read src/Sidebar.tsx (20 lines)', result: {}, startedAt: '', endedAt: '' },
          { id: 's2', projectId: 'p1', taskId: 't9', conversationId: 'c1', agent: 'reviewer', seq: 2, tool: 'read_file', args: { path: '.env' }, ok: false, summary: 'read_file failed: excluded', result: {}, startedAt: '', endedAt: '' },
        ],
      },
    });
    render(<AssistantPanel />);
    await flush();
    expect(screen.getByText(/Read src\/Sidebar.tsx — Read src\/Sidebar.tsx \(20 lines\)/)).toBeInTheDocument();
    expect(screen.getByText('reviewer')).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getAllByRole('button', { name: /Stop/ })[0]));
    expect(calls.some((c) => c.cmd === 'cancel_task' && c.args.taskId === 't9')).toBe(true);
  });
});
