// Runtime error capture: the injected preview script (run for real in jsdom), the helpers that
// summarise errors, the preview problem bar with "Fix it", and the website deploy section.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import inspectorRs from '../../crates/genius-core/src/inspector.rs?raw';
import { calls, clearMocks, installIpc, project, settings } from './ipc';
import { errorLocation, fixMessage, parseRuntimeReport, problemSummary, renderRuntimeErrors } from '../lib/runtime';
import type { DeployStatus, PreviewRuntimeError } from '../lib/types';
import PreviewPanel from '../views/workspace/PreviewPanel';
import { DeploySection } from '../views/workspace/PublishDialog';
import App from '../App';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';

vi.mock('react-resizable-panels', async () => {
  const { forwardRef, createElement } = await import('react');
  const Panel = forwardRef<unknown, { children?: React.ReactNode }>(function Panel({ children }, _ref) {
    return createElement('div', { 'data-panel': '' }, children);
  });
  const PanelGroup = ({ children }: { children?: React.ReactNode }) => createElement('div', null, children);
  return { Panel, PanelGroup, PanelResizeHandle: () => null };
});

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));

const crash: PreviewRuntimeError = {
  kind: 'error',
  message: "TypeError: Cannot read properties of undefined (reading 'map')",
  source: 'node_modules/.vite/deps/react-dom_client.js',
  line: 5000,
  column: 1,
  stack: "TypeError: Cannot read properties of undefined (reading 'map')\n    at WidgetGrid (src/components/WidgetGrid.tsx:42:13)\n    at renderWithHooks (node_modules/.vite/deps/react-dom_client.js:5000:1)",
  page: '/',
  at: '2026-10-01T00:00:00Z',
  count: 2,
};
const blank: PreviewRuntimeError = { kind: 'blank', message: 'The page shows nothing: #root is empty', page: '/', at: '2026-10-01T00:00:00Z', count: 1 };
const warn: PreviewRuntimeError = { kind: 'console', message: 'Each child in a list should have a unique "key" prop.', source: 'console.warn', page: '/', at: '2026-10-01T00:00:00Z', count: 1 };

describe('runtime error helpers', () => {
  it('summarises the worst problem with a project-code location', () => {
    expect(problemSummary([warn])).toBeNull();
    expect(problemSummary([warn, blank, crash])).toEqual({ severity: 'crash', title: 'Your app crashed', detail: crash.message, location: 'WidgetGrid.tsx:42', count: 3 });
    expect(problemSummary([blank])?.title).toBe('Your app shows a blank page');
    expect(errorLocation({ ...crash, stack: null })).toBe('react-dom_client.js:5000');
    expect(errorLocation(warn)).toBeNull();
  });

  it('renders compact text for Genius and validates reports from the page', () => {
    const text = renderRuntimeErrors([crash, blank]);
    expect(text).toContain("[1] Uncaught error (×2) at node_modules/.vite/deps/react-dom_client.js:5000:1 on page /\n  TypeError: Cannot read");
    expect(text).toContain('    at WidgetGrid (src/components/WidgetGrid.tsx:42:13)');
    expect(text.match(/TypeError/g)).toHaveLength(1);
    expect(text).toContain('[2] Blank page on page /');
    expect(fixMessage([blank]).startsWith('The preview has runtime errors. Find the cause and fix it:\n[1] Blank page')).toBe(true);
    const parsed = parseRuntimeReport({ errors: [{ kind: 'error', message: 'x', line: 3, count: 0 }, { kind: 'evil', message: 'y' }, { kind: 'error' }, null, { kind: 'console', message: 1 }] });
    expect(parsed).toHaveLength(1);
    expect(parsed[0]).toMatchObject({ kind: 'error', message: 'x', line: 3, count: 1, page: '/' });
    expect(parseRuntimeReport({ errors: 'nope' })).toEqual([]);
  });
});

