// "Improve my prompt": rewrites the idea in place, offers Undo, and keeps the text on errors.
import { useState } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, settings } from './ipc';
import App from '../App';
import { EnhanceButton } from '../components/EnhanceButton';
import { useApp } from '../store/app';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const ready = { key_status: () => ({ stored: true, hint: '…abcd', backend: 'memory', error: null }), list_projects: () => [] };

describe('prompt enhancer on Home', () => {
  beforeEach(() => {
    installIpc(ready);
    useApp.setState({ route: { name: 'home' }, settings, toasts: [] });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('improves the idea in place and can undo it', async () => {
    render(<App />);
    await flush();
    const box = screen.getByLabelText('Describe your app') as HTMLTextAreaElement;
    const improve = screen.getByRole('button', { name: 'Improve my prompt' });
    expect(improve).toBeDisabled(); // nothing typed yet
    fireEvent.change(box, { target: { value: 'recipe site' } });
    expect(improve).toBeEnabled();
    await act(async () => void fireEvent.click(improve));
    await flush();
    expect(calls.find((c) => c.cmd === 'enhance_prompt')?.args).toEqual({ text: 'recipe site' });
    expect(box.value).toMatch(/^A polished web app for: recipe site\.\nPages\/sections:/);
    const undo = screen.getByRole('button', { name: /Undo/ });
    await act(async () => void fireEvent.click(undo));
    expect(box.value).toBe('recipe site');
    expect(screen.queryByRole('button', { name: /Undo/ })).toBeNull();
  });

  it('hides Undo once the improved text is edited', async () => {
    render(<App />);
    await flush();
    const box = screen.getByLabelText('Describe your app') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'a habit tracker' } });
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Improve my prompt' })));
    await flush();
    expect(screen.getByRole('button', { name: /Undo/ })).toBeInTheDocument();
    fireEvent.change(box, { target: { value: `${box.value} with streaks` } });
    expect(screen.queryByRole('button', { name: /Undo/ })).toBeNull();
  });

  it('keeps the text and explains when improving fails', async () => {
    installIpc({ ...ready, enhance_prompt: () => Promise.reject({ kind: 'provider_rate_limited', message: 'Too many requests' }) });
    render(<App />);
    await flush();
    const box = screen.getByLabelText('Describe your app') as HTMLTextAreaElement;
    fireEvent.change(box, { target: { value: 'recipe site' } });
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Improve my prompt' })));
    await flush();
    expect(box.value).toBe('recipe site');
    expect(useApp.getState().toasts.some((t) => t.tone === 'bad' && t.text.includes('Too many requests'))).toBe(true);
  });

  it('is disabled until the AI is set up', async () => {
    installIpc({ list_projects: () => [] });
    render(<App />);
    await flush();
    fireEvent.change(screen.getByLabelText('Describe your app'), { target: { value: 'recipe site' } });
    expect(screen.getByRole('button', { name: 'Improve my prompt' })).toBeDisabled();
  });
});

describe('EnhanceButton (reusable)', () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  function Harness({ initial }: { initial: string }) {
    const [v, setV] = useState(initial);
    return (
      <>
        <textarea aria-label="text" value={v} onChange={(e) => setV(e.target.value)} />
        <EnhanceButton value={v} onChange={setV} compact />
      </>
    );
  }

  it('does not overwrite text typed while the request was running', async () => {
    let resolve: (s: string) => void = () => {};
    installIpc({ enhance_prompt: () => new Promise<string>((r) => (resolve = r)) });
    render(<Harness initial="todo app" />);
    const box = screen.getByLabelText('text') as HTMLTextAreaElement;
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Improve' })));
    expect(screen.getByRole('button', { name: 'Improving…' })).toBeDisabled();
    fireEvent.change(box, { target: { value: 'todo app for teams' } });
    await act(async () => resolve('A detailed brief'));
    await flush();
    expect(box.value).toBe('todo app for teams');
  });

  it('rejects text over the limit without calling the backend', async () => {
    installIpc();
    render(<Harness initial={'x'.repeat(4001)} />);
    expect(screen.getByRole('button', { name: 'Improve' })).toBeDisabled();
    expect(calls.some((c) => c.cmd === 'enhance_prompt')).toBe(false);
  });
});
