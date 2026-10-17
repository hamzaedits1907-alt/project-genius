// Mounts the real App (StrictMode, like main.tsx) against a mocked Tauri IPC and drives every
// view through mount → interaction → unmount. Any effect whose cleanup is not a function makes
// React log "must not return anything besides a function" and later throw
// "destroy is not a function"; both fail this test.
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { emit } from '@tauri-apps/api/event';
import { clearMocks, installIpc } from './ipc';
import App from '../App';

// jsdom has no layout engine, so react-resizable-panels cannot size panels here. Replace it
// with a pass-through stand-in exposing the same imperative handle; the real library is
// exercised in the real webview by tests/ui/walk_views.py and tests/ui/e2e_ui.py.
vi.mock('react-resizable-panels', async () => {
  const { forwardRef, useImperativeHandle, useState, createElement } = await import('react');
  const Panel = forwardRef<unknown, { children?: React.ReactNode }>(function Panel({ children }, ref) {
    const [collapsed, setCollapsed] = useState(false);
    useImperativeHandle(ref, () => ({ isCollapsed: () => collapsed, expand: () => setCollapsed(false), collapse: () => setCollapsed(true) }), [collapsed]);
    return createElement('div', { 'data-panel': '' }, collapsed ? null : children);
  });
  const PanelGroup = ({ children }: { children?: React.ReactNode }) => createElement('div', null, children);
  const PanelResizeHandle = () => null;
  return { Panel, PanelGroup, PanelResizeHandle };
});
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const click = async (el: Element) => {
  await act(async () => void fireEvent.click(el));
  await flush();
};

let errors: string[];
function assertHealthy(where: string) {
  const boundary = document.querySelector('.empty h3')?.textContent?.includes('Something went wrong');
  const bad = errors.filter((e) => /destroy is not a function|must not return anything besides a function|is not a function/.test(e));
  expect({ where, boundary: boundary ? document.querySelector('.empty')?.textContent : null, bad }).toEqual({ where, boundary: null, bad: [] });
}

