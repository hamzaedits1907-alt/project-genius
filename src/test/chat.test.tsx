// The chat experience: slash commands, @-mentions, effort/autopilot, live thinking, steering,
// message actions and run reports — against the mocked backend (no real model involved).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { emit } from '@tauri-apps/api/event';
import { calls, clearMocks, installIpc, messages, project, settings, type Handler } from './ipc';
import AssistantPanel from '../views/workspace/AssistantPanel';
import { resetCommandCache } from '../views/workspace/assistant/useCommands';
import { filterCommands, fuzzyScore, mentionQuery, parseCommand, slashQuery, BUILTIN_COMMANDS } from '../views/workspace/assistant/commands';
import { Report } from '../views/workspace/assistant/Message';
import { Markdown } from '../components/Markdown';
import { groupSteps, reportFooter, stepLabel } from '../lib/agent';
import { subscribeAll } from '../store/events';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';
import type { AgentReport, AgentStep, MessageRow } from '../lib/types';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const box = () => screen.getByLabelText('Change request') as HTMLTextAreaElement;
const type = async (text: string) => {
  await act(async () => void fireEvent.change(box(), { target: { value: text } }));
};
const key = async (k: string) => {
  await act(async () => void fireEvent.keyDown(box(), { key: k }));
  await flush();
};
const startCalls = () => calls.filter((c) => c.cmd === 'start_agent').map((c) => c.args.request as Record<string, unknown>);
const running = { id: 't9', projectId: 'p1', kind: 'agent', title: 'Genius: fix mobile', startedAt: '2026-10-01T00:00:00Z' };

const report = (over: Partial<AgentReport> = {}): AgentReport => ({ steps: 1, filesRead: [], searches: 0, checks: [], subagents: [], failedSteps: [], changeset: null, verification: null, notVerified: [], warnings: [], error: null, ...over });
const step = (over: Partial<AgentStep>): AgentStep => ({ id: Math.random().toString(36), projectId: 'p1', taskId: 't9', conversationId: 'c1', agent: 'main', seq: 1, tool: 'read_file', args: {}, ok: true, summary: '', result: {}, startedAt: '', endedAt: '', ...over });

function mount(overrides: Record<string, Handler> = {}, variant: 'simple' | 'advanced' = 'advanced') {
  installIpc(overrides);
  return render(<AssistantPanel variant={variant} />);
}

beforeEach(() => {
  try {
    localStorage.clear();
  } catch {
    /* ignore */
  }
  resetCommandCache();
  useApp.setState({ settings, route: { name: 'project', id: 'p1' } });
  useWorkspace.setState({ projectId: 'p1', project, tasks: [], activity: [], agentSteps: {}, thinking: {}, queue: [], files: [{ path: 'src/App.tsx', size: 10, isText: true, sensitive: false }, { path: 'src/components/Header.tsx', size: 10, isText: true, sensitive: false }, { path: '.env', size: 3, isText: true, sensitive: true }] });
});
afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

