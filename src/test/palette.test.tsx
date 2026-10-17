// Command palette: opening via keyboard, filtering, keyboard navigation, running actions,
// and commands contributed by other code through the registry.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { calls, clearMocks, installIpc, project, settings } from './ipc';
import App from '../App';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';
import { fuzzyMatch, rankCommands, registerCommands, registeredCommands, subscribeCommands, unregisterCommands, type PaletteCommand } from '../lib/commands';

vi.mock('react-resizable-panels', async () => {
  const { forwardRef, useImperativeHandle, useState, createElement } = await import('react');
  const Panel = forwardRef<unknown, { children?: React.ReactNode }>(function Panel({ children }, ref) {
    const [collapsed, setCollapsed] = useState(false);
    useImperativeHandle(ref, () => ({ isCollapsed: () => collapsed, expand: () => setCollapsed(false), collapse: () => setCollapsed(true) }), [collapsed]);
    return createElement('div', { 'data-panel': '' }, collapsed ? null : children);
  });
  return { Panel, PanelGroup: ({ children }: { children?: React.ReactNode }) => createElement('div', null, children), PanelResizeHandle: () => null };
});

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const press = (key: string, opts: Partial<KeyboardEventInit> = {}) => act(async () => void fireEvent.keyDown(window, { key, ...opts }));
const palette = () => screen.queryByRole('dialog', { name: 'Command palette' });
const search = () => screen.getByRole('combobox', { name: 'Search commands' });
const type = async (text: string) => {
  await act(async () => void fireEvent.change(search(), { target: { value: text } }));
};
const options = () => within(within(palette()!).getByRole('listbox')).queryAllByRole('option');
const activeOption = () => document.getElementById(search().getAttribute('aria-activedescendant') ?? '');

