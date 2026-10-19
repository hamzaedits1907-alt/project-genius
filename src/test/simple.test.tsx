// Simple mode, Genius's chat helpers and the Markdown renderer (mocked backend, no real model).
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, messages, settings } from './ipc';
import App from '../App';
import AssistantPanel from '../views/workspace/AssistantPanel';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';
import { project } from './ipc';
import { parseBlocks } from '../components/Markdown';
import { askOf, latestNote, latestTodo } from '../lib/agent';
import { nameFromIdea, recommendModel, recommendModels } from '../views/Home';
import type { AgentStep } from '../lib/types';

vi.mock('react-resizable-panels', async () => {
  const { forwardRef, createElement } = await import('react');
  const Panel = forwardRef<unknown, { children?: React.ReactNode }>(function Panel({ children }, _ref) {
    return createElement('div', { 'data-panel': '' }, children);
  });
  const PanelGroup = ({ children }: { children?: React.ReactNode }) => createElement('div', null, children);
  return { Panel, PanelGroup, PanelResizeHandle: () => null };
});

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));

const step = (over: Partial<AgentStep>): AgentStep => ({ id: Math.random().toString(), projectId: 'p1', taskId: 't1', conversationId: 'c1', agent: 'main', seq: 1, tool: 'read_file', args: {}, ok: true, summary: '', result: {}, startedAt: '', endedAt: '', ...over });