describe('injected preview script', () => {
  const script = /pub const CLIENT_SCRIPT: &str = r#"([\s\S]*?)"#;/.exec(inspectorRs)![1];
  const posted: { type: string; errors?: PreviewRuntimeError[] }[] = [];
  let origError: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers();
    posted.length = 0;
    origError = vi.fn();
    console.error = origError as unknown as typeof console.error;
    document.body.innerHTML = '<div id="root"></div>';
    Object.defineProperty(window, 'parent', { configurable: true, value: { postMessage: (m: unknown) => posted.push(JSON.parse(JSON.stringify(m))) } });
  });
  afterEach(() => vi.useRealTimers());

  it('captures, deduplicates, rate-limits and reports blank or healthy pages', async () => {
    new Function(script)();
    const errs = () => posted.filter((m) => m.type === 'genius:runtime').flatMap((m) => m.errors!);
    const err = new TypeError("Cannot read properties of undefined (reading 'map')");
    err.stack = `TypeError: ${err.message}\n    at WidgetGrid (http://127.0.0.1:5173/src/components/WidgetGrid.tsx?t=1:42:13)\n    at x (http://127.0.0.1:5173/node_modules/.vite/deps/react-dom.js:9:9)`;
    for (let i = 0; i < 3; i++) window.dispatchEvent(new ErrorEvent('error', { error: err, message: err.message, filename: 'http://127.0.0.1:5173/node_modules/.vite/deps/react-dom.js', lineno: 9, colno: 9 }));
    console.error('Failed to load %s: %d', 'widgets', 500);
    const rej = new Event('unhandledrejection');
    Object.assign(rej, { reason: new Error('fetch failed') });
    window.dispatchEvent(rej);
    const img = document.createElement('img');
    img.src = 'http://127.0.0.1:5173/logo.png';
    document.body.appendChild(img);
    img.dispatchEvent(new Event('error'));
    expect(origError).toHaveBeenCalledWith('Failed to load %s: %d', 'widgets', 500);
    expect(posted).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(300);
    expect(posted).toHaveLength(1);
    const [e1, c1, r1, res] = errs();
    expect(e1).toMatchObject({ kind: 'error', message: "TypeError: Cannot read properties of undefined (reading 'map')", source: 'http://127.0.0.1:5173/src/components/WidgetGrid.tsx?t=1', line: 42, column: 13, count: 3, page: '/' });
    expect(e1.stack).toContain('WidgetGrid');
    expect(c1).toMatchObject({ kind: 'console', source: 'console.error', message: 'Failed to load widgets: 500', count: 1 });
    expect(r1).toMatchObject({ kind: 'rejection', message: 'Error: fetch failed' });
    expect(res).toMatchObject({ kind: 'resource', message: 'Failed to load image: http://127.0.0.1:5173/logo.png' });

    // repeats are sent as a count delta at most every ~5 s
    window.dispatchEvent(new ErrorEvent('error', { error: err, message: err.message }));
    await vi.advanceTimersByTimeAsync(300);
    expect(posted).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(7000);
    expect(posted).toHaveLength(2);
    expect(errs().at(-1)).toMatchObject({ kind: 'error', count: 1 });

    // blank page ~2.5 s after load, healthy after an HMR update that renders content
    window.dispatchEvent(new Event('load'));
    await vi.advanceTimersByTimeAsync(3000);
    expect(errs().at(-1)).toMatchObject({ kind: 'blank', message: expect.stringContaining('#root is empty') });
    const root = document.getElementById('root')!;
    root.innerHTML = '<h1>Dashboard</h1>';
    root.getBoundingClientRect = () => new DOMRect(0, 0, 800, 600);
    console.debug('[vite] hot updated: /src/App.tsx');
    await vi.advanceTimersByTimeAsync(3000);
    expect(posted.at(-1)).toEqual({ type: 'genius:runtime-healthy', page: '/' });

    // at most 20 errors leave the page per 10 s; the rest follow later, none lost
    await vi.advanceTimersByTimeAsync(11_000);
    const before = errs().length;
    for (let i = 0; i < 30; i++) window.dispatchEvent(new ErrorEvent('error', { message: `e${i}` }));
    await vi.advanceTimersByTimeAsync(2500);
    expect(errs().length - before).toBe(20);
    await vi.advanceTimersByTimeAsync(10_000);
    expect(errs().length - before).toBe(30);
  });
});