describe('slash commands', () => {
  it('opens a filtered popup and runs /plan with Enter', async () => {
    mount();
    await flush();
    await type('/pl');
    const list = screen.getByRole('listbox', { name: 'Commands' });
    const names = within(list).getAllByRole('option').map((o) => o.textContent);
    expect(names[0]).toContain('/plan');
    expect(names.some((n) => n?.includes('/review'))).toBe(false);
    expect(screen.getByRole('radio', { name: 'Build' })).toHaveAttribute('aria-checked', 'true');
    await key('Enter');
    expect(screen.getByRole('radio', { name: 'Plan only' })).toHaveAttribute('aria-checked', 'true');
    expect(box().value).toBe('');
    expect(screen.queryByRole('listbox')).toBeNull();
    // /plan toggles back
    await type('/plan');
    await key('Enter');
    expect(screen.getByRole('radio', { name: 'Build' })).toHaveAttribute('aria-checked', 'true');
  });

  it('navigates with arrows, closes with Esc, and Tab completes a command that takes arguments', async () => {
    mount();
    await flush();
    await type('/');
    const options = () => within(screen.getByRole('listbox')).getAllByRole('option');
    expect(options()[0]).toHaveAttribute('aria-selected', 'true');
    await key('ArrowDown');
    expect(options()[1]).toHaveAttribute('aria-selected', 'true');
    await key('ArrowUp');
    await key('ArrowUp'); // wraps around
    expect(options()[options().length - 1]).toHaveAttribute('aria-selected', 'true');
    await key('Escape');
    expect(screen.queryByRole('listbox')).toBeNull();
    await type('/eff');
    await key('Tab');
    expect(box().value).toBe('/effort ');
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
  });

  it('/effort sets the effort and /autopilot toggles it, both sent with the next request', async () => {
    mount();
    await flush();
    await type('/effort fast');
    await key('Enter');
    expect(screen.getByLabelText('Effort')).toHaveValue('fast');
    await type('/autopilot on');
    await key('Enter');
    expect(screen.getByRole('button', { name: 'Autopilot' })).toHaveAttribute('aria-pressed', 'true');
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
    await type('Make it faster');
    await key('Enter');
    expect(startCalls()[0]).toMatchObject({ message: 'Make it faster', effort: 'fast', autopilot: true });
  });

  it('sends prompt commands as message text for the backend to expand', async () => {
    mount();
    await flush();
    await type('/review src/App.tsx');
    await key('Enter');
    expect(startCalls()[0]).toMatchObject({ message: '/review src/App.tsx' });
  });

  it('/help lists the commands and /model opens AI settings', async () => {
    mount();
    await flush();
    await type('/help');
    await key('Enter');
    const help = screen.getByRole('dialog', { name: 'Commands' });
    expect(within(help).getByText('/compact')).toBeInTheDocument();
    expect(within(help).getByText('/review')).toBeInTheDocument();
    await type('/model');
    await key('Enter');
    expect(useApp.getState().route).toEqual({ name: 'settings', tab: 'ai' });
  });

  it('falls back to the built-in commands when the backend cannot list them', async () => {
    mount({
      list_commands: () => {
        throw { kind: 'test', message: 'not implemented' };
      },
    });
    await flush();
    await type('/');
    const names = within(screen.getByRole('listbox')).getAllByRole('option').map((o) => o.textContent ?? '');
    for (const n of ['clear', 'plan', 'build', 'effort', 'autopilot', 'undo', 'compact', 'model', 'help']) expect(names.some((x) => x.startsWith(`/${n}`))).toBe(true);
  });

  it('/compact compacts the conversation and /undo rewinds the last turn after confirming', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    mount();
    await flush();
    await type('/compact');
    await key('Enter');
    expect(calls.find((c) => c.cmd === 'compact_conversation')?.args).toEqual({ conversationId: 'c1' });
    await type('/undo');
    await key('Enter');
    expect(calls.find((c) => c.cmd === 'rewind_conversation')?.args).toEqual({ conversationId: 'c1', messageId: 'u1', revertFiles: true });
    expect(box().value).toBe('Add a sidebar');
  });

  it('parses and filters commands', () => {
    expect(slashQuery('/rev', 4)).toBe('rev');
    expect(slashQuery('/review x', 9)).toBeNull();
    expect(slashQuery('hi /x', 5)).toBeNull();
    expect(parseCommand('/effort  max ')).toEqual({ name: 'effort', arg: 'max' });
    expect(parseCommand('hello')).toBeNull();
    expect(filterCommands(BUILTIN_COMMANDS, 'co').map((c) => c.name)[0]).toBe('compact');
    expect(mentionQuery('fix @src/Ap', 11)).toEqual({ query: 'src/Ap', start: 4 });
    expect(mentionQuery('mail a@b', 8)).toBeNull();
    expect(fuzzyScore('src/components/Header.tsx', 'hdr')).toBeGreaterThan(0);
    expect(fuzzyScore('src/App.tsx', 'zzz')).toBe(-1);
  });
});