describe('views mount, navigate and unmount', () => {
  beforeEach(() => {
    errors = [];
    vi.spyOn(console, 'error').mockImplementation((...a: unknown[]) => void errors.push(a.map((x) => (x instanceof Error ? `${x.message}\n${x.stack}` : String(x))).join(' ')));
    installIpc();
    useApp.setState({ route: { name: 'home' } });
    useWorkspace.setState({ layout: 'simple', projectId: null });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('walks every top-level view and settings tab', async () => {
    const r = render(<StrictMode><App /></StrictMode>);
    await flush();
    for (const route of [{ name: 'create' }, { name: 'home' }, { name: 'settings' }] as const) {
      await act(async () => useApp.getState().navigate(route));
      await flush();
      assertHealthy(route.name);
    }
    for (const label of ['GitHub', 'Integrations', 'Approvals', 'Workspace', 'Appearance', 'AI provider']) {
      const btn = screen.getAllByRole('button').find((b) => b.textContent?.trim().startsWith(label));
      if (btn) await click(btn);
      assertHealthy(`settings:${label}`);
    }
    r.unmount();
    assertHealthy('unmount');
  });

  it('walks the workspace panels, tabs, dialogs and live events', async () => {
    const r = render(<StrictMode><App /></StrictMode>);
    await flush();
    await act(async () => useApp.getState().navigate({ name: 'project', id: 'p1' }));
    await flush(60);
    assertHealthy('workspace');
    // ---- simple layout (the default): conversation + preview + changes/activity
    expect(useWorkspace.getState().layout).toBe('simple');
    expect(screen.getByText('Ask Genius')).toBeTruthy();
    for (const name of [/Activity/, /Changes/]) {
      await click(screen.getAllByRole('tab').find((t) => name.test(t.textContent ?? ''))!);
      assertHealthy(`simple ${String(name)}`);
    }
    await click(screen.getAllByRole('button').find((b) => b.textContent?.trim() === 'Preview')!);
    assertHealthy('preview start');
    const base = { projectId: 'p1', port: 5199, runId: 'r1', modulesChecked: 0, inspector: true, updatedAt: '' };
    await act(async () => {
      await emit('genius://preview', { projectId: 'p1', status: 'starting', state: { ...base, status: 'installing', message: 'Installing dependencies…', url: null, problem: null, generation: 1 } });
    });
    await flush();
    expect(screen.getByText('Install dependencies')).toBeTruthy();
    await act(async () => {
      await emit('genius://preview', { projectId: 'p1', status: 'failed', state: { ...base, status: 'failed', message: 'The development server is running, but your app has an error.', url: 'http://127.0.0.1:5199/', generation: 1, problem: { stage: 'compile', summary: 'Your app could not be compiled: Failed to resolve import "uuid"', details: 'Plugin: vite:import-analysis', file: 'src/App.tsx', runId: 'r1', fixable: true } } });
      // an event of an older server generation must not overwrite the newer state
      await emit('genius://preview', { projectId: 'p1', status: 'ready', state: { ...base, status: 'ready', message: 'old', url: 'http://127.0.0.1:5000/', generation: 0, problem: null } });
      // and another project's server is never shown here
      await emit('genius://preview', { projectId: 'other', status: 'ready', state: { ...base, projectId: 'other', status: 'ready', message: 'x', url: 'http://127.0.0.1:6000/', generation: 9, problem: null } });
    });
    await flush();
    expect(useWorkspace.getState().preview.status).toBe('failed');
    expect(screen.getByText('Your website has an error')).toBeTruthy();
    await click(screen.getAllByRole('button').find((b) => b.textContent?.trim() === 'View details')!);
    expect(screen.getByText(/vite:import-analysis/)).toBeTruthy();
    await click(screen.getAllByRole('button').filter((b) => b.textContent?.trim() === 'Fix automatically')[0]);
    assertHealthy('fix automatically');
    await click(screen.getAllByRole('button').find((b) => b.textContent?.trim().startsWith('More'))!);
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(expect.arrayContaining([expect.stringContaining('Install dependencies'), expect.stringContaining('Publish to GitHub')]));
    await act(async () => void fireEvent.click(document.querySelector('.menu-backdrop')!));
    // ---- advanced layout: every tool stays reachable
    await click(screen.getAllByRole('button').find((b) => b.textContent?.trim() === 'Show code & tools')!);
    assertHealthy('advanced');
    for (const label of ['Search', 'Source control', 'History & recovery', 'Project memory', 'Imported files', 'Integrations', 'Files']) {
      await click(screen.getByRole('button', { name: label }));
      assertHealthy(label);
    }
    for (const name of [/Terminal/, /Readiness/, /Activity/, /Preview/, /Assistant/]) {
      await click(screen.getAllByRole('tab').find((t) => name.test(t.textContent ?? ''))!);
      assertHealthy(String(name));
    }
    // open a file in the editor, then live backend events
    await click(screen.getAllByText('App.tsx')[0]);
    assertHealthy('open file');
    await act(async () => {
      await emit('genius://activity', { id: 'e1', projectId: 'p1', taskId: 't1', kind: 'task', status: 'running', title: 'Change', detail: {}, ts: new Date().toISOString(), updatedTs: new Date().toISOString() });
      await emit('genius://ai-delta', { taskId: 't1', projectId: 'p1', text: 'hello' });
      await emit('genius://file-stream', { type: 'fileStart', path: 'src/New.tsx', taskId: 't1', projectId: 'p1' });
      await emit('genius://file-stream', { type: 'fileDelta', path: 'src/New.tsx', text: 'export {}', taskId: 't1', projectId: 'p1' });
      await emit('genius://preview', { projectId: 'p1', status: 'ready', url: 'http://127.0.0.1:5199/' });
      await emit('genius://fs-changed', { projectId: 'p1', paths: ['src/App.tsx'] });
    });
    await flush(60);
    assertHealthy('events');
    await click(screen.getAllByRole('button').find((b) => b.textContent?.trim().startsWith('More'))!);
    await click(screen.getAllByRole('menuitem').find((m) => m.textContent?.includes('Export source'))!);
    assertHealthy('Export source');
    await act(async () => void fireEvent.keyDown(window, { key: 'Escape' }));
    await act(async () => void fireEvent.keyDown(window, { key: 'p', ctrlKey: true }));
    await flush();
    assertHealthy('quick open');
    await act(async () => void fireEvent.keyDown(window, { key: 'Escape' }));
    await act(async () => useApp.getState().navigate({ name: 'home' }));
    await flush();
    assertHealthy('leave workspace');
    r.unmount();
    assertHealthy('unmount');
    expect(useWorkspace.getState()).toBeTruthy();
  });
});
