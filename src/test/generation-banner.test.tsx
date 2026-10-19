// The banner that tells the truth about a generated project: drafts that are not in the project,
// a starter page that was never replaced, a build that fails, and a build nobody opened in a browser.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { clearMocks, installIpc, calls, project } from './ipc';
import GenerationBanner from '../views/workspace/GenerationBanner';
import { useWorkspace } from '../store/workspace';
import type { ProjectStatus } from '../lib/types';

const noop = () => undefined;
const setStatus = (status: ProjectStatus, drafts = {}) => useWorkspace.setState({ project: { ...project, status }, drafts, tasks: [] });
const mount = () => render(<GenerationBanner onReview={noop} onPlan={noop} onFix={noop} />);

describe('generation banner', () => {
  beforeEach(() => installIpc({ apply_changeset: () => ({ id: 'cs1', status: 'applied', files: [] }) }));
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it('says plainly that pending AI files are not in the project and offers to apply them', async () => {
    setStatus({ stage: 'generated', pendingChangeset: 'cs1', lastBuild: { status: 'succeeded', exitCode: 0, at: '2026-10-01T00:00:00Z', runId: 'r', durationMs: 1 } }, { 'src/App.tsx': { path: 'src/App.tsx', content: 'x', status: 'complete' }, 'src/a.ts': { path: 'src/a.ts', content: 'y', status: 'complete' } });
    mount();
    expect(screen.getByTestId('generation-banner')).toHaveTextContent('2 AI draft file(s) are NOT in your project yet');
    expect(screen.getByTestId('generation-banner')).toHaveTextContent('describe the version without them');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /apply and build/i })));
    expect(calls.some((c) => c.cmd === 'apply_changeset' && c.args.changesetId === 'cs1')).toBe(true);
  });

  it('is an alert when the starter page was never replaced', () => {
    setStatus({ stage: 'generation_incomplete' });
    mount();
    expect(screen.getByRole('alert')).toHaveTextContent('The application was not generated');
    expect(screen.getByRole('alert')).toHaveTextContent('starter page');
  });

  it('reports a failing build and a crashing app differently from success', () => {
    setStatus({ stage: 'generated', generation: { state: 'build_failed', at: 'x' } });
    const r = mount();
    expect(screen.getByRole('alert')).toHaveTextContent('does not build');
    r.unmount();
    setStatus({ stage: 'generated', generation: { state: 'runtime_errors', at: 'x' } });
    mount();
    expect(screen.getByRole('alert')).toHaveTextContent('fails when opened');
  });

  it('does not call a build "verified": it says the browser check did not happen', () => {
    setStatus({ stage: 'generated', generation: { state: 'built', at: 'x' } });
    mount();
    expect(screen.getByTestId('generation-banner')).toHaveTextContent('not verified');
  });

  it('shows nothing for a verified project', () => {
    setStatus({ stage: 'generated', generation: { state: 'verified', at: 'x' } });
    const r = mount();
    expect(r.container).toBeEmptyDOMElement();
  });
});