describe('@-mentions', () => {
  it('picks a file, inserts @path and pins it as a removable chip', async () => {
    mount();
    await flush();
    await type('Fix the @App');
    const list = screen.getByRole('listbox', { name: 'Include a project file' });
    expect(within(list).getAllByRole('option')[0]).toHaveTextContent('App.tsx');
    expect(within(list).queryByText('.env')).toBeNull(); // secret files are never offered
    await key('Enter');
    expect(box().value).toBe('Fix the @src/App.tsx ');
    expect(screen.getByRole('button', { name: 'Unpin src/App.tsx' })).toBeInTheDocument();
    await key('Enter');
    expect(startCalls()[0]).toMatchObject({ message: 'Fix the @src/App.tsx', pinned: ['src/App.tsx'] });
  });

  it('removes a pinned chip', async () => {
    mount();
    await flush();
    await type('@Head');
    await key('Tab');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Unpin src/components/Header.tsx' })));
    expect(screen.queryByRole('button', { name: 'Unpin src/components/Header.tsx' })).toBeNull();
  });
});

describe('effort and autopilot', () => {
  it('passes the chosen effort and autopilot on every request', async () => {
    mount();
    await flush();
    expect(screen.getByLabelText('Effort')).toHaveValue('max');
    expect(screen.getByRole('button', { name: 'Autopilot' })).toHaveAttribute('aria-pressed', 'false');
    await act(async () => void fireEvent.change(screen.getByLabelText('Effort'), { target: { value: 'balanced' } }));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Autopilot' })));
    await type('Add a footer');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Send' })));
    await flush();
    expect(startCalls()[0]).toMatchObject({ effort: 'balanced', autopilot: true });
    expect(localStorage.getItem('genius.autopilot.advanced')).toBe('on');
  });

  it('defaults Autopilot on in Simple mode and uses the saved default effort', async () => {
    useApp.setState({ settings: { ...settings, effort: 'fast' } });
    mount({}, 'simple');
    await flush();
    expect(screen.getByRole('button', { name: 'Autopilot' })).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByLabelText('Effort')).toHaveValue('fast');
  });
});

