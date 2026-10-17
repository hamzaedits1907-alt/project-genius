// AI provider settings: auto model routing, effort and thinking (mocked backend, no real model).
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, settings } from './ipc';
import App from '../App';
import { useApp } from '../store/app';
import type { Settings } from '../lib/types';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const saved = () => calls.filter((c) => c.cmd === 'save_settings').map((c) => c.args.settings as Settings);

describe('AI provider settings', () => {
  beforeEach(() => {
    installIpc();
    useApp.setState({ route: { name: 'settings' } });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('auto mode shows the models Genius will use and switches to manual', async () => {
    render(<StrictMode><App /></StrictMode>);
    await flush(60);
    expect(screen.getByRole('radio', { name: /Auto \(recommended\)/ })).toBeChecked();
    const table = screen.getByRole('table', { name: 'Models Genius will use' });
    expect(table.textContent).toContain('nvidia/nemotron-3-super-120b-a12b');
    expect(table.textContent).toContain('then qwen/qwen3-coder-480b-a35b-instruct');
    expect(table.textContent).toContain('No vision model available');
    expect(screen.queryByRole('listbox', { name: 'Available models' })).toBeNull();
    await act(async () => void fireEvent.click(screen.getByRole('radio', { name: /Choose a model/ })));
    await flush();
    expect(saved().at(-1)?.modelMode).toBe('manual');
    expect(screen.getByRole('listbox', { name: 'Available models' })).toBeInTheDocument();
  });

  it('effort and thinking are saved', async () => {
    render(<StrictMode><App /></StrictMode>);
    await flush(60);
    expect(screen.getByRole('radio', { name: 'Max' })).toHaveAttribute('aria-checked', 'true');
    await act(async () => void fireEvent.click(screen.getByRole('radio', { name: 'Balanced' })));
    await flush();
    expect(saved().at(-1)?.effort).toBe('balanced');
    await act(async () => void fireEvent.click(screen.getByLabelText('Let the model think')));
    await flush();
    expect(saved().at(-1)?.thinking).toBe(false);
  });

  it('manual mode keeps the model picker', async () => {
    installIpc({ get_settings: () => ({ ...settings, modelMode: 'manual' }) });
    render(<StrictMode><App /></StrictMode>);
    await flush(60);
    expect(screen.getByRole('radio', { name: /Choose a model/ })).toBeChecked();
    expect(screen.getByText(`Selected: ${settings.model}`)).toBeInTheDocument();
  });
});
