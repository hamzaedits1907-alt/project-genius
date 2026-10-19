// Owner admin panel. Every action calls a SECURITY DEFINER function in the database that checks the
// caller's administrator status itself and writes an audit record; this screen is only a front end for
// them. A person who reaches this page without being an administrator gets errors from the database,
// not data.
import { useCallback, useEffect, useState } from 'react';
import { Loader2, Search, ShieldAlert } from 'lucide-react';
import { sb, useAuth, featureOf } from './auth';

interface UserRow {
  id: string;
  email: string;
  display_name: string | null;
  status: 'active' | 'suspended';
  plan: 'free' | 'pro';
  plan_expires_at: string | null;
  created_at: string;
  conversations: number;
  messages: number;
  is_admin: boolean;
}
interface Flag { key: string; enabled: boolean; kill: boolean; rollout_percent: number; cohort_id: string | null; note: string | null }
interface Feature { key: string; name: string; description: string; min_plan: string }
interface Cohort { id: string; name: string; description: string }
interface Audit { id: number; at: string; actor: string; action: string; target_user: string | null; detail: Record<string, unknown> }

type Tab = 'overview' | 'users' | 'features' | 'early' | 'audit' | 'system';
const errText = (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));

export default function AdminPanel() {
  const access = useAuth((s) => s.access);
  const [tab, setTab] = useState<Tab>('overview');
  const [err, setErr] = useState<string | null>(null);
  if (!access) return <div className="pj-root"><div className="pj-inner"><Loader2 className="spin" /> Checking your access…</div></div>;
  if (!access.isAdmin) {
    return (
      <div className="pj-root"><div className="pj-inner">
        <div className="callout bad" role="alert"><ShieldAlert size={16} /> <div><b>Not authorized.</b> This area is only for the owner of this Genius Web deployment.</div></div>
      </div></div>
    );
  }
  const tabs: [Tab, string][] = [['overview', 'Overview'], ['users', 'Users'], ['features', 'Features & flags'], ['early', 'Early access'], ['audit', 'Audit log'], ['system', 'System']];
  return (
    <div className="pj-root">
      <div className="pj-inner">
        <header className="pj-head"><div className="grow"><h1>Owner admin</h1><p className="muted" style={{ margin: '4px 0 0' }}>Every change here is checked by the database and written to the audit log.</p></div></header>
        <div className="tabs" role="tablist" aria-label="Admin sections" style={{ height: 40, borderBottom: '1px solid var(--border)' }}>
          {tabs.map(([k, l]) => <button key={k} role="tab" aria-selected={tab === k} className={`tab ${tab === k ? 'on' : ''}`} onClick={() => setTab(k)}>{l}</button>)}
        </div>
        {err && <div className="callout bad" role="alert">{err}</div>}
        {tab === 'overview' && <Overview onError={setErr} />}
        {tab === 'users' && <Users onError={setErr} />}
        {tab === 'features' && <Features onError={setErr} />}
        {tab === 'early' && <Early onError={setErr} />}
        {tab === 'audit' && <AuditLog onError={setErr} />}
        {tab === 'system' && <System />}
      </div>
    </div>
  );
}

type P = { onError: (e: string | null) => void };

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

function Overview({ onError }: P) {
  const { data, loading } = useLoad(async () => {
    const { data, error } = await sb().rpc('admin_usage_summary');
    if (error) throw error;
    return data as Record<string, unknown>;
  }, onError);
  if (loading) return <Loader2 className="spin" />;
  if (!data) return null;
  const events = (data.eventsLast7Days ?? {}) as Record<string, number>;
  const tiles: [string, unknown][] = [['Users', data.users], ['Pro users', data.pro], ['Suspended', data.suspended], ['Conversations', data.conversations], ['Messages', data.messages], ['Active (7 days)', data.activeLastDays ?? data.activeLast7Days]];
  return (
    <>
      <div className="pj-grid">{tiles.map(([k, v]) => <div key={k} className="pj-card" style={{ cursor: 'default' }}><span className="small muted">{k}</span><h3 style={{ fontSize: 26 }}>{String(v ?? 0)}</h3></div>)}</div>
      <p className="small muted">Last 7 days: {Object.keys(events).length ? Object.entries(events).map(([k, n]) => `${k} ${n}`).join(' · ') : 'no usage recorded yet'}. Counts only — no message content is shown here.</p>
    </>
  );
}