describe('live run', () => {
  it('streams thinking for the running task, collapsed with a timer, and expands', async () => {
    installIpc();
    const unlisten = await subscribeAll();
    useWorkspace.setState({ tasks: [running] });
    render(<AssistantPanel />);
    await flush();
    await act(async () => {
      await emit('genius://ai-thinking', { taskId: 't9', projectId: 'p1', text: 'The header overflows on small screens.\n' });
      await emit('genius://ai-thinking', { taskId: 't9', projectId: 'p1', text: 'I should check the nav first.' });
      await emit('genius://ai-thinking', { taskId: 'other', projectId: 'p2', text: 'not ours' });
    });
    await flush();
    expect(screen.getByText('I should check the nav first.')).toBeInTheDocument();
    const head = screen.getByRole('button', { name: /Thinking… \d+s/ });
    expect(head).toHaveAttribute('aria-expanded', 'false');
    await act(async () => void fireEvent.click(head));
    expect(screen.getByLabelText('Reasoning')).toHaveTextContent('The header overflows on small screens.');
    expect(useWorkspace.getState().thinking.other).toBeUndefined();
    unlisten();
  });

  it('caps stored thinking', () => {
    useWorkspace.getState().onAiThinking('t1', 'x'.repeat(50_000));
    useWorkspace.getState().onAiThinking('t1', 'END');
    const t = useWorkspace.getState().thinking.t1;
    expect(t.text.length).toBe(40_000);
    expect(t.text.endsWith('END')).toBe(true);
  });

  it('steers the running agent with a queued bubble that turns delivered', async () => {
    installIpc();
    const unlisten = await subscribeAll();
    useWorkspace.setState({ tasks: [running] });
    render(<AssistantPanel />);
    await flush();
    expect(box()).not.toBeDisabled();
    await type('Also make the buttons blue');
    await key('Enter');
    expect(calls.find((c) => c.cmd === 'steer_agent')?.args).toEqual({ taskId: 't9', message: 'Also make the buttons blue' });
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
    expect(screen.getByText('Queued — Genius will read this after its current step')).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: /Stop/ }).length).toBeGreaterThan(0);
    await act(async () => void (await emit('genius://agent-steered', { taskId: 't9', projectId: 'p1', message: 'Also make the buttons blue' })));
    await flush();
    expect(screen.getByText('Delivered — Genius read this')).toBeInTheDocument();
    // The task ends: delivered messages are not sent again.
    await act(async () => useWorkspace.setState({ tasks: [] }));
    await flush();
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
    unlisten();
  });

  it('sends undelivered queued text as a new message once the task ends', async () => {
    useWorkspace.setState({ tasks: [running] });
    mount({
      steer_agent: () => {
        throw { kind: 'conflict', message: 'task finished' };
      },
    });
    await flush();
    await type('And add a footer');
    await key('Enter');
    expect(screen.getByText('Will be sent as a new message when Genius finishes')).toBeInTheDocument();
    await act(async () => useWorkspace.setState({ tasks: [] }));
    await flush();
    expect(startCalls()).toHaveLength(1);
    expect(startCalls()[0]).toMatchObject({ message: 'And add a footer' });
    expect(useWorkspace.getState().queue).toEqual([]);
  });

  it('labels new tools, groups one turn and folds retried failures', () => {
    const steps = [
      step({ id: 'a', tool: 'write_file', args: { path: 'src/x.tsx' }, turn: 1 }),
      step({ id: 'b', tool: 'edit_file', args: { path: 'src/y.tsx' }, result: { added: 3, removed: 1 }, turn: 1 }),
      step({ id: 'c', tool: 'edit_file', args: { path: 'src/z.tsx' }, ok: false, summary: 'old text not found', turn: 2 }),
      step({ id: 'd', tool: 'edit_file', args: { path: 'src/z.tsx' }, turn: 3 }),
      step({ id: 'e', tool: 'preview_errors', turn: 4 }),
      step({ id: 'f', tool: 'delegate', args: { role: 'explorer', task: 'find the router' }, turn: 5 }),
      step({ id: 'g', tool: 'todo', turn: 5 }),
    ];
    const groups = groupSteps(steps);
    expect(groups.map((g) => g.items.map((i) => i.step.id))).toEqual([['a', 'b'], ['d'], ['e'], ['f']]);
    expect(groups[1].items[0].retries).toBe(1);
    expect(groups.flatMap((g) => g.items.map((i) => stepLabel(i.step)))).toEqual([
      'Wrote src/x.tsx',
      'Edited src/y.tsx (+3 −1)',
      'Edited src/z.tsx',
      'Checked the preview for errors',
      'Asked the Explorer agent: find the router',
    ]);
  });

  it('renders grouped steps in the run card', async () => {
    useWorkspace.setState({ tasks: [running], agentSteps: { t9: [step({ id: 'a', tool: 'write_file', args: { path: 'src/x.tsx' }, turn: 1 }), step({ id: 'b', tool: 'write_file', args: { path: 'src/y.tsx' }, turn: 1 })] } });
    mount();
    await flush();
    expect(screen.getByText('2 actions at once')).toBeInTheDocument();
    expect(screen.getByText('Wrote src/y.tsx')).toBeInTheDocument();
  });
});

