// Owner admin panel. Every action calls a SECURITY DEFINER function in the database that checks the
// caller's administrator status itself and writes an audit record; this screen is only a front end for
// them. A person who reaches this page without being an administrator gets errors from the database,
// not data. Nothing here can create administrators (that is a one-time SQL step by the owner) and no
// provider key or message content ever appears here: only counts and account metadata.
import { useCallback, useEffect, useMemo, useState } from 'react';
import { Download, Loader2, Megaphone, Power, Search, ShieldAlert, Trash2, X } from 'lucide-react';
import { sb, useAuth, featureOf, type Limits } from './auth';

interface UserRow {
  id: string;
  email: string;
  display_name: string | null;
  status: 'active' | 'suspended';
  plan: 'free' | 'pro';
  plan_expires_at: string | null;
  created_at: string;
  last_seen_at: string | null;
  conversations: number;
  messages: number;
  events_30d: number;
  is_admin: boolean;
  total_count: number;
}
interface Flag { key: string; enabled: boolean; kill: boolean; rollout_percent: number; cohort_id: string | null; note: string | null }
interface Feature { key: string; name: string; description: string; min_plan: string }
interface Cohort { id: string; name: string; description: string }
interface Audit { id: number; at: string; actor: string; action: string; target_user: string | null; detail: Record<string, unknown> }
interface Detail {
  profile: { id: string; email: string; displayName: string | null; status: string; suspendedReason: string | null; createdAt: string; lastSeenAt: string | null };
  isAdmin: boolean;
  entitlement: { plan: string; expiresAt: string | null; note: string | null } | null;
  grants: { feature: string; expiresAt: string | null; reason: string | null; createdAt: string }[];
  cohorts: string[];
  note: string;
  usage30d: Record<string, number>;
  counts: { conversations: number; messages: number };
  audit: { at: string; action: string; detail: Record<string, unknown> }[];
}
interface Announcement { id: string; title: string; body: string; level: 'info' | 'warning' | 'critical'; active: boolean; starts_at: string; ends_at: string | null }

type Tab = 'overview' | 'users' | 'features' | 'early' | 'limits' | 'site' | 'audit' | 'system';
const errText = (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));
const day = (iso: string | null) => (iso ? iso.slice(0, 10) : '—');
const when = (iso: string | null) => (iso ? iso.slice(0, 16).replace('T', ' ') : 'never');

function csvOf(rows: Record<string, unknown>[]): string {
  if (!rows.length) return '';
  const cols = Object.keys(rows[0]);
  const cell = (v: unknown) => {
    let s = v == null ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v);
    if (/^[=+\-@]/.test(s)) s = `'${s}`; // spreadsheet formula injection
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  return [cols.join(','), ...rows.map((r) => cols.map((c) => cell(r[c])).join(','))].join('\n');
}
function download(name: string, text: string) {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }));
  const a = document.createElement('a');
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export default function AdminPanel() {
  const access = useAuth((s) => s.access);
  const [tab, setTab] = useState<Tab>('overview');
  const [err, setErr] = useState<string | null>(null);
  const [ok, setOk] = useState<string | null>(null);
  if (!access) return <div className="pj-root"><div className="pj-inner"><Loader2 className="spin" /> Checking your access…</div></div>;
  if (!access.isAdmin) {
    return (
      <div className="pj-root"><div className="pj-inner">
        <div className="callout bad" role="alert"><ShieldAlert size={16} /> <div><b>Not authorized.</b> This area is only for the owner of this Genius Web deployment.</div></div>
      </div></div>
    );
  }
  const tabs: [Tab, string][] = [['overview', 'Overview'], ['users', 'Users'], ['features', 'Features & flags'], ['early', 'Early access'], ['limits', 'Plans & limits'], ['site', 'Site controls'], ['audit', 'Audit log'], ['system', 'System']];
  const notify = { onError: (e: string | null) => { setErr(e); if (e) setOk(null); }, onOk: (m: string) => { setOk(m); setErr(null); setTimeout(() => setOk(null), 4000); } };
  return (
    <div className="pj-root">
      <div className="pj-inner" style={{ maxWidth: 1180 }}>
        <header className="pj-head"><div className="grow"><h1>Owner admin</h1><p className="muted" style={{ margin: '4px 0 0' }}>Every change here is checked by the database and written to the audit log. No message content and no provider keys are ever shown.</p></div></header>
        <div className="tabs" role="tablist" aria-label="Admin sections" style={{ height: 40, borderBottom: '1px solid var(--border)', overflowX: 'auto' }}>
          {tabs.map(([k, l]) => <button key={k} role="tab" aria-selected={tab === k} className={`tab ${tab === k ? 'on' : ''}`} onClick={() => setTab(k)}>{l}</button>)}
        </div>
        {err && <div className="callout bad" role="alert">{err}</div>}
        {ok && <div className="callout ok" role="status">{ok}</div>}
        {tab === 'overview' && <Overview {...notify} goTo={setTab} />}
        {tab === 'users' && <Users {...notify} />}
        {tab === 'features' && <Features {...notify} />}
        {tab === 'early' && <Early {...notify} />}
        {tab === 'limits' && <LimitsTab {...notify} />}
        {tab === 'site' && <Site {...notify} />}
        {tab === 'audit' && <AuditLog {...notify} />}
        {tab === 'system' && <System {...notify} />}
      </div>
    </div>
  );
}

type P = { onError: (e: string | null) => void; onOk: (m: string) => void };

