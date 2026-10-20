// The sync algorithm for the desktop app, with every side effect passed in (local database commands, the
// server's sync functions), so it can be tested without a window or a network.
//
//   1. First time this computer syncs: bind it to the account and queue every existing conversation, so the
//      history already on the computer is uploaded once (same ids: never duplicated, never overwritten).
//   2. Pull: everything the server changed after our cursor (with a small overlap, because a transaction that
//      commits late can carry an older timestamp) is merged into the local database.
//   3. Push: what changed locally is uploaded in batches and acknowledged only after the server accepted it.
//
// A different account on a computer that already belongs to someone else's history is never merged silently.

export interface Cursor { ts: string; id: string }
export interface OutKey { kind: string; id: string; ts: string }
export interface Collected { conversations: Row[]; messages: Row[]; keys: OutKey[]; remaining: number; skipped: string[] }
type Row = Record<string, unknown>;

export interface SyncPorts {
  // local database (Tauri commands)
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  enqueueAll(): Promise<number>;
  collect(limit: number): Promise<Collected>;
  ack(keys: OutKey[]): Promise<void>;
  apply(batch: { conversations?: Row[]; messages?: Row[] }): Promise<{ conversations: number; messages: number; deleted: number; skipped: number }>;
  // the account's server (Supabase functions)
  pullConversations(after: Cursor | { ts: string; id: null } | null, limit: number): Promise<Row[]>;
  pullMessages(after: Cursor | { ts: string; id: null } | null, limit: number): Promise<Row[]>;
  push(conversations: Row[], messages: Row[]): Promise<void>;
  now?: () => number;
}

export interface SyncResult {
  paused: boolean;
  pulled: number;
  pushed: number;
  /** Local data changed because of the pull (the interface should reload its lists). */
  changed: boolean;
  warnings: string[];
}

const OVERLAP_MS = 30_000;
const PAGE_CONV = 200;
const PAGE_MSG = 300;
const BATCH = 200;

const cursorKey = (kind: 'conv' | 'msg', user: string) => `sync.cursor.${kind}.${user}`;
export const ACCOUNT_KEY = 'sync.account';

const code = (e: unknown) => (e && typeof e === 'object' && 'code' in e ? String((e as { code: unknown }).code) : '');
/** A row the server will never accept (malformed): skipped and reported, instead of blocking every later sync. */
const isRejection = (e: unknown) => /^22/.test(code(e));

function overlapped(c: Cursor | null, now: number): { ts: string; id: null } | null {
  if (!c) return null;
  const t = Date.parse(c.ts);
  return Number.isFinite(t) ? { ts: new Date(Math.min(t, now) - OVERLAP_MS).toISOString(), id: null } : null;
}

export async function syncOnce(p: SyncPorts, userId: string): Promise<SyncResult> {
  const now = (p.now ?? Date.now)();
  const out: SyncResult = { paused: false, pulled: 0, pushed: 0, changed: false, warnings: [] };

  const bound = await p.get(ACCOUNT_KEY);
  if (!bound) {
    await p.set(ACCOUNT_KEY, userId);
    await p.enqueueAll();
  } else if (bound !== userId) {
    out.paused = true;
    return out;
  }

  // ---- pull (conversations first: messages need their conversation to exist locally)
  for (const kind of ['conv', 'msg'] as const) {
    const saved = await p.get(cursorKey(kind, userId));
    let cur: Cursor | null = saved ? (JSON.parse(saved) as Cursor) : null;
    let after: Cursor | { ts: string; id: null } | null = overlapped(cur, now);
    for (let page = 0; page < 200; page++) {
      const rows: Row[] = kind === 'conv' ? await p.pullConversations(after, PAGE_CONV) : await p.pullMessages(after, PAGE_MSG);
      if (!rows.length) break;
      const rep = await p.apply(kind === 'conv' ? { conversations: rows } : { messages: rows });
      out.pulled += rows.length;
      if (rep.conversations || rep.messages || rep.deleted) out.changed = true;
      const last: Row = rows[rows.length - 1];
      cur = { ts: String(last.syncedAt), id: String(last.id) };
      after = cur;
      await p.set(cursorKey(kind, userId), JSON.stringify(cur));
      if (rows.length < (kind === 'conv' ? PAGE_CONV : PAGE_MSG)) break;
    }
  }

  // ---- push
  for (let i = 0; i < 50; i++) {
    const b = await p.collect(BATCH);
    if (!b.keys.length) break;
    out.warnings.push(...b.skipped);
    if (b.conversations.length || b.messages.length) {
      try {
        await p.push(b.conversations, b.messages);
        out.pushed += b.conversations.length + b.messages.length;
      } catch (e) {
        if (!isRejection(e) || b.conversations.length + b.messages.length < 2) {
          if (isRejection(e)) out.warnings.push(`A row was rejected by the server and skipped: ${(e as { message?: string }).message ?? 'invalid data'}`);
          else throw e;
        } else {
          // one bad row must not block the rest: send the rows one at a time, skipping only what is refused
          for (const c of b.conversations) await pushOne(p, [c], [], out);
          for (const m of b.messages) await pushOne(p, [], [m], out);
        }
      }
    }
    await p.ack(b.keys);
    if (b.remaining <= 0) break;
  }
  return out;
}

async function pushOne(p: SyncPorts, c: Row[], m: Row[], out: SyncResult) {
  try {
    await p.push(c, m);
    out.pushed += c.length + m.length;
  } catch (e) {
    if (!isRejection(e)) throw e;
    out.warnings.push(`A row was rejected by the server and skipped: ${(e as { message?: string }).message ?? 'invalid data'}`);
  }
}

/** The user chose to move this computer's conversations to the signed-in account. */
export async function relink(p: SyncPorts, userId: string) {
  await p.set(ACCOUNT_KEY, userId);
  await p.enqueueAll();
}