describe('message actions', () => {
  it('edit and resend rewinds (reverting files) and then starts the agent with the new text', async () => {
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    mount();
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Edit message' })));
    fireEvent.change(screen.getByLabelText('Edited message'), { target: { value: 'Add a collapsible sidebar' } });
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Save & resend/ })));
    await flush();
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining('undone'));
    const order = calls.filter((c) => c.cmd === 'rewind_conversation' || c.cmd === 'start_agent').map((c) => c.cmd);
    expect(order).toEqual(['rewind_conversation', 'start_agent']);
    expect(calls.find((c) => c.cmd === 'rewind_conversation')?.args).toEqual({ conversationId: 'c1', messageId: 'u1', revertFiles: true });
    expect(startCalls()[0]).toMatchObject({ message: 'Add a collapsible sidebar' });
  });

  it('does nothing when the edit is not confirmed', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    mount();
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Edit message' })));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Save & resend/ })));
    expect(calls.some((c) => c.cmd === 'rewind_conversation' || c.cmd === 'start_agent')).toBe(false);
  });

  it('retry rewinds to the user message before the answer and resends it; rewind-to-here only rewinds', async () => {
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    mount();
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Retry' })));
    await flush();
    expect(calls.find((c) => c.cmd === 'rewind_conversation')?.args).toMatchObject({ messageId: 'u1', revertFiles: true });
    expect(startCalls()[0]).toMatchObject({ message: 'Add a sidebar' });
    calls.length = 0;
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Rewind to here' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'rewind_conversation')).toBe(true);
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
    expect(calls.some((c) => c.cmd === 'list_files')).toBe(true); // files refreshed after rewinding
  });

  it('disables actions while a task runs', async () => {
    useWorkspace.setState({ tasks: [running] });
    mount();
    await flush();
    expect(screen.getByRole('button', { name: 'Edit message' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Retry' })).toBeDisabled();
  });
});

describe('reports', () => {
  const answer = (meta: Record<string, unknown>): MessageRow => ({ id: 'a9', projectId: 'p1', taskId: 't0', role: 'assistant', content: 'Done.', createdAt: '2026-10-01T00:00:12Z', conversationId: 'c1', kind: 'answer', meta });

  it('shows model, tokens, duration and autopilot under a finished answer', async () => {
    const r = report({ model: 'meta/llama-3.3-70b-instruct', usage: { promptTokens: 10000, completionTokens: 2345, totalTokens: 12345, calls: 3, models: ['meta/llama-3.3-70b-instruct'] }, autopilot: { applied: ['src/A.tsx', 'src/B.tsx'], checks: [{ check: 'typecheck', ok: true }] } });
    mount({ conversation_messages: () => [messages[0], answer({ report: r })] });
    await flush();
    expect(screen.getByLabelText('Answer details')).toHaveTextContent('llama-3.3-70b-instruct · 12.3k tokens · 12s · Autopilot applied 2 files, typecheck ✓');
  });

  it('formats the footer', () => {
    expect(reportFooter(report({ usage: { promptTokens: 1, completionTokens: 1, totalTokens: 950, calls: 1, models: [] }, durationMs: 95_000 }))).toEqual(['950 tokens', '1m 35s']);
    expect(reportFooter(undefined)).toEqual([]);
  });

  it('never renders empty report bullets', () => {
    const r = report({ filesRead: ['', 'src/App.tsx'], failedSteps: ['', ' '], notVerified: ['', '   ', 'No build ran.'], warnings: ['', '\n'], checks: [{ check: '', ok: true, status: '', exitCode: null, summary: '' }], error: '' });
    const { container } = render(<Report report={r} />);
    const items = Array.from(container.querySelectorAll('li'));
    expect(items.length).toBe(4);
    for (const li of items) expect(li.textContent?.trim()).toMatch(/\w/);
    expect(container.textContent).toContain('Files read: src/App.tsx');
  });

  it('never renders empty markdown bullets', () => {
    const { container } = render(<Markdown text={'Done:\n\n1. \n**Header** fixed\n2. \n3. Footer\n\n- \n- \n* real item\n- '} />);
    const items = Array.from(container.querySelectorAll('li'));
    expect(items.map((li) => li.textContent)).toEqual(['Header fixed', 'Footer', 'real item']);
  });
});