function useLoad<T>(fn: () => Promise<T>, onError: (e: string | null) => void, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const reload = useCallback(async () => {
    setLoading(true);
    try {
      setData(await fn());
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setLoading(false);
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { data, loading, reload };
}

async function rpc<T = unknown>(fn: string, args?: Record<string, unknown>): Promise<T> {
  const { data, error } = await sb().rpc(fn, args);
  if (error) throw error;
  return data as T;
}

// ------------------------------------------------------------------------------------------ overview
const KIND_COLOR: Record<string, string> = { chat: 'var(--accent)', research: 'var(--ok)', agent: 'var(--warn)', code: 'var(--accent-2, #a99fff)', web_tool: 'var(--text-3)' };

function Overview({ onError, goTo }: P & { goTo: (t: Tab) => void }) {
  const { data, loading } = useLoad(async () => {
    const [summary, series, top, signups, settings] = await Promise.all([
      rpc<Record<string, unknown>>('admin_usage_summary'),
      rpc<{ day: string; kind: string; n: number }[]>('admin_usage_series', { days: 30 }),
      rpc<{ user_id: string; email: string; events: number }[]>('admin_top_users', { days: 30, lim: 8 }),
      rpc<{ day: string; n: number }[]>('admin_signups_series', { days: 30 }),
      sb().from('app_settings').select('key, value'),
    ]);
    const s = new Map(((settings.data ?? []) as { key: string; value: Record<string, unknown> }[]).map((r) => [r.key, r.value]));
    return { summary, series, top, signups, maintenance: s.get('maintenance') as { enabled?: boolean; message?: string } | undefined, signupsOpen: (s.get('signups') as { open?: boolean } | undefined)?.open !== false };
  }, onError);
  const days = useMemo(() => {
    const out: string[] = [];
    for (let i = 29; i >= 0; i--) out.push(new Date(Date.now() - i * 864e5).toISOString().slice(0, 10));
    return out;
  }, []);
  if (loading) return <Loader2 className="spin" />;
  if (!data) return null;
  const sm = data.summary;
  const tiles: [string, unknown][] = [['Users', sm.users], ['Pro users', sm.pro], ['Suspended', sm.suspended], ['Conversations', sm.conversations], ['Messages', sm.messages], ['Active (7 days)', sm.activeLast7Days ?? sm.activeLastDays]];
  const perDay = days.map((d) => {
    const rows = data.series.filter((r) => String(r.day).slice(0, 10) === d);
    return { d, total: rows.reduce((a, r) => a + Number(r.n), 0), rows };
  });
  const max = Math.max(1, ...perDay.map((p) => p.total));
  const kinds = [...new Set(data.series.map((r) => r.kind))];
  const signupMap = new Map(data.signups.map((r) => [String(r.day).slice(0, 10), Number(r.n)]));
  const smax = Math.max(1, ...days.map((d) => signupMap.get(d) ?? 0));
  const W = 600;
  const bw = W / days.length;
  return (
    <>
      {(data.maintenance?.enabled || !data.signupsOpen) && (
        <div className="callout warn" role="status">
          <Power size={15} />
          <div className="grow">
            {data.maintenance?.enabled && <div><b>Maintenance mode is ON.</b> Ordinary users cannot use Genius Web right now.</div>}
            {!data.signupsOpen && <div><b>New registrations are closed.</b></div>}
          </div>
          <button className="btn sm" onClick={() => goTo('site')}>Open site controls</button>
        </div>
      )}
      <div className="pj-grid">{tiles.map(([k, v]) => <div key={k} className="pj-card" style={{ cursor: 'default' }}><span className="small muted">{k}</span><h3 style={{ fontSize: 26 }}>{String(v ?? 0)}</h3></div>)}</div>
      <div className="card pad col" style={{ gap: 8 }}>
        <b>Genius feature use, last 30 days</b>
        <svg viewBox={`0 0 ${W} 120`} role="img" aria-label="Events per day by kind" style={{ width: '100%', height: 'auto' }}>
          {perDay.map((p, i) => {
            let y = 110;
            return (
              <g key={p.d}>
                <title>{`${p.d}: ${p.total} event(s)`}</title>
                {kinds.map((k) => {
                  const n = p.rows.filter((r) => r.kind === k).reduce((a, r) => a + Number(r.n), 0);
                  const h = (n / max) * 100;
                  y -= h;
                  return n ? <rect key={k} x={i * bw + 1} y={y} width={bw - 2} height={h} fill={KIND_COLOR[k] ?? 'var(--accent)'} rx={1} /> : null;
                })}
              </g>
            );
          })}
          <line x1="0" y1="110" x2={W} y2="110" stroke="var(--border)" />
        </svg>
        <div className="row wrap small muted">{kinds.length ? kinds.map((k) => <span key={k} className="row" style={{ gap: 4 }}><i style={{ width: 10, height: 10, background: KIND_COLOR[k] ?? 'var(--accent)', borderRadius: 2, display: 'inline-block' }} />{k}</span>) : 'No usage recorded yet.'}</div>
      </div>
      <div className="grid-2" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(300px, 1fr))', gap: 12 }}>
        <div className="card pad col" style={{ gap: 6 }}>
          <b>New accounts per day</b>
          <svg viewBox={`0 0 ${W} 70`} role="img" aria-label="New accounts per day" style={{ width: '100%', height: 'auto' }}>
            {days.map((d, i) => { const n = signupMap.get(d) ?? 0; const h = (n / smax) * 56; return <g key={d}><title>{`${d}: ${n}`}</title><rect x={i * bw + 1} y={62 - h} width={bw - 2} height={Math.max(h, n ? 2 : 0)} fill="var(--ok)" rx={1} /></g>; })}
            <line x1="0" y1="62" x2={W} y2="62" stroke="var(--border)" />
          </svg>
        </div>
        <div className="card pad col" style={{ gap: 6 }}>
          <b>Most active accounts (30 days)</b>
          {data.top.length === 0 ? <span className="faint small">Nothing yet.</span> : data.top.map((t) => <div key={t.user_id} className="row small"><span className="grow ellipsis">{t.email}</span><b>{Number(t.events)}</b></div>)}
        </div>
      </div>
      <p className="small muted">Counts only. Message content is never shown here.</p>
    </>
  );
}

// ------------------------------------------------------------------------------------------ users
const PAGE = 25;

function Users({ onError, onOk }: P) {
  const [q, setQ] = useState('');
  const [term, setTerm] = useState('');
  const [plan, setPlan] = useState('all');
  const [status, setStatus] = useState('all');
  const [sort, setSort] = useState('created');
  const [page, setPage] = useState(0);
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<string | null>(null);
  const { data, loading, reload } = useLoad(() => rpc<UserRow[]>('admin_search_users', { search: term, plan_filter: plan, status_filter: status, sort_by: sort, lim: PAGE, off: page * PAGE }), onError, [term, plan, status, sort, page]);
  const total = Number(data?.[0]?.total_count ?? 0);
  const rows = data ?? [];
  const allOn = rows.length > 0 && rows.every((r) => sel.has(r.id));
  const toggle = (id: string) => setSel((s) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const bulk = async (label: string, fn: () => Promise<unknown>) => {
    if (!window.confirm(`${label} for ${sel.size} selected user(s)?`)) return;
    try {
      await fn();
      onOk(`${label}: done for ${sel.size} user(s).`);
      setSel(new Set());
      await reload();
    } catch (e) {
      onError(errText(e));
    }
  };
  const ids = [...sel];
  return (
    <>
      <form className="row wrap" onSubmit={(e) => { e.preventDefault(); setPage(0); setTerm(q); }}>
        <label className="gx-search grow"><Search size={13} /><input placeholder="Search by email, name or user id" aria-label="Search users" value={q} onChange={(e) => setQ(e.target.value)} /></label>
        <select className="select" aria-label="Filter by plan" value={plan} onChange={(e) => { setPage(0); setPlan(e.target.value); }}><option value="all">All plans</option><option value="free">Free</option><option value="pro">Pro</option></select>
        <select className="select" aria-label="Filter by status" value={status} onChange={(e) => { setPage(0); setStatus(e.target.value); }}><option value="all">Any status</option><option value="active">Active</option><option value="suspended">Suspended</option></select>
        <select className="select" aria-label="Sort" value={sort} onChange={(e) => { setPage(0); setSort(e.target.value); }}><option value="created">Newest</option><option value="activity">Most active</option><option value="seen">Last seen</option><option value="email">Email A–Z</option></select>
        <button className="btn" type="submit">Search</button>
        <button className="btn" type="button" disabled={!rows.length} onClick={() => download('genius-users-page.csv', csvOf(rows.map((r) => ({ id: r.id, email: r.email, name: r.display_name, plan: r.plan, plan_expires: r.plan_expires_at, status: r.status, joined: r.created_at, last_seen: r.last_seen_at, conversations: r.conversations, messages: r.messages, events_30d: r.events_30d }))))}><Download size={13} /> CSV (this page)</button>
      </form>
      {sel.size > 0 && (
        <div className="callout info" role="status">
          <div className="grow"><b>{sel.size} selected.</b> Bulk actions are checked and logged as one audit entry each.</div>
          <button className="btn sm" onClick={() => void bulk('Make Pro', () => rpc('admin_bulk_set_plan', { targets: ids, new_plan: 'pro', until: null, note: 'bulk' }))}>Make Pro</button>
          <button className="btn sm" onClick={() => void bulk('30-day Pro', () => rpc('admin_bulk_set_plan', { targets: ids, new_plan: 'pro', until: new Date(Date.now() + 30 * 864e5).toISOString(), note: 'bulk 30-day' }))}>30-day Pro</button>
          <button className="btn sm" onClick={() => void bulk('Make Free', () => rpc('admin_bulk_set_plan', { targets: ids, new_plan: 'free', until: null, note: 'bulk' }))}>Make Free</button>
          <button className="btn sm ghost" onClick={() => setSel(new Set())}>Clear</button>
        </div>
      )}
      {loading ? <Loader2 className="spin" /> : (
        <div className="card" style={{ overflow: 'auto' }}>
          <table className="md-table" style={{ width: '100%' }}>
            <thead><tr><th style={{ width: 28 }}><input type="checkbox" aria-label="Select all on this page" checked={allOn} onChange={() => setSel(allOn ? new Set() : new Set(rows.map((r) => r.id)))} /></th><th>User</th><th>Plan</th><th>Status</th><th>Activity</th><th /></tr></thead>
            <tbody>
              {rows.map((u) => (
                <tr key={u.id}>
                  <td><input type="checkbox" aria-label={`Select ${u.email}`} checked={sel.has(u.id)} onChange={() => toggle(u.id)} /></td>
                  <td><b>{u.email}</b>{u.is_admin && <span className="badge accent" style={{ marginLeft: 6 }}>owner</span>}<div className="tiny faint">{u.display_name ?? ''} joined {day(u.created_at)} · seen {when(u.last_seen_at)}</div></td>
                  <td>{u.plan}{u.plan_expires_at ? <div className="tiny faint">until {day(u.plan_expires_at)}</div> : null}</td>
                  <td><span className={`badge ${u.status === 'active' ? 'ok' : 'bad'}`}>{u.status}</span></td>
                  <td className="small">{u.conversations} chats · {u.messages} msgs<div className="tiny faint">{Number(u.events_30d)} Genius events / 30 d</div></td>
                  <td><button className="btn sm" onClick={() => setOpen(u.id)}>Manage</button></td>
                </tr>
              ))}
              {rows.length === 0 && <tr><td colSpan={6} className="faint">No users match.</td></tr>}
            </tbody>
          </table>
        </div>
      )}
      <div className="row small muted">
        <span className="grow">{total ? `${page * PAGE + 1}–${Math.min(total, page * PAGE + rows.length)} of ${total}` : ''}</span>
        <button className="btn sm" disabled={page === 0} onClick={() => setPage(page - 1)}>Previous</button>
        <button className="btn sm" disabled={(page + 1) * PAGE >= total} onClick={() => setPage(page + 1)}>Next</button>
      </div>
      {open && <UserDrawer id={open} onClose={() => setOpen(null)} onChanged={() => void reload()} onError={onError} onOk={onOk} />}
    </>
  );
}

function UserDrawer({ id, onClose, onChanged, onError, onOk }: { id: string; onClose: () => void; onChanged: () => void } & P) {
  const { data: d, loading, reload } = useLoad(() => rpc<Detail>('admin_user_detail', { target: id }), onError, [id]);
  const { data: features } = useLoad(async () => ((await sb().from('features').select('*').order('key')).data ?? []) as Feature[], onError);
  const { data: cohorts } = useLoad(async () => ((await sb().from('early_access_cohorts').select('*').order('id')).data ?? []) as Cohort[], onError);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const act = async (label: string, fn: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await fn();
      onOk(label);
      await reload();
      onChanged();
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(false);
    }
  };
  const p = d?.profile;
  const isPro = d?.entitlement?.plan === 'pro' && (!d.entitlement.expiresAt || new Date(d.entitlement.expiresAt) > new Date());
  return (
    <div className="gx-drawer" role="dialog" aria-modal="true" aria-label="User details">
      <div className="gx-drawer-head row"><b className="grow ellipsis">{p?.email ?? 'User'}</b><button className="icon-btn" aria-label="Close" onClick={onClose}><X size={16} /></button></div>
      {loading || !d || !p ? <Loader2 className="spin" /> : (
        <div className="col" style={{ gap: 12 }}>
          <div className="card pad col" style={{ gap: 4 }}>
            <div className="small"><b>{p.displayName ?? '(no name)'}</b> {d.isAdmin && <span className="badge accent">owner</span>} <span className={`badge ${p.status === 'active' ? 'ok' : 'bad'}`}>{p.status}</span></div>
            <div className="tiny faint mono" style={{ userSelect: 'all' }}>{p.id}</div>
            <div className="tiny faint">Joined {day(p.createdAt)} · last seen {when(p.lastSeenAt)} · {d.counts.conversations} chats, {d.counts.messages} messages</div>
            {p.suspendedReason && <div className="small">Suspension reason: {p.suspendedReason}</div>}
          </div>
          <div className="card pad col" style={{ gap: 8 }}>
            <b>Plan</b>
            <div className="small">{isPro ? `Pro${d.entitlement?.expiresAt ? ` until ${day(d.entitlement.expiresAt)}` : ' (no expiry)'}` : 'Free'}{d.entitlement?.note ? ` · ${d.entitlement.note}` : ''}</div>
            <div className="row wrap">
              <button className="btn sm" disabled={busy} onClick={() => void act('Plan set to Pro.', () => rpc('admin_set_plan', { target: id, new_plan: 'pro', until: null, note: 'set by owner' }))}>Pro</button>
              {[7, 30, 90].map((n) => <button key={n} className="btn sm" disabled={busy} onClick={() => void act(`Pro for ${n} days.`, () => rpc('admin_set_plan', { target: id, new_plan: 'pro', until: new Date(Date.now() + n * 864e5).toISOString(), note: `${n}-day trial` }))}>Pro {n} d</button>)}
              <button className="btn sm" disabled={busy} onClick={() => void act('Plan set to Free.', () => rpc('admin_set_plan', { target: id, new_plan: 'free', until: null, note: 'set by owner' }))}>Free</button>
            </div>
          </div>
          <div className="card pad col" style={{ gap: 8 }}>
            <b>Feature grants</b>
            {d.grants.length === 0 ? <span className="faint small">None.</span> : d.grants.map((g) => (
              <div key={g.feature} className="row small"><span className="grow"><b>{g.feature}</b> {g.expiresAt ? `until ${day(g.expiresAt)}` : '(no expiry)'}{g.reason ? ` · ${g.reason}` : ''}</span><button className="btn sm danger" disabled={busy} onClick={() => void act(`Revoked ${g.feature}.`, () => rpc('admin_revoke_feature', { target: id, feat: g.feature }))}>Revoke</button></div>
            ))}
            <div className="row wrap">
              <select className="select" aria-label="Grant a feature" value="" onChange={(e) => e.target.value && void act(`Granted ${e.target.value} for 7 days.`, () => rpc('admin_grant_feature', { target: id, feat: e.target.value, until: new Date(Date.now() + 7 * 864e5).toISOString(), why: '7-day grant' }))}>
                <option value="">Grant a feature for 7 days…</option>
                {(features ?? []).filter((f) => f.min_plan === 'pro').map((f) => <option key={f.key} value={f.key}>{f.name}</option>)}
              </select>
              {d.grants.length > 0 && <button className="btn sm danger" disabled={busy} onClick={() => window.confirm('Revoke every grant of this user?') && void act('All grants revoked.', () => rpc('admin_revoke_all_grants', { target: id }))}>Revoke all</button>}
            </div>
          </div>
          <div className="card pad col" style={{ gap: 8 }}>
            <b>Early-access cohorts</b>
            <div className="row wrap">
              {(cohorts ?? []).length === 0 && <span className="faint small">No cohorts defined yet.</span>}
              {(cohorts ?? []).map((c) => { const inIt = d.cohorts.includes(c.id); return <button key={c.id} className={`btn sm ${inIt ? 'primary' : ''}`} disabled={busy} aria-pressed={inIt} onClick={() => void act(inIt ? `Removed from ${c.name}.` : `Added to ${c.name}.`, () => rpc('admin_cohort_member', { cohort: c.id, target: id, add_member: !inIt }))}>{c.name}</button>; })}
            </div>
          </div>
          <div className="card pad col" style={{ gap: 6 }}>
            <b>Usage, last 30 days</b>
            <div className="small">{Object.keys(d.usage30d).length ? Object.entries(d.usage30d).map(([k, n]) => `${k} ${n}`).join(' · ') : 'No recorded Genius events.'}</div>
          </div>
          <div className="card pad col" style={{ gap: 6 }}>
            <b>Private note</b>
            <textarea className="textarea" rows={3} maxLength={4000} aria-label="Private admin note" value={note ?? d.note} onChange={(e) => setNote(e.target.value)} />
            <div className="row"><button className="btn sm" disabled={busy || note === null} onClick={() => void act('Note saved.', () => rpc('admin_set_note', { target: id, new_note: note ?? '' }))}>Save note</button><span className="tiny faint">Visible to administrators only.</span></div>
          </div>
          {!d.isAdmin && (
            <div className="card pad col" style={{ gap: 6 }}>
              <b>Account</b>
              {p.status === 'active'
                ? <button className="btn danger" disabled={busy} onClick={() => { const why = window.prompt(`Suspend ${p.email}? Reason (shown to the user):`, ''); if (why !== null) void act('Account suspended.', () => rpc('admin_set_status', { target: id, new_status: 'suspended', why })); }}>Suspend account</button>
                : <button className="btn" disabled={busy} onClick={() => void act('Account restored.', () => rpc('admin_set_status', { target: id, new_status: 'active', why: null }))}>Restore account</button>}
            </div>
          )}
          <div className="card pad col" style={{ gap: 4 }}>
            <b>Recent administrative actions on this account</b>
            {d.audit.length === 0 ? <span className="faint small">None.</span> : d.audit.map((a, i) => <div key={i} className="tiny"><span className="faint">{when(a.at)}</span> · {a.action} <span className="mono faint">{JSON.stringify(a.detail)}</span></div>)}
          </div>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------------------------------ features
function Features({ onError, onOk }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const [f, fl, c] = await Promise.all([sb().from('features').select('*').order('key'), sb().from('feature_flags').select('*'), sb().from('early_access_cohorts').select('id, name')]);
    if (f.error) throw f.error;
    if (fl.error) throw fl.error;
    return { features: f.data as Feature[], flags: new Map((fl.data as Flag[]).map((x) => [x.key, x])), cohorts: (c.data ?? []) as { id: string; name: string }[] };
  }, onError);
  const set = async (f: Flag, patch: Partial<Flag>) => {
    const n = { ...f, ...patch };
    try {
      await rpc('admin_set_flag', { feat: n.key, is_enabled: n.enabled, is_kill: n.kill, rollout: n.rollout_percent, cohort: n.cohort_id, why: n.note });
      onOk(`Saved ${n.key}.`);
      await reload();
    } catch (e) {
      onError(errText(e));
    }
  };
  if (loading) return <Loader2 className="spin" />;
  const killed = data?.features.filter((f) => data.flags.get(f.key)?.kill) ?? [];
  return (
    <>
      <p className="small muted">A <b>flag</b> controls whether a feature is deployed (rollout, kill switch, early-access cohort). The user's <b>plan or grant</b> controls whether they may use it. The two are separate.</p>
      {killed.length > 0 && <div className="callout bad" role="status"><Power size={15} /><div>Switched off for everyone: <b>{killed.map((f) => f.name).join(', ')}</b></div></div>}
      <div className="card" style={{ overflow: 'auto' }}>
        <table className="md-table" style={{ width: '100%' }}>
          <thead><tr><th>Feature</th><th>Needs</th><th>Deployed</th><th>Kill switch</th><th>Rollout %</th><th>Cohort</th><th>Note</th></tr></thead>
          <tbody>
            {data?.features.map((f) => {
              const fl = data.flags.get(f.key);
              if (!fl) return null;
              return (
                <tr key={f.key}>
                  <td><b>{f.name}</b><div className="tiny faint">{f.description}</div></td>
                  <td>{f.min_plan}</td>
                  <td><input type="checkbox" aria-label={`${f.name} deployed`} checked={fl.enabled} onChange={(e) => void set(fl, { enabled: e.target.checked })} /></td>
                  <td><input type="checkbox" aria-label={`${f.name} kill switch`} checked={fl.kill} onChange={(e) => { if (!e.target.checked || window.confirm(`Switch off “${f.name}” for EVERYONE right now?`)) void set(fl, { kill: e.target.checked }); }} /></td>
                  <td><input className="input" style={{ width: 70, height: 26 }} type="number" min={0} max={100} aria-label={`${f.name} rollout percent`} defaultValue={fl.rollout_percent} onBlur={(e) => { const v = Math.max(0, Math.min(100, Number(e.target.value))); if (v !== fl.rollout_percent) void set(fl, { rollout_percent: v }); }} /></td>
                  <td><select className="select" style={{ height: 26 }} aria-label={`${f.name} cohort`} value={fl.cohort_id ?? ''} onChange={(e) => void set(fl, { cohort_id: e.target.value || null })}><option value="">— none —</option>{data.cohorts.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}</select></td>
                  <td><input className="input" style={{ height: 26, minWidth: 140 }} aria-label={`${f.name} note`} defaultValue={fl.note ?? ''} placeholder="why" onBlur={(e) => { if (e.target.value !== (fl.note ?? '')) void set(fl, { note: e.target.value || null }); }} /></td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

// ------------------------------------------------------------------------------------------ early access
function Early({ onError, onOk }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const c = await sb().from('early_access_cohorts').select('*').order('id');
    const m = await sb().from('cohort_members').select('cohort_id, user_id');
    const fl = await sb().from('feature_flags').select('key, cohort_id');
    if (c.error) throw c.error;
    return { cohorts: c.data as Cohort[], members: (m.data ?? []) as { cohort_id: string; user_id: string }[], flags: (fl.data ?? []) as { key: string; cohort_id: string | null }[] };
  }, onError);
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [emails, setEmails] = useState<Record<string, string>>({});
  const run = async (label: string, fn: () => Promise<unknown>) => {
    try {
      await fn();
      onOk(label);
      await reload();
    } catch (e) {
      onError(errText(e));
    }
  };
  const addMany = async (cohort: string) => {
    const list = (emails[cohort] ?? '').split(/[\s,;]+/).map((x) => x.trim().toLowerCase()).filter(Boolean);
    if (!list.length) return;
    const found: string[] = [];
    const missing: string[] = [];
    for (const em of list.slice(0, 100)) {
      const users = await rpc<UserRow[]>('admin_search_users', { search: em, plan_filter: 'all', status_filter: 'all', sort_by: 'email', lim: 5, off: 0 });
      const u = users.find((x) => x.email.toLowerCase() === em);
      if (u) found.push(u.id); else missing.push(em);
    }
    await run(`Added ${found.length} member(s)${missing.length ? `; not found: ${missing.join(', ')}` : ''}.`, async () => {
      if (found.length) await rpc('admin_bulk_cohort', { cohort, targets: found, add_members: true });
      setEmails({ ...emails, [cohort]: '' });
    });
  };
  if (loading) return <Loader2 className="spin" />;
  return (
    <>
      <p className="small muted">An early-access cohort is a named group. Point a feature's flag at a cohort (Features tab; set rollout to 0 %) to open it only to the members.</p>
      <form className="row wrap" onSubmit={(e) => { e.preventDefault(); void run('Cohort saved.', async () => { await rpc('admin_save_cohort', { cohort: id, cohort_name: name, cohort_desc: '' }); setId(''); setName(''); }); }}>
        <input className="input" placeholder="id (e.g. beta)" aria-label="Cohort id" pattern="[a-z0-9_-]{2,40}" value={id} onChange={(e) => setId(e.target.value)} required />
        <input className="input" placeholder="Name" aria-label="Cohort name" value={name} onChange={(e) => setName(e.target.value)} required />
        <button className="btn primary" type="submit">Save cohort</button>
      </form>
      {data?.cohorts.map((c) => (
        <div key={c.id} className="card pad col" style={{ gap: 6 }}>
          <div className="row"><b className="grow">{c.name} <span className="faint small">({c.id})</span></b><span className="small muted">{data.members.filter((m) => m.cohort_id === c.id).length} member(s)</span>
            <button className="btn sm danger" aria-label={`Delete cohort ${c.id}`} onClick={() => window.confirm(`Delete cohort “${c.name}”? Features pointing at it become unrestricted by cohort.`) && void run('Cohort deleted.', () => rpc('admin_delete_cohort', { cohort: c.id }))}><Trash2 size={12} /></button></div>
          <div className="small muted">Used by: {data.flags.filter((f) => f.cohort_id === c.id).map((f) => f.key).join(', ') || 'no feature yet'}</div>
          <div className="row"><input className="input grow" placeholder="Add members by exact email (separate several with commas or spaces)" aria-label={`Add members to ${c.id}`} value={emails[c.id] ?? ''} onChange={(e) => setEmails({ ...emails, [c.id]: e.target.value })} /><button className="btn" disabled={!(emails[c.id] ?? '').trim()} onClick={() => void addMany(c.id).catch((e) => onError(errText(e)))}>Add</button></div>
        </div>
      ))}
    </>
  );
}

// ------------------------------------------------------------------------------------------ limits
const LIMIT_FIELDS: [keyof Limits, string, string][] = [
  ['maxModelCalls', 'Model calls per task', 'Agent/mission steps that call the model'],
  ['maxToolCalls', 'Tool calls per task', 'Searches, file reads, calculations…'],
  ['maxTaskMinutes', 'Minutes per task', 'Wall-clock limit of one task'],
  ['maxConcurrentTasks', 'Tasks at the same time', 'Parallel tasks per user'],
  ['maxRetries', 'Retries per step', 'Bounded recovery attempts'],
  ['contextChars', 'Context size (characters)', 'How much history/files a request may carry'],
  ['dailyAgentMissions', 'Agent missions per day', 'Enforced when the use is recorded (24 h window)'],
  ['dailyResearch', 'Research reports per day', 'Enforced when the use is recorded (24 h window)'],
  ['dailyWebToolCalls', 'Web searches / page reads per day', 'Enforced when the use is recorded (24 h window)'],
];

function LimitsTab({ onError, onOk }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const r = await sb().from('app_settings').select('key, value').in('key', ['limits_free', 'limits_pro']);
    if (r.error) throw r.error;
    const m = new Map((r.data as { key: string; value: Limits }[]).map((x) => [x.key, x.value]));
    return { free: m.get('limits_free') ?? ({} as Limits), pro: m.get('limits_pro') ?? ({} as Limits) };
  }, onError);
  const [draft, setDraft] = useState<{ free: Partial<Record<keyof Limits, string>>; pro: Partial<Record<keyof Limits, string>> } | null>(null);
  useEffect(() => {
    if (!data) return;
    const to = (l: Limits) => Object.fromEntries(LIMIT_FIELDS.map(([k]) => [k, l[k] == null ? '' : String(l[k])]));
    setDraft({ free: to(data.free), pro: to(data.pro) });
  }, [data]);
  if (loading || !draft) return <Loader2 className="spin" />;
  const save = async (plan: 'free' | 'pro') => {
    const obj: Record<string, number | null> = {};
    for (const [k] of LIMIT_FIELDS) {
      const raw = (draft[plan][k] ?? '').trim();
      if (raw === '') obj[k] = null;
      else {
        const n = Number(raw);
        if (!Number.isFinite(n) || n < 0) return onError(`“${k}” must be a number of 0 or more, or empty for no cap.`);
        obj[k] = Math.floor(n);
      }
    }
    try {
      await rpc('admin_set_setting', { setting: `limits_${plan}`, new_value: obj, why: 'edited in admin panel' });
      onOk(`${plan === 'free' ? 'Free' : 'Pro'} limits saved.`);
      await reload();
    } catch (e) {
      onError(errText(e));
    }
  };
  return (
    <>
      <p className="small muted">These are <b>Genius</b> limits: they cap tasks, tools and missions inside Genius. They never change what a user's AI provider charges (every user brings their own key). Leave a field empty for “no cap”. Daily caps are enforced where the use is recorded; per-task budgets are applied by the app and are not a defence against a modified client.</p>
      <div className="grid-2" style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(340px, 1fr))', gap: 12 }}>
        {(['free', 'pro'] as const).map((plan) => (
          <div key={plan} className="card pad col" style={{ gap: 8 }}>
            <b>{plan === 'free' ? 'Free plan' : 'Pro plan'}</b>
            {LIMIT_FIELDS.map(([k, label, hint]) => (
              <label key={k} className="col" style={{ gap: 2 }}>
                <span className="small">{label} <span className="faint tiny">— {hint}</span></span>
                <input className="input" inputMode="numeric" placeholder="no cap" aria-label={`${plan} ${label}`} value={draft[plan][k] ?? ''} onChange={(e) => setDraft({ ...draft, [plan]: { ...draft[plan], [k]: e.target.value } })} />
              </label>
            ))}
            <div className="row"><button className="btn primary" onClick={() => void save(plan)}>Save {plan} limits</button></div>
          </div>
        ))}
      </div>
    </>
  );
}

// ------------------------------------------------------------------------------------------ site controls
function Site({ onError, onOk }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const [s, a] = await Promise.all([sb().from('app_settings').select('key, value').in('key', ['maintenance', 'signups']), rpc<Announcement[]>('admin_list_announcements')]);
    if (s.error) throw s.error;
    const m = new Map((s.data as { key: string; value: Record<string, unknown> }[]).map((x) => [x.key, x.value]));
    return { maintenance: (m.get('maintenance') ?? { enabled: false, message: '' }) as { enabled: boolean; message: string }, signupsOpen: (m.get('signups') as { open?: boolean } | undefined)?.open !== false, announcements: a };
  }, onError);
  const [msg, setMsg] = useState<string | null>(null);
  const [form, setForm] = useState({ id: '' as string, title: '', body: '', level: 'info' as Announcement['level'], active: true, endsDays: '' });
  const run = async (label: string, fn: () => Promise<unknown>) => {
    try {
      await fn();
      onOk(label);
      await reload();
    } catch (e) {
      onError(errText(e));
    }
  };
  if (loading || !data) return <Loader2 className="spin" />;
  const m = data.maintenance;
  return (
    <>
      <div className="card pad col" style={{ gap: 8 }}>
        <div className="row"><Power size={15} /><b className="grow">Maintenance mode</b><span className={`badge ${m.enabled ? 'bad' : 'ok'}`}>{m.enabled ? 'ON' : 'off'}</span></div>
        <p className="small muted" style={{ margin: 0 }}>While on, every feature reports “maintenance” for ordinary users and the app shows your message. Administrators keep working so they can switch it off. Already running tasks are not killed.</p>
        <input className="input" aria-label="Maintenance message" placeholder="Message shown to users (e.g. Back at 18:00 UTC)" value={msg ?? m.message} onChange={(e) => setMsg(e.target.value)} maxLength={300} />
        <div className="row">
          <button className={`btn ${m.enabled ? '' : 'danger'}`} onClick={() => (m.enabled || window.confirm('Switch Genius Web off for all ordinary users?')) && void run(m.enabled ? 'Maintenance ended.' : 'Maintenance started.', () => rpc('admin_set_setting', { setting: 'maintenance', new_value: { enabled: !m.enabled, message: (msg ?? m.message).trim() }, why: 'maintenance toggle' }))}>{m.enabled ? 'End maintenance' : 'Start maintenance'}</button>
          {m.enabled && <button className="btn" onClick={() => void run('Message updated.', () => rpc('admin_set_setting', { setting: 'maintenance', new_value: { enabled: true, message: (msg ?? m.message).trim() }, why: 'message' }))}>Update message</button>}
        </div>
      </div>
      <div className="card pad col" style={{ gap: 8 }}>
        <div className="row"><b className="grow">New registrations</b><span className={`badge ${data.signupsOpen ? 'ok' : 'bad'}`}>{data.signupsOpen ? 'open' : 'closed'}</span></div>
        <p className="small muted" style={{ margin: 0 }}>Closing registrations rejects new sign-ups at the database. Existing accounts keep working.</p>
        <div className="row"><button className="btn" onClick={() => void run(data.signupsOpen ? 'Registrations closed.' : 'Registrations opened.', () => rpc('admin_set_setting', { setting: 'signups', new_value: { open: !data.signupsOpen }, why: 'signups toggle' }))}>{data.signupsOpen ? 'Close registrations' : 'Open registrations'}</button></div>
      </div>
      <div className="card pad col" style={{ gap: 8 }}>
        <div className="row"><Megaphone size={15} /><b>Announcements</b></div>
        <form className="col" style={{ gap: 6 }} onSubmit={(e) => { e.preventDefault(); void run('Announcement saved.', async () => { await rpc('admin_save_announcement', { ann_id: form.id || null, ann_title: form.title, ann_body: form.body, ann_level: form.level, ann_active: form.active, ann_ends: form.endsDays ? new Date(Date.now() + Number(form.endsDays) * 864e5).toISOString() : null }); setForm({ id: '', title: '', body: '', level: 'info', active: true, endsDays: '' }); }); }}>
          <input className="input" placeholder="Title" aria-label="Announcement title" value={form.title} maxLength={120} onChange={(e) => setForm({ ...form, title: e.target.value })} required />
          <textarea className="textarea" rows={2} placeholder="Details (optional)" aria-label="Announcement text" value={form.body} maxLength={2000} onChange={(e) => setForm({ ...form, body: e.target.value })} />
          <div className="row wrap">
            <select className="select" aria-label="Level" value={form.level} onChange={(e) => setForm({ ...form, level: e.target.value as Announcement['level'] })}><option value="info">Info</option><option value="warning">Warning</option><option value="critical">Critical</option></select>
            <label className="row small"><input type="checkbox" checked={form.active} onChange={(e) => setForm({ ...form, active: e.target.checked })} /> Show now</label>
            <input className="input" style={{ width: 150 }} inputMode="numeric" placeholder="Hide after N days" aria-label="Hide after days" value={form.endsDays} onChange={(e) => setForm({ ...form, endsDays: e.target.value.replace(/\D/g, '') })} />
            <button className="btn primary" type="submit">{form.id ? 'Update' : 'Publish'}</button>
            {form.id && <button className="btn ghost" type="button" onClick={() => setForm({ id: '', title: '', body: '', level: 'info', active: true, endsDays: '' })}>Cancel edit</button>}
          </div>
        </form>
        {data.announcements.map((a) => (
          <div key={a.id} className="row small" style={{ borderTop: '1px solid var(--border)', paddingTop: 6 }}>
            <span className={`badge ${a.level === 'critical' ? 'bad' : a.level === 'warning' ? 'warn' : 'accent'}`}>{a.level}</span>
            <span className="grow"><b>{a.title}</b> <span className="faint">{a.active ? '' : '(hidden)'} {a.ends_at ? `until ${day(a.ends_at)}` : ''}</span></span>
            <button className="btn sm" onClick={() => setForm({ id: a.id, title: a.title, body: a.body, level: a.level, active: a.active, endsDays: '' })}>Edit</button>
            <button className="btn sm danger" aria-label={`Delete ${a.title}`} onClick={() => window.confirm('Delete this announcement?') && void run('Announcement deleted.', () => rpc('admin_delete_announcement', { ann_id: a.id }))}><Trash2 size={12} /></button>
          </div>
        ))}
        {data.announcements.length === 0 && <span className="faint small">No announcements.</span>}
      </div>
    </>
  );
}

// ------------------------------------------------------------------------------------------ audit
function AuditLog({ onError }: P) {
  const [action, setAction] = useState('');
  const [user, setUser] = useState('');
  const [limit, setLimit] = useState(100);
  const { data, loading } = useLoad(async () => {
    let query = sb().from('admin_audit').select('*').order('id', { ascending: false }).limit(limit);
    if (action.trim()) query = query.ilike('action', `%${action.trim()}%`);
    if (/^[0-9a-f-]{36}$/i.test(user.trim())) query = query.eq('target_user', user.trim());
    const { data, error } = await query;
    if (error) throw error;
    return data as Audit[];
  }, onError, [action, user, limit]);
  return (
    <>
      <div className="row wrap">
        <input className="input" placeholder="Filter by action (e.g. set_plan)" aria-label="Filter audit by action" value={action} onChange={(e) => setAction(e.target.value)} />
        <input className="input" style={{ minWidth: 300 }} placeholder="Target user id" aria-label="Filter audit by user id" value={user} onChange={(e) => setUser(e.target.value)} />
        <button className="btn" disabled={!data?.length} onClick={() => download('genius-audit.csv', csvOf((data ?? []).map((a) => ({ id: a.id, at: a.at, actor: a.actor, action: a.action, target_user: a.target_user, detail: a.detail }))))}><Download size={13} /> CSV</button>
      </div>
      {loading ? <Loader2 className="spin" /> : (
        <div className="card" style={{ overflow: 'auto' }}>
          <table className="md-table" style={{ width: '100%' }}>
            <thead><tr><th>When (UTC)</th><th>Action</th><th>Target</th><th>Detail</th></tr></thead>
            <tbody>
              {(data ?? []).map((a) => <tr key={a.id}><td className="small">{a.at.slice(0, 19).replace('T', ' ')}</td><td>{a.action}</td><td className="tiny mono">{a.target_user?.slice(0, 8) ?? '—'}</td><td className="tiny mono">{JSON.stringify(a.detail)}</td></tr>)}
              {data?.length === 0 && <tr><td colSpan={4} className="faint">No matching administrative actions.</td></tr>}
            </tbody>
          </table>
        </div>
      )}
      {data && data.length >= limit && <div className="row"><button className="btn" onClick={() => setLimit(limit + 200)}>Load more</button></div>}
    </>
  );
}

// ------------------------------------------------------------------------------------------ system
function System({ onError }: P) {
  const access = useAuth((s) => s.access);
  const agent = featureOf(access, 'agent_mode');
  const { data, loading, reload } = useLoad(async () => {
    const t0 = performance.now();
    const admins = await rpc<{ user_id: string; email: string; since: string }[]>('admin_list_admins');
    const dbMs = Math.round(performance.now() - t0);
    const web = await fetch('/api/web', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' }).then((r) => ({ status: r.status })).catch(() => ({ status: 0 }));
    const relay = await fetch('/api/relay', { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' }).then((r) => ({ status: r.status })).catch(() => ({ status: 0 }));
    return { admins, dbMs, web, relay };
  }, onError);
  const reach = (s: number) => (s === 0 ? <span className="badge bad">unreachable</span> : <span className="badge ok">reachable (HTTP {s}; an error status is expected for an empty test request)</span>);
  return (
    <div className="card pad col" style={{ gap: 8 }}>
      <div className="row"><b className="grow">This deployment</b><button className="btn sm" onClick={() => void reload()}>Re-check</button></div>
      {loading ? <Loader2 className="spin" /> : (
        <dl className="am-facts">
          <dt>Accounts</dt><dd>Supabase ({(import.meta.env.VITE_SUPABASE_URL as string | undefined)?.replace(/^https:\/\//, '') ?? 'not configured'}) — database answered in {data?.dbMs} ms</dd>
          <dt>Administrators</dt><dd>{data?.admins.map((a) => a.email).join(', ') || 'none'} <span className="faint small">(read-only here; changed only in the SQL editor — see docs/ADMIN-BOOTSTRAP.md)</span></dd>
          <dt>Your plan</dt><dd>{access?.plan}</dd>
          <dt>Agent Mode for you</dt><dd>{agent.available ? 'available' : agent.reason}</dd>
          <dt>Web proxy</dt><dd>/api/web {data && reach(data.web.status)}</dd>
          <dt>Provider relay</dt><dd>/api/relay {data && reach(data.relay.status)}</dd>
          <dt>Provider keys</dt><dd>Never stored by Genius: each user keeps theirs in their own browser (session-only or encrypted on their device) or in the desktop credential store. Nothing to rotate or view here.</dd>
          <dt>Email confirmation</dt><dd>Set in Supabase (Authentication → Sign In / Providers → Email → “Confirm email”). It cannot be read or changed from this page.</dd>
        </dl>
      )}
      <p className="small muted" style={{ margin: 0 }}>Provider health is checked by each user in Settings → AI provider → Test connection, because every user brings their own key.</p>
    </div>
  );
}