describe('simple mode', () => {
  beforeEach(() => {
    installIpc({ get_settings: () => ({ ...settings, uiMode: 'simple' }) });
    useApp.setState({ route: { name: 'home' } });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('shows chat and the stage, switches views and modes', async () => {
    render(<StrictMode><App /></StrictMode>);
    await flush();
    await act(async () => useApp.getState().navigate({ name: 'project', id: 'p1' }));
    await flush(60);
    // The IDE rail is hidden in simple mode.
    expect(screen.queryByRole('button', { name: 'Source control' })).toBeNull();
    expect(screen.getByLabelText('Change request')).toBeInTheDocument();
    for (const name of [/Code/, /Preview/]) {
      await act(async () => void fireEvent.click(screen.getAllByRole('tab').find((t) => name.test(t.textContent ?? ''))!));
      await flush();
    }
    await act(async () => void fireEvent.click(screen.getByRole('radio', { name: 'Advanced' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'save_settings' && (c.args.settings as { uiMode: string }).uiMode === 'advanced')).toBe(true);
    expect(screen.getByRole('button', { name: 'Source control' })).toBeInTheDocument();
  });

  it('home prompt creates a project, plans it and opens it', async () => {
    installIpc({ get_settings: () => ({ ...settings, uiMode: 'simple' }), key_status: () => ({ stored: true, hint: '…abcd', backend: 'memory', error: null }),
      create_project: (_c, a) => ({ ...project, id: 'p2', name: a.name, description: a.description }),
      start_plan: () => ({ id: 't2', projectId: 'p2', kind: 'plan', title: 'Plan', startedAt: '2026-10-01T00:00:00Z' }),
    });
    render(<App />);
    await flush();
    fireEvent.change(screen.getByLabelText('Describe your app'), { target: { value: 'A landing page for a coffee shop with a menu' } });
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Build it' })));
    await flush();
    expect(calls.find((c) => c.cmd === 'create_project')?.args).toEqual({ name: 'Landing Coffee Shop', description: 'A landing page for a coffee shop with a menu' });
    expect(calls.some((c) => c.cmd === 'start_plan')).toBe(true);
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p2' });
    expect(localStorage.getItem('genius.autoBuild')).toContain('p2');
  });
});

describe('Genius asks questions', () => {
  beforeEach(() => {
    useApp.setState({ settings });
    useWorkspace.setState({ projectId: 'p1', project, tasks: [], activity: [], agentSteps: {} });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('renders suggested answers and sends the chosen one', async () => {
    const ask = { id: 'a2', projectId: 'p1', taskId: 't1', role: 'assistant' as const, content: 'Happy to build it!', createdAt: '2026-10-01T00:00:02Z', conversationId: 'c1', kind: 'question', meta: { ask: { question: 'Which style?', options: ['Minimal', 'Bold'], multi: false } } };
    installIpc({ conversation_messages: () => [...messages, ask] });
    render(<AssistantPanel variant="simple" />);
    await flush();
    expect(screen.getByText('Which style?')).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Bold' })));
    await flush();
    expect((calls.find((c) => c.cmd === 'start_agent')?.args.request as { message: string }).message).toBe('Bold');
  });
});

describe('helpers', () => {
  it('parses markdown blocks', () => {
    const b = parseBlocks('# Title\n\nHello **world**\n\n- a\n- b\n\n```ts\nconst x = 1;\n```\n1. one\n2. two');
    expect(b.map((x) => x.kind)).toEqual(['h', 'p', 'ul', 'code', 'ol']);
    expect(b[3]).toEqual({ kind: 'code', lang: 'ts', body: 'const x = 1;' });
  });

  it('reads the latest checklist, narration and question', () => {
    const steps = [
      step({ tool: 'todo', args: { items: [{ text: 'Design', status: 'active' }] } }),
      step({ tool: 'read_file', result: { note: 'Checking the header first.' } }),
      step({ tool: 'todo', args: { items: [{ text: 'Design', status: 'done' }, { text: 'Build', status: 'weird' }] } }),
    ];
    expect(latestTodo(steps)).toEqual([{ text: 'Design', status: 'done' }, { text: 'Build', status: 'pending' }]);
    expect(latestNote(steps)).toBe('Checking the header first.');
    expect(askOf({ ask: { question: 'Q?', options: ['a', 1, 'b'] } })).toEqual({ question: 'Q?', options: ['a', 'b'], multi: false });
    expect(askOf({})).toBeNull();
  });

  it('recommends a strong chat/code model', () => {
    const m = (id: string, inferredCategory = 'chat') => ({ id, owner: null, created: null, inferredCategory });
    expect(recommendModel([m('nvidia/nv-embed', 'embedding'), m('meta/llama-3.1-8b-instruct'), m('meta/llama-3.3-70b-instruct')])).toBe('meta/llama-3.3-70b-instruct');
    expect(recommendModel([m('x/small'), m('qwen/qwen3-coder-480b-a35b-instruct', 'code')])).toBe('qwen/qwen3-coder-480b-a35b-instruct');
    expect(recommendModel([m('nvidia/nv-embed', 'embedding')])).toBeNull();
    // candidates keep their preference order and every other chat model follows (the caller probes them in turn)
    expect(recommendModels([m('x/small'), m('nvidia/nemotron-3-super-120b-a12b'), m('meta/llama-3.3-70b-instruct'), m('nvidia/nv-embed', 'embedding')])).toEqual(['nvidia/nemotron-3-super-120b-a12b', 'meta/llama-3.3-70b-instruct', 'x/small']);
  });

  it('derives a project name from an idea', () => {
    expect(nameFromIdea('Build me a portfolio website for a photographer')).toBe('Portfolio Photographer');
    expect(nameFromIdea('!!!')).toBe('My App');
  });
});

describe('updates', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('offers the new version, installs it, or skips it', async () => {
    const { default: UpdateDialog } = await import('../views/UpdateDialog');
    installIpc({ install_update: () => 'Genius was updated and will restart.' });
    useApp.setState({
      update: { currentVersion: '0.2.0', latestVersion: '0.2.1', available: true, notes: '- Skip versions', publishedAt: null, releaseUrl: null, asset: { name: 'Genius_0.2.1_amd64.AppImage', size: 10, url: 'https://github.com/x', sha256Url: 'https://github.com/x.sha256' }, detail: null },
      skippedVersion: null,
    });
    const onClose = vi.fn();
    render(<UpdateDialog onClose={onClose} />);
    expect(screen.getByText('Genius 0.2.1 is available')).toBeInTheDocument();
    expect(screen.getByText('Skip versions')).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Skip this version' })));
    expect(useApp.getState().skippedVersion).toBe('0.2.1');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Update now/ })));
    expect(calls.some((c) => c.cmd === 'install_update')).toBe(true);
  });
});
