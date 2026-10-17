// Beginner entry point against a mocked backend (labelled: no real model or npm involved).
import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { calls, clearMocks, installIpc, settings } from './ipc';
import Home from '../views/Home';
import { useApp } from '../store/app';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));
const tools = [
  { name: 'node', found: true, path: '/n', version: 'v22', hint: '' },
  { name: 'npm', found: true, path: '/n', version: '10', hint: '' },
  { name: 'git', found: false, path: null, version: null, hint: 'Install Git from https://git-scm.com' },
];

describe('home: describe an app and build it', () => {
  beforeEach(() => {
    installIpc({ detect_tools: () => tools });
    useApp.setState({ settings, keyStatus: { stored: true, hint: 'nvapi-…abcd', backend: 'memory', error: null }, route: { name: 'home' } });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  it('creates the project in one step and opens it', async () => {
    render(<StrictMode><Home /></StrictMode>);
    await flush();
    expect(screen.getByText('ready to build')).toBeInTheDocument();
    expect(screen.getByText(/Git not found/)).toBeInTheDocument();
    const create = screen.getByRole('button', { name: 'Create' });
    expect(create).toBeDisabled();
    await act(async () => void fireEvent.change(screen.getByLabelText('Describe your app'), { target: { value: 'Create a modern personal dashboard with notes, a weather widget, and quick links.' } }));
    expect(create).not.toBeDisabled();
    await act(async () => void fireEvent.click(create));
    await flush();
    const call = calls.find((c) => c.cmd === 'start_create_app');
    expect(call?.args).toEqual({ name: null, description: 'Create a modern personal dashboard with notes, a weather widget, and quick links.', reviewPlan: false });
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p1' });
  });

  it('explains why Create is unavailable before setup is complete', async () => {
    useApp.setState({ keyStatus: { stored: false, hint: null, backend: 'memory', error: null } });
    render(<StrictMode><Home /></StrictMode>);
    await flush();
    expect(screen.getByText('action needed')).toBeInTheDocument();
    expect(screen.getByText(/Before Genius can build: Save your NVIDIA API key/)).toBeInTheDocument();
  });
});
