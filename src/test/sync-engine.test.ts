import { describe, expect, it } from 'vitest';
import { ACCOUNT_KEY, relink, syncOnce, type Collected, type SyncPorts } from '../web/syncEngine';

type Row = Record<string, unknown>;

function ports(opts: { convs?: Row[]; msgs?: Row[]; queue?: Collected[]; pushFail?: (c: Row[], m: Row[]) => unknown } = {}) {
  const kv = new Map<string, string>();
  const calls: string[] = [];
  const pulls: { kind: string; after: unknown }[] = [];
  const queue = [...(opts.queue ?? [])];
  const acked: string[][] = [];
  const pushed: { c: Row[]; m: Row[] }[] = [];
  const p: SyncPorts = {
    get: async (k) => kv.get(k) ?? null,
    set: async (k, v) => void kv.set(k, v),
    enqueueAll: async () => (calls.push('enqueueAll'), 0),
    collect: async () => queue.shift() ?? { conversations: [], messages: [], keys: [], remaining: 0, skipped: [] },
    ack: async (keys) => void acked.push(keys.map((k) => k.id)),
    apply: async (b) => (calls.push(b.conversations ? 'apply-conv' : 'apply-msg'), { conversations: b.conversations?.length ?? 0, messages: b.messages?.length ?? 0, deleted: 0, skipped: 0 }),
    pullConversations: async (after, limit) => {
      pulls.push({ kind: 'conv', after });
      const all = opts.convs ?? [];
      const start = after && 'id' in after && after.id ? all.findIndex((r) => r.id === after.id) + 1 : 0;
      return all.slice(start, start + limit);
    },
    pullMessages: async (after, limit) => {
      pulls.push({ kind: 'msg', after });
      const all = opts.msgs ?? [];
      const start = after && 'id' in after && after.id ? all.findIndex((r) => r.id === after.id) + 1 : 0;
      return all.slice(start, start + limit);
    },
    push: async (c, m) => {
      const e = opts.pushFail?.(c, m);
      if (e) throw e;
      pushed.push({ c, m });
    },
    now: () => Date.parse('2026-10-19T12:00:00Z'),
  };
  return { p, kv, calls, pulls, acked, pushed };
}
const key = (id: string) => ({ kind: 'conv', id, ts: '2026-10-19T11:59:00Z' });
const collected = (ids: string[], remaining = 0): Collected => ({ conversations: ids.map((id) => ({ id })), messages: [], keys: ids.map(key), remaining, skipped: [] });

describe('desktop sync engine', () => {
  it('the first sync binds the computer to the account and queues the existing history once', async () => {
    const t = ports();
    await syncOnce(t.p, 'alice');
    expect(t.kv.get(ACCOUNT_KEY)).toBe('alice');
    expect(t.calls.filter((c) => c === 'enqueueAll')).toHaveLength(1);
    await syncOnce(t.p, 'alice');
    expect(t.calls.filter((c) => c === 'enqueueAll')).toHaveLength(1);
  });

  it("does nothing (and does not merge) when the computer's history belongs to another account", async () => {
    const t = ports({ convs: [{ id: 'c1', syncedAt: '2026-10-19T11:00:00Z' }] });
    t.kv.set(ACCOUNT_KEY, 'alice');
    const r = await syncOnce(t.p, 'bob');
    expect(r.paused).toBe(true);
    expect(t.pulls).toHaveLength(0);
    expect(t.calls).toEqual([]);
    await relink(t.p, 'bob'); // the user chose to move the history to this account
    expect(t.kv.get(ACCOUNT_KEY)).toBe('bob');
    expect(t.calls).toEqual(['enqueueAll']);
  });

  it('applies conversations before messages, pages through everything and remembers the cursor', async () => {
    const convs = Array.from({ length: 250 }, (_, i) => ({ id: `c${String(i).padStart(3, '0')}`, syncedAt: `2026-10-19T10:${String(Math.floor(i / 60)).padStart(2, '0')}:${String(i % 60).padStart(2, '0')}Z` }));
    const msgs = [{ id: 'm1', syncedAt: '2026-10-19T11:00:00Z' }];
    const t = ports({ convs, msgs });
    const r = await syncOnce(t.p, 'alice');
    expect(r.pulled).toBe(251);
    expect(r.changed).toBe(true);
    expect(t.calls.filter((c) => c.startsWith('apply'))).toEqual(['apply-conv', 'apply-conv', 'apply-msg']);
    expect(JSON.parse(t.kv.get('sync.cursor.conv.alice')!)).toEqual({ id: 'c249', ts: convs[249].syncedAt });
    expect(JSON.parse(t.kv.get('sync.cursor.msg.alice')!).id).toBe('m1');
  });

  it('starts the next pull a little before the saved cursor, so a transaction that committed late is not missed', async () => {
    const t = ports({ convs: [] });
    t.kv.set(ACCOUNT_KEY, 'alice');
    t.kv.set('sync.cursor.conv.alice', JSON.stringify({ ts: '2026-10-19T11:50:00.000Z', id: 'c9' }));
    await syncOnce(t.p, 'alice');
    expect(t.pulls[0]).toEqual({ kind: 'conv', after: { ts: '2026-10-19T11:49:30.000Z', id: null } });
  });

  it('acknowledges uploaded changes only after the server accepted them', async () => {
    const ok = ports({ queue: [collected(['a', 'b'])] });
    ok.kv.set(ACCOUNT_KEY, 'alice');
    const r = await syncOnce(ok.p, 'alice');
    expect(r.pushed).toBe(2);
    expect(ok.acked).toEqual([['a', 'b']]);

    const down = ports({ queue: [collected(['a'])], pushFail: () => new Error('Failed to fetch') });
    down.kv.set(ACCOUNT_KEY, 'alice');
    await expect(syncOnce(down.p, 'alice')).rejects.toThrow('Failed to fetch');
    expect(down.acked).toEqual([]); // still queued: it is sent on the next run
  });

  it('one malformed row is reported and skipped; the others still upload', async () => {
    const t = ports({ queue: [collected(['good1', 'bad', 'good2'])], pushFail: (c) => (c.some((x) => x.id === 'bad') ? { code: '22023', message: 'unsupported message role' } : null) });
    t.kv.set(ACCOUNT_KEY, 'alice');
    const r = await syncOnce(t.p, 'alice');
    expect(t.pushed.flatMap((x) => x.c.map((y) => y.id))).toEqual(['good1', 'good2']);
    expect(r.warnings.join(' ')).toMatch(/rejected/);
    expect(t.acked).toEqual([['good1', 'bad', 'good2']]);
  });

  it('keeps pushing until the queue is empty', async () => {
    const t = ports({ queue: [collected(['a'], 1), collected(['b'], 0)] });
    t.kv.set(ACCOUNT_KEY, 'alice');
    const r = await syncOnce(t.p, 'alice');
    expect(r.pushed).toBe(2);
    expect(t.acked).toEqual([['a'], ['b']]);
  });
});