function Users({ onError }: P) {
  const [q, setQ] = useState('');
  const [term, setTerm] = useState('');
  const { data, loading, reload } = useLoad(async () => {
    const { data, error } = await sb().rpc('admin_list_users', { search: term, lim: 100 });
    if (error) throw error;
    return data as UserRow[];
  }, onError, [term]);
  const { data: features } = useLoad(async () => ((await sb().from('features').select('*')).data ?? []) as Feature[], onError);
  const [busy, setBusy] = useState<string | null>(null);
  const act = async (id: string, fn: () => PromiseLike<{ error: unknown }>) => {
    setBusy(id);
    try {
      const { error } = await fn();
      if (error) throw error;
      await reload();
      onError(null);
    } catch (e) {
      onError(errText(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <>
      <form className="row" onSubmit={(e) => { e.preventDefault(); setTerm(q); }}>
        <label className="gx-search grow"><Search size={13} /><input placeholder="Search by email or name" aria-label="Search users" value={q} onChange={(e) => setQ(e.target.value)} /></label>
        <button className="btn" type="submit">Search</button>
      </form>
      {loading ? <Loader2 className="spin" /> : (
        <div className="card" style={{ overflow: 'auto' }}>
          <table className="md-table" style={{ width: '100%' }}>
            <thead><tr><th>User</th><th>Plan</th><th>Status</th><th>Usage</th><th>Actions</th></tr></thead>
            <tbody>
              {(data ?? []).map((u) => (
                <tr key={u.id}>
                  <td><b>{u.email}</b>{u.is_admin && <span className="badge accent" style={{ marginLeft: 6 }}>owner</span>}<div className="tiny faint">{u.display_name ?? ''} joined {u.created_at.slice(0, 10)}</div></td>
                  <td>{u.plan}{u.plan_expires_at ? <div className="tiny faint">until {u.plan_expires_at.slice(0, 10)}</div> : null}</td>
                  <td><span className={`badge ${u.status === 'active' ? 'ok' : 'bad'}`}>{u.status}</span></td>
                  <td className="small">{u.conversations} chats · {u.messages} msgs</td>
                  <td>
                    <div className="row wrap" style={{ gap: 4 }}>
                      {u.plan === 'free' ? (
                        <>
                          <button className="btn sm" disabled={busy === u.id} onClick={() => void act(u.id, () => sb().rpc('admin_set_plan', { target: u.id, new_plan: 'pro', until: null, note: 'set by owner' }))}>Make Pro</button>
                          <button className="btn sm" disabled={busy === u.id} onClick={() => void act(u.id, () => sb().rpc('admin_set_plan', { target: u.id, new_plan: 'pro', until: new Date(Date.now() + 30 * 864e5).toISOString(), note: '30-day trial' }))}>30-day Pro</button>
                        </>
                      ) : <button className="btn sm" disabled={busy === u.id} onClick={() => void act(u.id, () => sb().rpc('admin_set_plan', { target: u.id, new_plan: 'free', until: null, note: 'set by owner' }))}>Make Free</button>}
                      <select className="select" style={{ height: 26, width: 140 }} aria-label={`Grant a feature to ${u.email}`} value="" onChange={(e) => e.target.value && void act(u.id, () => sb().rpc('admin_grant_feature', { target: u.id, feat: e.target.value, until: new Date(Date.now() + 7 * 864e5).toISOString(), why: '7-day grant' }))}>
                        <option value="">Grant 7 days…</option>
                        {(features ?? []).filter((f) => f.min_plan === 'pro').map((f) => <option key={f.key} value={f.key}>{f.name}</option>)}
                      </select>
                      {!u.is_admin && (u.status === 'active'
                        ? <button className="btn sm danger" disabled={busy === u.id} onClick={() => { const why = window.prompt(`Suspend ${u.email}? Reason (shown to the user):`, ''); if (why !== null) void act(u.id, () => sb().rpc('admin_set_status', { target: u.id, new_status: 'suspended', why })); }}>Suspend</button>
                        : <button className="btn sm" disabled={busy === u.id} onClick={() => void act(u.id, () => sb().rpc('admin_set_status', { target: u.id, new_status: 'active', why: null }))}>Restore</button>)}
                    </div>
                  </td>
                </tr>
              ))}
              {data?.length === 0 && <tr><td colSpan={5} className="faint">No users match.</td></tr>}
            </tbody>
          </table>
        </div>
      )}
    </>
  );
}

function Features({ onError }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const [f, fl] = await Promise.all([sb().from('features').select('*').order('key'), sb().from('feature_flags').select('*')]);
    if (f.error) throw f.error;
    if (fl.error) throw fl.error;
    return { features: f.data as Feature[], flags: new Map((fl.data as Flag[]).map((x) => [x.key, x])) };
  }, onError);
  const set = async (f: Flag, patch: Partial<Flag>) => {
    const n = { ...f, ...patch };
    const { error } = await sb().rpc('admin_set_flag', { feat: n.key, is_enabled: n.enabled, is_kill: n.kill, rollout: n.rollout_percent, cohort: n.cohort_id, why: n.note });
    if (error) onError(errText(error));
    else {
      onError(null);
      await reload();
    }
  };
  if (loading) return <Loader2 className="spin" />;
  return (
    <>
      <p className="small muted">A <b>flag</b> controls whether a feature is deployed (rollout, kill switch). The user's <b>plan or grant</b> controls whether they may use it. The two are separate.</p>
      <div className="card" style={{ overflow: 'auto' }}>
        <table className="md-table" style={{ width: '100%' }}>
          <thead><tr><th>Feature</th><th>Needs</th><th>Deployed</th><th>Kill switch</th><th>Rollout %</th></tr></thead>
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
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </>
  );
}

function Early({ onError }: P) {
  const { data, loading, reload } = useLoad(async () => {
    const c = await sb().from('early_access_cohorts').select('*').order('id');
    const m = await sb().from('cohort_members').select('cohort_id, user_id');
    const fl = await sb().from('feature_flags').select('key, cohort_id');
    if (c.error) throw c.error;
    return { cohorts: c.data as Cohort[], members: (m.data ?? []) as { cohort_id: string; user_id: string }[], flags: (fl.data ?? []) as { key: string; cohort_id: string | null }[] };
  }, onError);
  const [id, setId] = useState('');
  const [name, setName] = useState('');
  const [email, setEmail] = useState('');
  const [target, setTarget] = useState('');
  const run = async (fn: () => PromiseLike<{ error: unknown }>) => {
    const { error } = await fn();
    if (error) onError(errText(error));
    else {
      onError(null);
      await reload();
    }
  };
  const addMember = async () => {
    const { data: users, error } = await sb().rpc('admin_list_users', { search: email, lim: 5 });
    if (error) return onError(errText(error));
    const u = (users as UserRow[]).find((x) => x.email.toLowerCase() === email.trim().toLowerCase());
    if (!u) return onError('No user with exactly that email.');
    await run(() => sb().rpc('admin_cohort_member', { cohort: target, target: u.id, add_member: true }));
    setEmail('');
  };
  if (loading) return <Loader2 className="spin" />;
  return (
    <>
      <p className="small muted">An early-access cohort is a named group. Point a feature's flag at a cohort (Features tab → rollout 0 %) to open it only to the members.</p>
      <form className="row wrap" onSubmit={(e) => { e.preventDefault(); void run(() => sb().rpc('admin_save_cohort', { cohort: id, cohort_name: name, cohort_desc: '' })).then(() => { setId(''); setName(''); }); }}>
        <input className="input" placeholder="id (e.g. beta)" aria-label="Cohort id" pattern="[a-z0-9_-]{2,40}" value={id} onChange={(e) => setId(e.target.value)} required />
        <input className="input" placeholder="Name" aria-label="Cohort name" value={name} onChange={(e) => setName(e.target.value)} required />
        <button className="btn primary" type="submit">Save cohort</button>
      </form>
      {data?.cohorts.map((c) => (
        <div key={c.id} className="card pad col" style={{ gap: 6 }}>
          <div className="row"><b className="grow">{c.name} <span className="faint small">({c.id})</span></b><span className="small muted">{data.members.filter((m) => m.cohort_id === c.id).length} member(s)</span></div>
          <div className="small muted">Used by: {data.flags.filter((f) => f.cohort_id === c.id).map((f) => f.key).join(', ') || 'no feature yet'}</div>
          <div className="row"><input className="input grow" placeholder="Add a member by exact email" aria-label={`Add member to ${c.id}`} value={target === c.id ? email : ''} onFocus={() => setTarget(c.id)} onChange={(e) => { setTarget(c.id); setEmail(e.target.value); }} /><button className="btn" disabled={target !== c.id || !email} onClick={() => void addMember()}>Add</button></div>
        </div>
      ))}
    </>
  );
}

function AuditLog({ onError }: P) {
  const { data, loading } = useLoad(async () => {
    const { data, error } = await sb().from('admin_audit').select('*').order('id', { ascending: false }).limit(200);
    if (error) throw error;
    return data as Audit[];
  }, onError);
  if (loading) return <Loader2 className="spin" />;
  return (
    <div className="card" style={{ overflow: 'auto' }}>
      <table className="md-table" style={{ width: '100%' }}>
        <thead><tr><th>When</th><th>Action</th><th>Target</th><th>Detail</th></tr></thead>
        <tbody>
          {(data ?? []).map((a) => <tr key={a.id}><td className="small">{a.at.slice(0, 19).replace('T', ' ')}</td><td>{a.action}</td><td className="tiny mono">{a.target_user?.slice(0, 8) ?? '—'}</td><td className="tiny mono">{JSON.stringify(a.detail)}</td></tr>)}
          {data?.length === 0 && <tr><td colSpan={4} className="faint">No administrative actions yet.</td></tr>}
        </tbody>
      </table>
    </div>
  );
}

function System() {
  const access = useAuth((s) => s.access);
  const agent = featureOf(access, 'agent_mode');
  return (
    <div className="card pad col" style={{ gap: 8 }}>
      <b>This deployment</b>
      <dl className="am-facts">
        <dt>Accounts</dt><dd>Supabase ({(import.meta.env.VITE_SUPABASE_URL as string | undefined)?.replace(/^https:\/\//, '') ?? 'not configured'})</dd>
        <dt>Your plan</dt><dd>{access?.plan}</dd>
        <dt>Agent Mode for you</dt><dd>{agent.available ? 'available' : agent.reason}</dd>
        <dt>Web proxy</dt><dd>/api/web (search and page reading, checks sign-in and kill switches)</dd>
        <dt>Provider relay</dt><dd>/api/relay (NVIDIA and custom endpoints; stores nothing)</dd>
        <dt>Provider keys</dt><dd>Never stored: they stay in each user's browser tab.</dd>
      </dl>
      <p className="small muted" style={{ margin: 0 }}>Provider health is checked per user in Settings → AI provider → Test connection, because every user brings their own key.</p>
    </div>
  );
}
