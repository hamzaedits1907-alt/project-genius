// Blueprint gallery on Home: data integrity, filtering, filling the idea box, building.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { calls, clearMocks, installIpc, project, settings } from './ipc';
import App from '../App';
import { useApp } from '../store/app';
import { BLUEPRINTS, BLUEPRINT_FILTERS, filterBlueprints, blueprintById } from '../lib/blueprints';
import { hasBlueprintArt } from '../components/BlueprintArt';

const flush = (ms = 30) => act(() => new Promise((r) => setTimeout(r, ms)));

describe('blueprint data', () => {
  it('has at least 12 complete, unique blueprints', () => {
    expect(BLUEPRINTS.length).toBeGreaterThanOrEqual(12);
    expect(new Set(BLUEPRINTS.map((b) => b.id)).size).toBe(BLUEPRINTS.length);
    expect(new Set(BLUEPRINTS.map((b) => b.title)).size).toBe(BLUEPRINTS.length);
    for (const b of BLUEPRINTS) {
      expect(b.id).toMatch(/^[a-z0-9-]+$/);
      expect(['website', 'app', 'game', 'tool']).toContain(b.category);
      expect(b.title.trim().length).toBeGreaterThan(2);
      expect(b.description.length, b.id).toBeGreaterThan(20);
      expect(b.description.length, b.id).toBeLessThanOrEqual(90);
      expect(b.tags.length).toBeGreaterThanOrEqual(2);
      expect(b.tags.length).toBeLessThanOrEqual(4);
      expect(b.projectName.length).toBeGreaterThan(0);
      expect(b.projectName.length).toBeLessThanOrEqual(30);
      for (const c of b.colors) expect(c).toMatch(/^#[0-9a-f]{6}$/i);
      expect(hasBlueprintArt(b.id), `${b.id} has an illustration`).toBe(true);
    }
  });

  it('carries rich, buildable briefs', () => {
    for (const b of BLUEPRINTS) {
      // Long enough to be specific, short enough for the prompt enhancer and the planner.
      expect(b.prompt.length, b.id).toBeGreaterThan(600);
      expect(b.prompt.length, b.id).toBeLessThan(4000);
      for (const section of ['Pages & sections:', 'Key features:', 'Data:', 'Visual style:', 'Must-haves:']) expect(b.prompt, `${b.id}: ${section}`).toContain(section);
      expect(b.prompt, b.id).toMatch(/localStorage/);
      expect(b.prompt, b.id).toMatch(/responsive/);
      expect(b.prompt.split('\n- ').length - 1, `${b.id} lists features`).toBeGreaterThanOrEqual(3);
    }
    // Content that is invented is labelled as sample content.
    expect(BLUEPRINTS.filter((b) => /sample/i.test(b.prompt)).length).toBeGreaterThanOrEqual(BLUEPRINTS.length - 2);
  });

  it('covers every category and filters by category and text', () => {
    for (const f of BLUEPRINT_FILTERS) expect(filterBlueprints(BLUEPRINTS, f.id).length, f.id).toBeGreaterThan(0);
    expect(filterBlueprints(BLUEPRINTS, 'all')).toHaveLength(BLUEPRINTS.length);
    expect(filterBlueprints(BLUEPRINTS, 'game').every((b) => b.category === 'game')).toBe(true);
    expect(filterBlueprints(BLUEPRINTS, 'all', 'kanban').map((b) => b.id)).toEqual(['kanban']);
    expect(filterBlueprints(BLUEPRINTS, 'website', 'kanban')).toEqual([]);
    expect(filterBlueprints(BLUEPRINTS, 'all', 'CHARTS').length).toBeGreaterThan(1); // tags match, case-insensitive
    expect(blueprintById('game-2048')?.title).toBe('2048 puzzle');
  });
});

describe('blueprint gallery on Home', () => {
  beforeEach(() => {
    installIpc({
      key_status: () => ({ stored: true, hint: '…abcd', backend: 'memory', error: null }),
      create_project: (_c, a) => ({ ...project, id: 'p9', name: a.name, description: a.description }),
      start_plan: () => ({ id: 't9', projectId: 'p9', kind: 'plan', title: 'Plan', startedAt: '2026-10-01T00:00:00Z' }),
      list_projects: () => [],
    });
    useApp.setState({ route: { name: 'home' }, settings });
  });
  afterEach(() => {
    cleanup();
    clearMocks();
    vi.restoreAllMocks();
  });

  const cards = () => within(screen.getByRole('list')).getAllByRole('listitem');

  it('filters by category pills', async () => {
    render(<App />);
    await flush();
    expect(screen.getByRole('heading', { name: 'Start from a blueprint' })).toBeInTheDocument();
    expect(cards()).toHaveLength(BLUEPRINTS.length);
    const games = screen.getByRole('button', { name: /^Games/ });
    await act(async () => void fireEvent.click(games));
    expect(games).toHaveAttribute('aria-pressed', 'true');
    const shown = cards().map((c) => c.getAttribute('aria-label'));
    expect(shown).toEqual(BLUEPRINTS.filter((b) => b.category === 'game').map((b) => b.title));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /^Tools/ })));
    expect(cards().map((c) => c.getAttribute('aria-label'))).toContain('Docs / knowledge base');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /^All/ })));
    expect(cards()).toHaveLength(BLUEPRINTS.length);
  });

  it('clicking a card fills the idea box for customising; Enter builds it', async () => {
    render(<App />);
    await flush();
    const bp = blueprintById('kanban')!;
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Use blueprint: Kanban board' })));
    const box = screen.getByLabelText('Describe your app') as HTMLTextAreaElement;
    expect(box.value).toBe(bp.prompt);
    expect(document.activeElement).toBe(box);
    expect(screen.getByText(/Customize, then press Enter/)).toBeInTheDocument();
    expect(calls.some((c) => c.cmd === 'create_project')).toBe(false); // nothing is built yet
    await act(async () => void fireEvent.keyDown(box, { key: 'Enter' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'create_project')?.args).toEqual({ name: 'Kanban Board', description: bp.prompt });
    expect(calls.find((c) => c.cmd === 'start_plan')?.args).toMatchObject({ projectId: 'p9', description: bp.prompt });
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p9' });
  });

  it('an edited blueprint is named from its text, and the hint can be cleared', async () => {
    render(<App />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Use blueprint: Restaurant & menu' })));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Clear blueprint' })));
    const box = screen.getByLabelText('Describe your app') as HTMLTextAreaElement;
    expect(box.value).toBe('');
    expect(screen.queryByText(/Customize, then press Enter/)).toBeNull();
    // The existing idea chips keep working.
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Recipe book' })));
    expect(box.value).toMatch(/^A recipe book/);
  });

  it('"Build now" starts building straight from the card', async () => {
    render(<App />);
    await flush();
    const bp = blueprintById('game-2048')!;
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Build 2048 puzzle now' })));
    await flush();
    expect(calls.find((c) => c.cmd === 'create_project')?.args).toEqual({ name: '2048', description: bp.prompt });
    expect(calls.some((c) => c.cmd === 'start_plan')).toBe(true);
    expect(localStorage.getItem('genius.autoBuild')).toContain('p9');
  });

  it('"Build now" waits for setup', async () => {
    installIpc({ list_projects: () => [] }); // no API key stored
    render(<App />);
    await flush();
    expect(screen.getByRole('button', { name: 'Build 2048 puzzle now' })).toBeDisabled();
  });
});
