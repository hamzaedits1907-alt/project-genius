import { describe, expect, it } from 'vitest';
import { applyFileStream, buildTree, diffLines, matchesFilter, timeAgo, upsertEvent, languageFor } from './logic';
import type { ActivityEvent } from './types';

const ev = (id: string, ts: string, patch: Partial<ActivityEvent> = {}): ActivityEvent => ({
  id,
  projectId: 'p',
  taskId: null,
  kind: 'build',
  status: 'running',
  title: id,
  detail: {},
  ts,
  updatedTs: ts,
  ...patch,
});

describe('upsertEvent', () => {
  it('replaces by id and keeps order', () => {
    let l = upsertEvent([], ev('a', '2026-01-01T00:00:01Z'));
    l = upsertEvent(l, ev('b', '2026-01-01T00:00:02Z'));
    l = upsertEvent(l, ev('a', '2026-01-01T00:00:01Z', { status: 'completed' }));
    expect(l.map((e) => e.id)).toEqual(['a', 'b']);
    expect(l[0].status).toBe('completed');
  });
  it('sorts late arrivals and bounds size', () => {
    let l = upsertEvent([], ev('b', '2026-01-01T00:00:02Z'));
    l = upsertEvent(l, ev('a', '2026-01-01T00:00:01Z'));
    expect(l.map((e) => e.id)).toEqual(['a', 'b']);
    l = upsertEvent(l, ev('c', '2026-01-01T00:00:03Z'), 2);
    expect(l.map((e) => e.id)).toEqual(['b', 'c']);
  });
});

describe('filters', () => {
  it('matches categories', () => {
    expect(matchesFilter(ev('x', 't', { kind: 'git' }), 'git')).toBe(true);
    expect(matchesFilter(ev('x', 't', { kind: 'git' }), 'builds')).toBe(false);
    expect(matchesFilter(ev('x', 't', { status: 'failed' }), 'errors')).toBe(true);
    expect(matchesFilter(ev('x', 't', { kind: 'approval' }), 'approvals')).toBe(true);
  });
});

describe('buildTree', () => {
  it('nests folders first, sorted', () => {
    const t = buildTree([{ path: 'src/b.ts' }, { path: 'README.md' }, { path: 'src/components/A.tsx' }, { path: 'index.html' }]);
    expect(t.map((n) => n.name)).toEqual(['src', 'index.html', 'README.md']);
    expect(t[0].children.map((n) => n.name)).toEqual(['components', 'b.ts']);
    expect(t[0].children[0].children[0].path).toBe('src/components/A.tsx');
  });
});

describe('applyFileStream', () => {
  it('accumulates drafts and marks completion', () => {
    let d = applyFileStream({}, { type: 'fileStart', path: 'a.ts', taskId: 't', projectId: 'p' });
    d = applyFileStream(d, { type: 'fileDelta', path: 'a.ts', text: 'x\n', taskId: 't', projectId: 'p' });
    expect(d['a.ts']).toMatchObject({ content: 'x\n', status: 'streaming' });
    d = applyFileStream(d, { type: 'fileEnd', path: 'a.ts', content: 'x\n', complete: false, taskId: 't', projectId: 'p' });
    expect(d['a.ts'].status).toBe('incomplete');
  });
});

describe('helpers', () => {
  it('classifies diff lines', () => {
    expect(diffLines('--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n c').map((l) => l.kind)).toEqual(['meta', 'meta', 'hunk', 'del', 'add', 'ctx']);
  });
  it('formats time', () => {
    const now = Date.parse('2026-01-01T01:00:00Z');
    expect(timeAgo('2026-01-01T00:59:50Z', now)).toBe('just now');
    expect(timeAgo('2026-01-01T00:30:00Z', now)).toBe('30 min ago');
    expect(timeAgo(null, now)).toBe('never');
  });
  it('detects languages', () => {
    expect(languageFor('src/App.tsx')).toBe('tsx');
    expect(languageFor('a.json')).toBe('json');
    expect(languageFor('Makefile')).toBe('text');
  });
});