describe('preview problem bar', () => {
  beforeEach(() => {
    installIpc();
    useApp.setState({ settings });
    useWorkspace.setState({ projectId: 'p1', project, tasks: [], previewErrors: [], activeConversationId: null, preview: { status: 'ready', url: 'http://127.0.0.1:5199/', nonce: 0 } });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  const post = (source: MessageEventSource | null, data: unknown) => act(async () => void window.dispatchEvent(new MessageEvent('message', { origin: 'http://127.0.0.1:5199', source, data })));

  it('forwards errors from the preview, explains them and asks Genius to fix them', async () => {
    render(<PreviewPanel onStart={() => undefined} />);
    const frame = screen.getByTitle('Application preview') as HTMLIFrameElement;
    await post(window, { type: 'genius:runtime', errors: [crash] });
    await flush(400);
    expect(calls.some((c) => c.cmd === 'report_preview_errors')).toBe(false);
    await post(frame.contentWindow, { type: 'genius:runtime', errors: [crash, { kind: 'bogus', message: 'x' }] });
    await flush(400);
    const report = calls.find((c) => c.cmd === 'report_preview_errors')!;
    expect(report.args).toMatchObject({ projectId: 'p1', errors: [{ kind: 'error', message: crash.message }] });
    expect((report.args.errors as unknown[]).length).toBe(1);
    expect(screen.getByRole('alert')).toHaveTextContent("Your app crashed: TypeError: Cannot read properties of undefined (reading 'map') — WidgetGrid.tsx:42");
    expect(screen.getByText('×2')).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Details' })));
    expect(screen.getByText(/at renderWithHooks/)).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Fix it' })));
    await flush();
    const agent = calls.find((c) => c.cmd === 'start_agent')!;
    const req = agent.args.request as { conversationId: string; message: string; mode: string };
    expect(req.conversationId).toBe('c1');
    expect(req.mode).toBe('build');
    expect(req.message).toMatch(/^The preview has runtime errors\. Find the cause and fix it:\n\[1\] Uncaught error \(×2\)/);
    expect(useWorkspace.getState().rightTab).toBe('assistant');

    // a reload that renders fine clears the problem
    await post(frame.contentWindow, { type: 'genius:runtime-healthy', page: '/' });
    expect(calls.some((c) => c.cmd === 'clear_preview_errors')).toBe(true);
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('shows a blank page, hides warnings and can be dismissed', async () => {
    useWorkspace.setState({ previewErrors: [warn] });
    render(<PreviewPanel onStart={() => undefined} />);
    expect(screen.queryByRole('alert')).toBeNull();
    await act(async () => useWorkspace.setState({ previewErrors: [warn, blank] }));
    expect(screen.getByRole('alert')).toHaveTextContent('Your app shows a blank page');
    expect(screen.getByRole('button', { name: 'Fix it' })).toBeEnabled();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Dismiss' })));
    expect(calls.some((c) => c.cmd === 'clear_preview_errors' && c.args.projectId === 'p1')).toBe(true);
    expect(screen.queryByRole('alert')).toBeNull();
  });
});

describe('website deploy', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('requires a published repository', async () => {
    installIpc();
    render(<DeploySection projectId="p1" published={false} />);
    await flush();
    expect(screen.getByRole('button', { name: 'Deploy website' })).toBeDisabled();
    expect(screen.getByText(/Publish the project to GitHub first/)).toBeInTheDocument();
  });

  it('deploys, shows progress and the live link', async () => {
    let st: DeployStatus | null = { step: 'deployed', repo: 'me/dash', url: 'https://me.github.io/dash/', head: 'abc', at: '2026-10-01T00:00:00Z', deployedAt: '2026-10-01T00:00:00Z', error: null, running: false };
    installIpc({ deploy_status: () => st });
    render(<DeploySection projectId="p1" published />);
    await flush();
    expect(screen.getByText('https://me.github.io/dash/')).toBeInTheDocument();
    expect(screen.getByText('live')).toBeInTheDocument();
    st = { ...st!, step: 'building', running: true };
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Redeploy' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'deploy_pages' && c.args.projectId === 'p1')).toBe(true);
    expect(screen.getByText('Building your app for the web…')).toBeInTheDocument();
    st = { ...st, step: 'failed', running: false, error: 'The production build failed (exit code 1)' };
    await flush(1700);
    expect(screen.getByText('The production build failed (exit code 1)')).toBeInTheDocument();
    expect(screen.getByText('https://me.github.io/dash/')).toBeInTheDocument();
  });
});

describe('simple mode "Fix it for me"', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('sends runtime errors to Genius when the build did not fail', async () => {
    installIpc({ get_settings: () => ({ ...settings, uiMode: 'simple' }), preview_errors: () => [crash] });
    useApp.setState({ route: { name: 'home' } });
    useWorkspace.setState({ projectId: null });
    render(<App />);
    await flush();
    await act(async () => useApp.getState().navigate({ name: 'project', id: 'p1' }));
    await flush(60);
    expect(screen.getByText('Needs a fix')).toBeInTheDocument();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Fix it for me' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'start_diagnose')).toBe(false);
    expect((calls.find((c) => c.cmd === 'start_agent')!.args.request as { message: string }).message).toContain('WidgetGrid.tsx:42:13');
  });
});
