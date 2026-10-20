// The Genius Web shell: account gate → runtime → the same App as the desktop, with URL routes
// (/app/chat, /app/research, /app/code, /app/agents, /app/projects, /app/settings, /admin).
import { useEffect, useMemo, useState } from 'react';
import { Loader2, Megaphone, X } from 'lucide-react';
import App from '../App';
import { setAccountView } from '../components/AccountBlock';
import { extraViews } from '../lib/extraViews';
import { setFeatureResolver } from '../lib/access';
import { useApp, type Route } from '../store/app';
import AdminPanel from './AdminPanel';
import AuthScreen from './AuthScreen';
import AccountSettings from './AccountSettings';
import CodeView from './CodeView';
import { accessToken, localOnly, sb, supabaseConfigured, useAuth, type Announcement } from './auth';
import { apiBase, isMobileApp } from '../lib/platform';
import { setRuntime } from './context';
import { MemoryStore } from './store';
import { SupabaseStore } from './supabaseStore';
import { createKeyVault } from './keyvault';
import { WebRuntime } from './runtime';
import { startLiveRefresh } from './liveSync';
import { keyHooks } from '../lib/api';
import { onKeyRemoved, onKeySaved, startKeySync, stopKeySync } from './keySync';
import { webLocalKeys } from './keyPorts';
import { createNativeKeyVault, isNativeApp } from './nativeKeys';
import { noWebTools, proxyTools } from './webTools';

extraViews.code = CodeView;
extraViews.admin = AdminPanel;
extraViews.account = AccountSettings;

const PATHS: Record<string, string> = { chat: '/app/chat', research: '/app/research', code: '/app/code', agents: '/app/agents', projects: '/app/projects', settings: '/app/settings', admin: '/admin' };
const routeToPath = (r: Route) => PATHS[r.name] ?? '/app/chat';
function pathToRoute(path: string): Route {
  const seg = path.replace(/\/+$/, '').split('/').filter(Boolean);
  if (seg[0] === 'admin') return { name: 'admin' };
  const name = seg[1];
  if (name === 'research' || name === 'code' || name === 'agents' || name === 'projects' || name === 'settings') return { name } as Route;
  return { name: 'chat' };
}

function Notice({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <main className="au-root">
      <div className="au-card">
        <h1>{title}</h1>
        {children}
      </div>
    </main>
  );
}

// A selector must return the SAME value while nothing changed: a fresh [] on every call makes zustand v5 loop
// forever (React error #185, a blank page right after sign-in on the deployed site).
const NO_ANNOUNCEMENTS: Announcement[] = [];

export function Announcements() {
  const items = useAuth((s) => s.context?.announcements ?? NO_ANNOUNCEMENTS);
  const [hidden, setHidden] = useState<string[]>(() => {
    try {
      return JSON.parse(sessionStorage.getItem('genius.announce.hidden') ?? '[]') as string[];
    } catch {
      return [];
    }
  });
  const shown = items.filter((a) => !hidden.includes(a.id)).slice(0, 2);
  if (!shown.length) return null;
  const hide = (id: string) => {
    const next = [...hidden, id];
    setHidden(next);
    try {
      sessionStorage.setItem('genius.announce.hidden', JSON.stringify(next));
    } catch {
      /* hidden for this render only */
    }
  };
  return (
    <div className="gx-announce" role="region" aria-label="Announcements">
      {shown.map((a) => (
        <div key={a.id} className={`callout ${a.level === 'critical' ? 'bad' : a.level === 'warning' ? 'warn' : 'info'}`} role={a.level === 'critical' ? 'alert' : 'status'}>
          <Megaphone size={15} />
          <div className="grow"><b>{a.title}</b>{a.body ? <div className="small">{a.body}</div> : null}</div>
          <button className="icon-btn" aria-label={`Dismiss ${a.title}`} onClick={() => hide(a.id)}><X size={13} /></button>
        </div>
      ))}
    </div>
  );
}

/** Early access / closed mode: the owner has not let this account in yet. */
function LockedScreen({ signOut }: { signOut: () => void }) {
  const info = useAuth((s) => s.access?.access);
  const refreshAccess = useAuth((s) => s.refreshAccess);
  const [note, setNote] = useState('');
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const ask = async () => {
    setBusy(true);
    setErr(null);
    const { error } = await sb().rpc('request_access', { p_note: note });
    if (error) setErr(error.message);
    await refreshAccess();
    setBusy(false);
  };
  const closed = info?.mode === 'closed';
  return (
    <Notice title={closed ? 'Genius is closed right now' : 'Genius is in early access'}>
      <p className="muted">{info?.message || (closed ? 'The owner has switched Genius off for now. Please come back later.' : 'This account has not been let in yet. Ask for access and the owner will see your request.')}</p>
      {!closed && info?.waitlist && info.request === 'pending' && <div className="callout info" role="status">Your request was sent. You will be let in as soon as the owner approves it; reload this page afterwards.</div>}
      {!closed && info?.waitlist && info.request === 'declined' && <div className="callout bad" role="status">Your request was not approved.</div>}
      {!closed && info?.waitlist && !info.request && (
        <div className="col" style={{ gap: 8 }}>
          <textarea className="input" rows={2} maxLength={500} placeholder="Optional: who you are and why you want in" aria-label="Note for the owner" value={note} onChange={(e) => setNote(e.target.value)} />
          <button className="btn primary" disabled={busy} onClick={() => void ask()}>Ask for access</button>
        </div>
      )}
      {err && <div className="callout bad" role="alert">{err}</div>}
      <div className="row" style={{ gap: 8 }}>
        <button className="btn" onClick={() => void refreshAccess()}>Check again</button>
        <button className="btn" onClick={signOut}>Sign out</button>
      </div>
    </Notice>
  );
}