describe('command palette', () => {
  beforeEach(() => {
    installIpc({ list_projects: () => [project, { ...project, id: 'p2', name: 'Recipe Book', description: 'Recipes' }] });
    useApp.setState({ route: { name: 'home' }, settings });
    useWorkspace.getState().close();
    localStorage.removeItem('genius.notifications');
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('opens with Ctrl+K and Ctrl+Shift+P, closes with Esc', async () => {
    render(<App />);
    await flush();
    expect(palette()).toBeNull();
    await press('k', { ctrlKey: true });
    expect(palette()).toBeInTheDocument();
    expect(document.activeElement).toBe(search());
    await act(async () => void fireEvent.keyDown(search(), { key: 'Escape' }));
    expect(palette()).toBeNull();
    await press('P', { ctrlKey: true, shiftKey: true });
    expect(palette()).toBeInTheDocument();
    await press('k', { metaKey: true }); // the shortcut toggles
    expect(palette()).toBeNull();
    // The rail button opens it too.
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Command palette' })));
    expect(palette()).toBeInTheDocument();
  });

  it('lists grouped commands and filters them fuzzily', async () => {
    render(<App />);
    await flush();
    await press('k', { ctrlKey: true });
    await flush();
    const groups = within(palette()!).getAllByRole('group').map((g) => g.getAttribute('aria-labelledby'));
    expect(groups).toEqual(expect.arrayContaining(['palette-g-navigation', 'palette-g-recent-projects', 'palette-g-preferences']));
    const titles = options().map((o) => o.textContent ?? '');
    for (const t of [/^Go Home$/, /^Open project Dashboard\s+A dashboard$/, /^Switch to Simple mode\s+Chat and preview only$/, /^Check for updates$/]) expect(titles.some((x) => t.test(x)), String(t)).toBe(true);
    // Shortcut hints are shown.
    expect(within(options().find((o) => o.textContent?.startsWith('Settings'))!).getByText('Ctrl+,')).toBeInTheDocument();

    await type('recipe');
    expect(options()[0]).toHaveTextContent('Open project Recipe Book');
    await type('tgl thm');
    expect(options()[0]).toHaveTextContent('Toggle theme');
    await type('zzzzqq');
    expect(options()).toHaveLength(0);
    expect(screen.getByText(/No commands match/)).toBeInTheDocument();
  });

  it('navigates with the arrow keys and runs the active command with Enter', async () => {
    render(<App />);
    await flush();
    await press('k', { ctrlKey: true });
    await flush();
    await type('open project');
    expect(activeOption()).toHaveTextContent('Open project Dashboard');
    expect(activeOption()).toHaveAttribute('aria-selected', 'true');
    await act(async () => void fireEvent.keyDown(search(), { key: 'ArrowDown' }));
    expect(activeOption()).toHaveTextContent('Open project Recipe Book');
    await act(async () => void fireEvent.keyDown(search(), { key: 'ArrowUp' }));
    await act(async () => void fireEvent.keyDown(search(), { key: 'ArrowUp' })); // wraps around
    expect(activeOption()).toHaveTextContent('Open project Recipe Book');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(palette()).toBeNull();
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p2' });
  });

  it('runs preferences: settings, interface mode and notifications', async () => {
    render(<App />);
    await flush();
    await press('k', { ctrlKey: true });
    await type('settings');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(useApp.getState().route).toEqual({ name: 'settings' });

    await press('k', { ctrlKey: true });
    await type('simple mode');
    await act(async () => void fireEvent.click(options()[0]));
    await flush();
    expect(calls.some((c) => c.cmd === 'save_settings' && (c.args.settings as { uiMode: string }).uiMode === 'simple')).toBe(true);

    await press('k', { ctrlKey: true });
    await type('notifications');
    expect(options()[0]).toHaveTextContent('Turn notifications off');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(localStorage.getItem('genius.notifications')).toBe('off');
    await press('k', { ctrlKey: true });
    await type('notifications');
    expect(options()[0]).toHaveTextContent('Turn notifications on');
  });

  it('offers workspace actions and project files while a project is open', async () => {
    render(<App />);
    await flush();
    await act(async () => useApp.getState().navigate({ name: 'project', id: 'p1' }));
    await flush(60);
    await press('k', { ctrlKey: true });
    await flush();
    const titles = options().map((o) => o.textContent ?? '');
    expect(titles.some((t) => t.startsWith('Show Preview'))).toBe(true);
    expect(titles.some((t) => t.startsWith('Open Terminal'))).toBe(true);
    expect(titles.some((t) => t.startsWith('Open Git'))).toBe(true);
    // Files are only listed when searching.
    expect(titles.some((t) => t.startsWith('src/App.tsx'))).toBe(false);
    await type('app.tsx');
    expect(options()[0]).toHaveTextContent('src/App.tsx');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(useWorkspace.getState().active).toBe('file:src/App.tsx');

    await press('k', { ctrlKey: true });
    await type('terminal');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(useWorkspace.getState().bottomTab).toBe('terminal');

    // Leaving the workspace removes its commands.
    await act(async () => useApp.getState().navigate({ name: 'home' }));
    await flush();
    expect(registeredCommands().some((c) => c.id === 'ws.terminal')).toBe(false);
  });

  it('shows commands registered by other code', async () => {
    const run = vi.fn();
    render(<App />);
    await flush();
    await press('k', { ctrlKey: true });
    await act(async () => void registerCommands('test-panel', [{ id: 'test.hello', title: 'Say hello', group: 'Testing', run }]));
    await type('say hello');
    expect(options()[0]).toHaveTextContent('Say hello');
    await act(async () => void fireEvent.keyDown(search(), { key: 'Enter' }));
    await flush();
    expect(run).toHaveBeenCalledOnce();
    unregisterCommands('test-panel');
  });
});

describe('command registry', () => {
  const cmd = (id: string, title = id): PaletteCommand => ({ id, title, run: () => {} });
  afterEach(() => {
    unregisterCommands('a');
    unregisterCommands('b');
  });

  it('registers, replaces and unregisters per owner and notifies subscribers', () => {
    const seen: number[] = [];
    const unsubscribe = subscribeCommands(() => seen.push(registeredCommands().length));
    const offA = registerCommands('a', [cmd('a1'), cmd('a2')]);
    registerCommands('b', [cmd('b1')]);
    expect(registeredCommands().map((c) => c.id)).toEqual(['a1', 'a2', 'b1']);
    const snapshot = registeredCommands();
    expect(registeredCommands()).toBe(snapshot); // stable between changes (useSyncExternalStore)
    registerCommands('a', [cmd('a3')]); // replaces a's commands
    expect(registeredCommands().map((c) => c.id)).toEqual(['a3', 'b1']);
    offA(); // stale cleanup: a newer registration under the same owner stays
    expect(registeredCommands().map((c) => c.id)).toEqual(['a3', 'b1']);
    unregisterCommands('a');
    expect(registeredCommands().map((c) => c.id)).toEqual(['b1']);
    unregisterCommands('missing'); // no-op, no notification
    unsubscribe();
    registerCommands('a', [cmd('a4')]);
    expect(seen).toEqual([2, 3, 2, 1]);
  });

  it('ranks fuzzy matches: prefixes and word starts first', () => {
    expect(fuzzyMatch('xyz', 'Go Home')).toBeNull();
    expect(fuzzyMatch('gh', 'Go Home')?.indices).toEqual([0, 3]);
    const list = [cmd('1', 'Show Preview'), cmd('2', 'Open project Snake'), cmd('3', 'Settings'), { ...cmd('4', 'Open Git'), keywords: ['source control'] }];
    expect(rankCommands(list, 'set')[0].command.title).toBe('Settings');
    expect(rankCommands(list, 'sp')[0].command.title).toBe('Show Preview');
    expect(rankCommands(list, 'source').map((r) => r.command.title)).toEqual(['Open Git']); // keyword match
    expect(rankCommands([...list, { ...cmd('5', 'hidden file'), searchOnly: true }], '').map((r) => r.command.id)).toEqual(['1', '2', '3', '4']);
  });
});
