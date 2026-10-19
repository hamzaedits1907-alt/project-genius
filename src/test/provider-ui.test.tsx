// Provider selection, connection diagnostics, task limits and the Project Brain panel against the
// mocked backend. They prove what the UI sends and shows; they do not call any provider.
import { afterEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, project, settings } from './ipc';
import { DiagnosticsCard, LimitsCard, ProviderCard } from '../views/ProviderPanels';
import BrainPanel from '../views/workspace/BrainPanel';
import { Report } from '../views/workspace/assistant/Message';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';
import type { BrainAnswer, BrainOverview, DiagReport, ProviderCapabilities, Settings } from '../lib/types';

const flush = (ms = 40) => act(() => new Promise((r) => setTimeout(r, ms)));
afterEach(() => {
  cleanup();
  clearMocks();
});

const caps = (p: ProviderCapabilities['provider'], label: string, verification: ProviderCapabilities['verification']): ProviderCapabilities => ({ provider: p, label, wire: 'x', streaming: true, usageReporting: true, modelListing: true, imageInput: 'by-model', autoRouting: p === 'nvidia', toolProtocol: 'genius-text-tools', verification, notes: [`${label} note`] });
const CAPS = [caps('nvidia', 'NVIDIA NIM', 'live'), caps('openai', 'OpenAI', 'contract-tests'), caps('anthropic', 'Anthropic', 'contract-tests'), caps('gemini', 'Google Gemini', 'contract-tests'), caps('custom', 'OpenAI-compatible endpoint', 'contract-tests')];

describe('provider card', () => {
  it('switching provider saves its default address, keeps Auto where models are picked by family, forces manual for custom endpoints and clears the model', async () => {
    installIpc({ provider_capabilities: () => CAPS });
    const saved: Settings[] = [];
    const save = async (s: Settings) => (saved.push(s), true);
    render(<ProviderCard settings={{ ...settings, modelMode: 'auto', model: 'meta/llama' }} save={save} />);
    await flush();
    expect(screen.getByText('verified live')).toBeTruthy();
    fireEvent.click(screen.getByRole('radio', { name: 'Anthropic' }));
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatchObject({ provider: 'anthropic', baseUrl: 'https://api.anthropic.com/v1', modelMode: 'auto', model: null });
    fireEvent.click(screen.getByRole('radio', { name: /OpenAI-compatible/ }));
    expect(saved[1]).toMatchObject({ provider: 'custom', modelMode: 'manual', model: null });
  });

  it('is honest that non-NVIDIA providers are only contract-tested', async () => {
    installIpc({ provider_capabilities: () => CAPS });
    render(<ProviderCard settings={{ ...settings, provider: 'gemini' }} save={async () => true} />);
    await flush();
    expect(screen.getByText('contract-tested only')).toBeTruthy();
    expect(screen.getByText('Google Gemini note')).toBeTruthy();
    expect(screen.getByText(/only ever sent to that provider/)).toBeTruthy();
  });
});

