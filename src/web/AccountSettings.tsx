// Settings → Account (Genius Web): who is signed in, what the plan allows, password change, privacy facts.
import { useState } from 'react';
import { Check, X } from 'lucide-react';
import { sb, useAuth } from './auth';

export default function AccountSettings() {
  const session = useAuth((s) => s.session);
  const access = useAuth((s) => s.access);
  const signOut = useAuth((s) => s.signOut);
  const [pw, setPw] = useState('');
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const change = async () => {
    if (pw.length < 8) return setMsg({ ok: false, text: 'The password must have at least 8 characters.' });
    const { error } = await sb().auth.updateUser({ password: pw });
    setMsg(error ? { ok: false, text: error.message } : { ok: true, text: 'Password changed.' });
    if (!error) setPw('');
  };
  return (
    <section>
      <h2>Account</h2>
      <div className="card pad col" style={{ gap: 6 }}>
        <div><b>{session?.user.email}</b></div>
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
          <li>Your conversations, sources and files are stored in your account and visible only to you (enforced by the database).</li>
          <li>Your AI provider key is kept in this browser tab only. It is never saved to your account or any database, and is lost when the tab closes.</li>
          <li>Prompts go from your browser to the provider you chose. NVIDIA and custom endpoints go through the Genius relay, which forwards them and stores nothing.</li>
          <li>Web searches and page reads go through the Genius web proxy, which blocks private and local addresses.</li>
        </ul>
      </div>
      <button className="btn" style={{ marginTop: 12 }} onClick={() => void signOut()}>Sign out</button>
    </section>
  );
}
