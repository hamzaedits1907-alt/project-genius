// Settings → Account (desktop and web): sign in / create an account, who is signed in, what the plan
// allows, the limits that apply, password change, privacy facts. The desktop app and Genius Web use the
// same account service, so one account works in both.
import { useState } from 'react';
import { Check, X } from 'lucide-react';
import { isWeb } from '../lib/platform';
import { extraViews } from '../lib/extraViews';
import AuthScreen from './AuthScreen';
import KeyVaultCard from './KeyVaultCard';
import { accountConfigSource, clearAccountConfig, saveAccountConfig, sb, supabaseConfigured, useAuth, type Limits } from './auth';

const LIMIT_LABELS: [keyof Limits, string][] = [
  ['maxModelCalls', 'Model calls per task'],
  ['maxToolCalls', 'Tool calls per task'],
  ['maxTaskMinutes', 'Minutes per task'],
  ['maxConcurrentTasks', 'Tasks at the same time'],
  ['maxRetries', 'Retries per step'],
  ['dailyAgentMissions', 'Agent missions per day'],
  ['dailyResearch', 'Research reports per day'],
  ['dailyWebToolCalls', 'Web searches/page reads per day'],
];

function ConnectService() {
  const [url, setUrl] = useState('');
  const [key, setKey] = useState('');
  const [msg, setMsg] = useState<string | null>(null);
  const save = () => {
    const err = saveAccountConfig(url, key);
    if (err) return setMsg(err);
    location.reload();
  };
  return (
    <div className="card pad col" style={{ gap: 8, maxWidth: 520 }}>
      <b>Connect the Genius account service</b>
      <p className="small muted" style={{ margin: 0 }}>
        This build does not include the account service address. Paste the two PUBLIC values from your Supabase project (Project settings → API): the project URL and the <b>anon / publishable</b> key. Never paste the service-role or secret key; this app refuses it.
      </p>
      <input className="input" placeholder="https://your-project.supabase.co" aria-label="Account service address" value={url} onChange={(e) => setUrl(e.target.value)} />
      <input className="input" placeholder="anon / publishable key" aria-label="Public key" value={key} onChange={(e) => setKey(e.target.value)} autoComplete="off" spellCheck={false} />
      {msg && <div className="callout bad" role="alert">{msg}</div>}
      <div className="row"><button className="btn primary" disabled={!url || !key} onClick={save}>Save and reload</button></div>
    </div>
  );
}

export default function AccountSettings() {
  const session = useAuth((s) => s.session);
  const access = useAuth((s) => s.access);
  const context = useAuth((s) => s.context);
  const signOut = useAuth((s) => s.signOut);
  const [pw, setPw] = useState('');
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  if (!supabaseConfigured) {
    return (
      <section>
        <h2>Account</h2>
        {isWeb ? <p className="muted">Accounts are not configured on this deployment.</p> : <ConnectService />}
      </section>
    );
  }
  if (!session) {
    return (
      <section>
        <h2>Account</h2>
        <p className="muted small">Signing in is optional on the desktop: Genius works without an account. An account adds the Genius Pro features and the same login in Genius Web.</p>
        <AuthScreen embedded />
      </section>
    );
  }
  const change = async () => {
    if (pw.length < 8) return setMsg({ ok: false, text: 'The password must have at least 8 characters.' });
    const { error } = await sb().auth.updateUser({ password: pw });
    setMsg(error ? { ok: false, text: error.message } : { ok: true, text: 'Password changed.' });
    if (!error) setPw('');
  };
  const limits = context?.limits;
  return (
    <section>
      <h2>Account</h2>
      <div className="card pad col" style={{ gap: 6 }}>
        <div><b>{session.user.email}</b></div>
        <div className="small muted">Plan: {access?.plan === 'pro' ? `Pro${access.planExpiresAt ? ` until ${access.planExpiresAt.slice(0, 10)}` : ''}` : 'Free'}{access?.isAdmin ? ' · owner' : ''}</div>
        <div className="small muted">Genius Pro adds Genius capabilities (Agent Mode, custom agents, higher task budgets). It never pays for model usage: you pay your AI provider directly.</div>
      </div>
      <div className="card pad col" style={{ gap: 6, marginTop: 12 }}>
        <b>What your plan allows</b>
        <ul style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 4 }}>
          {Object.entries(access?.features ?? {}).map(([k, f]) => (
            <li key={k} className="row small">
              {f.available ? <Check size={13} color="var(--ok)" /> : <X size={13} color="var(--text-3)" />}
              <b>{f.name}</b>
              <span className="faint">{f.available ? '' : f.reason === 'needs_pro' ? 'needs Pro' : f.reason.replace(/_/g, ' ')}</span>
            </li>
          ))}
        </ul>
      </div>
      <KeyVaultCard />
      {extraViews.sync && (() => { const S = extraViews.sync; return <S />; })()}
      {limits && (
        <div className="card pad col" style={{ gap: 6, marginTop: 12 }}>
          <b>Your Genius limits</b>
          <p className="small muted" style={{ margin: 0 }}>These cap Genius features (tasks, tools, missions). They do not change what your AI provider charges you.</p>
          <dl className="am-facts">
            {LIMIT_LABELS.map(([k, label]) => (
              <div key={k} style={{ display: 'contents' }}>
                <dt>{label}</dt>
                <dd>{limits[k] ?? 'no cap'}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}
      <div className="card pad col" style={{ gap: 8, marginTop: 12 }}>
        <b>Change password</b>
        <div className="row">
          <input className="input grow" type="password" autoComplete="new-password" placeholder="New password (8+ characters)" aria-label="New password" value={pw} onChange={(e) => setPw(e.target.value)} />
          <button className="btn" onClick={() => void change()}>Change</button>
        </div>
        {msg && <div className={`callout ${msg.ok ? 'ok' : 'bad'}`} role="status">{msg.text}</div>}
      </div>
      <div className="card pad col" style={{ gap: 6, marginTop: 12 }}>
        <b>Privacy</b>
        <ul className="small muted" style={{ margin: 0, paddingLeft: 18 }}>
          <li>Your account stores your profile, plan and your Chat and Research conversations (synced between Genius Desktop, Genius Web and the mobile apps), visible only to you (enforced by the database).</li>
          <li>Your AI provider keys are saved in your account only if you create the key vault, and then only as ciphertext that is encrypted on your device with a passphrase nobody else has: the database and the site owner cannot read them. Without a vault they stay on this device. {isWeb ? 'In this browser you choose in Settings → AI provider whether a key is remembered on this device (encrypted) or kept only until the tab closes.' : 'On this computer they are kept in the operating system credential store.'}</li>
          <li>Prompts go from {isWeb ? 'your browser' : 'this app'} to the provider you chose{isWeb ? '; NVIDIA and custom endpoints go through the Genius relay, which forwards them and stores nothing' : ''}.</li>
        </ul>
      </div>
      <div className="row" style={{ marginTop: 12 }}>
        <button className="btn" onClick={() => void signOut()}>Sign out</button>
        {!isWeb && accountConfigSource === 'saved' && <button className="btn ghost" onClick={() => { clearAccountConfig(); void signOut().finally(() => location.reload()); }}>Forget account service</button>}
      </div>
    </section>
  );
}