describe('diagnostics', () => {
  const report: DiagReport = {
    provider: 'anthropic', label: 'Anthropic', integrationVerified: 'contract-tests', host: 'api.anthropic.com', model: null, ok: false,
    verdict: 'Not ready: Reach the provider and list models — Anthropic did not accept this key.',
    steps: [
      { id: 'credentials', title: 'API key', status: 'ok', detail: 'Saved in the system credential store (value not shown).', hint: null, ms: null },
      { id: 'models', title: 'Reach the provider and list models', status: 'fail', detail: 'The provider rejected the API key (HTTP 401).', hint: 'Anthropic did not accept this key. Check that it is an Anthropic key.', ms: 120 },
      { id: 'chat', title: 'Send a test message', status: 'skipped', detail: 'Needs a working model.', hint: null, ms: null },
    ],
    checkedAt: '2026-10-19T10:00:00Z',
  };

  it('runs the quick or deep check and shows the verdict, the failing step and what to do', async () => {
    installIpc({ diagnose_provider: () => report });
    render(<DiagnosticsCard keyStored />);
    fireEvent.click(screen.getByRole('button', { name: 'Run diagnostics' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'diagnose_provider')?.args).toEqual({ deep: false });
    expect(screen.getByRole('alert').textContent).toContain('Not ready');
    expect(screen.getByText('Check that it is an Anthropic key.', { exact: false })).toBeTruthy();
    expect(screen.getByText(/integration: contract tests only/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Deep check' }));
    await flush();
    expect(calls.filter((c) => c.cmd === 'diagnose_provider').map((c) => c.args.deep)).toEqual([false, true]);
  });

  it('deep check needs a saved key', () => {
    installIpc();
    render(<DiagnosticsCard keyStored={false} />);
    expect((screen.getByRole('button', { name: 'Deep check' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('surfaces a backend failure as an error toast instead of silence', async () => {
    installIpc({ diagnose_provider: () => { throw { kind: 'provider_network', message: 'boom' }; } });
    useApp.setState({ toasts: [] });
    render(<DiagnosticsCard keyStored />);
    fireEvent.click(screen.getByRole('button', { name: 'Run diagnostics' }));
    await flush();
    expect(useApp.getState().toasts.some((t) => t.tone === 'bad')).toBe(true);
  });
});

describe('limits', () => {
  it('saves exactly the edited numbers and is not a spending cap', async () => {
    installIpc();
    const saved: Settings[] = [];
    render(<LimitsCard settings={settings} save={async (s) => (saved.push(s), true)} />);
    expect(screen.getByText(/not a spending cap/)).toBeTruthy();
    const save = screen.getByRole('button', { name: 'Save limits' }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText('Model calls per task'), { target: { value: '12' } });
    fireEvent.change(screen.getByLabelText('Tasks at the same time'), { target: { value: '99' } });
    expect(save.disabled).toBe(false);
    fireEvent.click(save);
    expect(saved[0].limits).toMatchObject({ maxModelCalls: 12, maxConcurrentTasks: 32 });
  });
});

describe('project brain panel', () => {
  const overview: BrainOverview = {
    projectType: ['React (Vite)'], languages: [['typescript', 4]], entryPoints: ['src/main.tsx'],
    commands: [{ purpose: 'build', command: 'npm run build', source: 'package.json scripts.build: tsc && vite build' }],
    modules: [], keyFiles: [{ path: 'src/lib/price.ts', importedBy: 3, reason: 'imported by 3 files' }], externalDeps: [],
    files: 4, lines: 40, symbols: 6, unresolvedImports: 0, tests: 1, indexedAt: '2026-10-19T10:00:00Z', truncated: false, provenance: 'pattern-based scan',
  };
  const answer: BrainAnswer = {
    indexedAt: '2026-10-19T10:00:00Z', provenance: 'pattern-based scan',
    hits: [{ path: 'src/lib/price.ts', kind: 'function', name: 'formatPrice', line: 1, score: 12 }],
    excerpts: [{ path: 'src/lib/price.ts', line: 1, text: '   1  export function formatPrice() {}' }],
    impact: { changed: ['src/lib/price.ts'], unknown: [], affected: [{ path: 'src/App.tsx', depth: 1, via: 'src/lib/price.ts' }], tests: ['src/lib/price.test.ts'], truncated: false, note: 'a lower bound' },
    notes: [{ kind: 'note', origin: 'agent inference', updated: '2026-01-01', text: 'formatPrice lives in src/utils/money.ts', stale: ['mentions files that no longer exist: src/utils/money.ts'] }],
    pastChanges: [], gitHistory: [{ path: 'src/lib/price.ts', short: 'abc1234', date: '2026-10-01', subject: 'Format prices' }],
  };

  it('shows the verified overview, answers questions, and flags stale recorded notes separately', async () => {
    installIpc({ brain_overview: () => overview, brain_ask: () => answer });
    useWorkspace.setState({ project });
    render(<BrainPanel />);
    await flush();
    expect(screen.getByText('React (Vite)')).toBeTruthy();
    expect(screen.getByText('npm run build')).toBeTruthy();
    expect(screen.getByText(/pattern-based, not a compiler/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Ask the Project Brain'), { target: { value: 'formatPrice' } });
    fireEvent.change(screen.getByLabelText('Files to analyse for impact'), { target: { value: 'src/lib/price.ts, ' } });
    fireEvent.click(screen.getByRole('button', { name: 'Ask' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'brain_ask')?.args).toEqual({ projectId: 'p1', query: 'formatPrice', impact: ['src/lib/price.ts'] });
    expect(screen.getByText('Verified source facts')).toBeTruthy();
    expect(screen.getByText('Tests to run:', { exact: false })).toBeTruthy();
    expect(screen.getByText('Recorded history')).toBeTruthy();
    expect(screen.getByText(/Possibly stale: mentions files that no longer exist/)).toBeTruthy();
    expect(screen.getByText('Format prices', { exact: false })).toBeTruthy();
  });
});

describe('task report', () => {
  it('shows a task stopped by a limit as blocked, naming the limit', () => {
    const report = { lifecycle: 'blocked', steps: 3, filesRead: [], searches: 0, checks: [], subagents: [], failedSteps: [], changeset: null, verification: null, notVerified: [], warnings: [], error: null, budget: { stoppedBy: { limit: 'maxModelCalls', used: 3, max: 3 } } } as unknown as Parameters<typeof Report>[0]['report'];
    render(<Report report={report} />);
    expect(screen.getByText(/stopped by a Genius limit before finishing/)).toBeTruthy();
    expect(screen.getByText(/Limit reached: maxModelCalls \(3\) after 3/)).toBeTruthy();
  });
});