function Shell({ runtime }: { runtime: WebRuntime }) {
  const access = useAuth((s) => s.access);
  const session = useAuth((s) => s.session);
  const signOut = useAuth((s) => s.signOut);
  const route = useApp((s) => s.route);
  const navigate = useApp((s) => s.navigate);

  // URL ↔ route. The address bar always shows where you are, and Back/Forward work.
  useEffect(() => {
    navigate(pathToRoute(location.pathname));
    const pop = () => navigate(pathToRoute(location.pathname));
    window.addEventListener('popstate', pop);
    return () => window.removeEventListener('popstate', pop);
  }, [navigate]);
  useEffect(() => {
    const p = routeToPath(route);
    if (!isMobileApp && location.pathname !== p) history.pushState(null, '', p); // inside the app there is no address bar
  }, [route]);

  useEffect(() => {
    setFeatureResolver((key) => {
      if (localOnly) return { available: true, reason: 'ok', name: key };
      const f = access?.features?.[key];
      if (!access) return { available: false, reason: 'loading', name: key };
      return f ? { available: f.available, reason: f.reason, name: f.name } : { available: false, reason: 'switched_off', name: key };
    });
    return () => setFeatureResolver(null);
  }, [access]);
  useEffect(() => {
    setAccountView(
      localOnly ? { email: 'local-only (not synced)', plan: 'development', isAdmin: false, signOut: () => undefined, openAdmin: () => undefined } : session ? { email: session.user.email ?? 'account', plan: access?.plan === 'pro' ? 'Pro' : 'Free', isAdmin: !!access?.isAdmin, signOut: () => void signOut(), openAdmin: () => navigate({ name: 'admin' }) } : null,
    );
    return () => setAccountView(null);
  }, [session, access, signOut, navigate]);

  if (access?.status === 'suspended') {
    return (
      <Notice title="Your account is suspended">
        <p className="muted">You can no longer create new content. Contact the owner of this deployment if you think this is a mistake.</p>
        <button className="btn" onClick={() => void signOut()}>Sign out</button>
      </Notice>
    );
  }
  if (access?.access?.locked && !access.isAdmin) return <LockedScreen signOut={() => void signOut()} />;
  if (access?.maintenance && !access.isAdmin) {
    return (
      <Notice title="Genius is being maintained">
        <p className="muted">{access.maintenance.message || 'The service is temporarily switched off by its owner. Please try again later.'}</p>
        <button className="btn" onClick={() => void signOut()}>Sign out</button>
      </Notice>
    );
  }
  void runtime;
  return (
    <>
      <Announcements />
      <App />
    </>
  );
}

export default function WebApp() {
  const { ready, session, recovery, init } = useAuth();
  const [runtime, setRt] = useState<WebRuntime | null>(null);
  useEffect(() => {
    void init();
  }, [init]);

  const userId = session?.user.id ?? null;
  const vault = useMemo(() => (isNativeApp() ? createNativeKeyVault() : createKeyVault()), []);
  const store = useMemo(() => (userId ? new SupabaseStore(sb()) : localOnly ? new MemoryStore('genius.web.local') : null), [userId]);
  useEffect(() => {
    if (!store) {
      setRuntime(null);
      setRt(null);
      return;
    }
    const rt = new WebRuntime({ store, tools: localOnly ? noWebTools : proxyTools({ url: `${apiBase}/api/web`, accessToken }), accessToken: localOnly ? async () => '' : accessToken, relayUrl: `${apiBase}/api/relay`, keys: vault });
    setRuntime(rt);
    void Promise.all([vault.ready, rt.load()]).then(() => setRt(rt)); // remembered keys are decrypted before the first request
    return () => {
      setRuntime(null);
      setRt(null);
    };
  }, [store, vault]);

  // The same account on another device: pick up its new conversations and messages without a manual reload.
  const live = !!runtime && !!userId;
  useEffect(() => {
    if (!live || !userId) return;
    return startLiveRefresh(sb(), userId);
  }, [live, userId]);

  // The account key vault: provider keys follow the account (encrypted in this browser before they leave it).
  useEffect(() => {
    if (!userId || localOnly) return;
    keyHooks.saved = (p, k) => void onKeySaved(p, k);
    keyHooks.removed = (p) => void onKeyRemoved(p);
    void startKeySync(sb(), userId, webLocalKeys(vault));
    return () => {
      stopKeySync();
      keyHooks.saved = undefined;
      keyHooks.removed = undefined;
    };
  }, [userId, vault]);

  if (!ready) return <Notice title="Genius"><Loader2 className="spin" /></Notice>;
  if (!supabaseConfigured && !localOnly) {
    return (
      <Notice title="Accounts are not set up yet">
        <p className="muted">This Genius Web deployment has no account service configured, so it cannot sign anyone in. The owner needs to set <code>VITE_SUPABASE_URL</code> and <code>VITE_SUPABASE_ANON_KEY</code> in the Netlify site settings and redeploy (see docs/DEPLOY-NETLIFY-SUPABASE.md).</p>
      </Notice>
    );
  }
  if (!session && !localOnly) return <AuthScreen key={recovery ? 'recovery' : 'auth'} />;
  if (recovery) return <AuthScreen key="recovery" />;
  if (!runtime) return <Notice title="Genius"><Loader2 className="spin" /></Notice>;
  return <Shell runtime={runtime} />;
}
