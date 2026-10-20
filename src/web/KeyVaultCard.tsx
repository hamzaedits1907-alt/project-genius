// Settings → Account: the account key vault (provider keys follow the account, encrypted on the device).
import { useState } from 'react';
import { KeyRound, Loader2, Lock, ShieldCheck } from 'lucide-react';
import { checkPassphrase } from './accountVault';
import { keyVaultActions, useKeyVault } from './keySync';

const LABEL: Record<string, string> = { nvidia: 'NVIDIA NIM', openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Google Gemini', custom: 'Custom endpoint' };
const STATE_TEXT = { 'signed-out': 'signed out', loading: 'checking…', none: 'not set up', locked: 'locked', unlocked: 'unlocked', unavailable: 'unavailable' } as const;

export default function KeyVaultCard() {
  const v = useKeyVault();
  const [pass, setPass] = useState('');
  const [pass2, setPass2] = useState('');
  const [remember, setRemember] = useState(true);
  const [oldPass, setOldPass] = useState('');
  const [newPass, setNewPass] = useState('');
  const [open, setOpen] = useState<null | 'change' | 'reset'>(null);
  const [local, setLocal] = useState<string | null>(null);

  const create = async () => {
    const bad = checkPassphrase(pass);
    if (bad) return setLocal(bad);
    if (pass !== pass2) return setLocal('The two passphrases are different.');
    setLocal(null);
    await keyVaultActions.create(pass, remember);
    setPass('');
    setPass2('');
  };
  const unlock = () => keyVaultActions.unlock(pass, remember).then(() => setPass(''));

  return (
    <div className="card pad col" style={{ gap: 8, marginTop: 12 }} aria-label="Key vault">
      <div className="row">
        <KeyRound size={15} />
        <b className="grow">Key vault: your AI keys in your account</b>
        <span className={`badge ${v.state === 'unlocked' ? 'ok' : v.state === 'unavailable' ? 'bad' : ''}`}>{STATE_TEXT[v.state]}</span>
      </div>
      <p className="small muted" style={{ margin: 0 }}>
        Save your provider keys once and they are on every device you sign in to (desktop, web, phone). They are encrypted on your device with a passphrase only you know before they reach your account, so the database, the site owner and Supabase can only see scrambled data. There is no way to recover the passphrase: if you forget it you reset the vault and paste your keys again.
      </p>

      {v.state === 'loading' && <Loader2 className="spin" />}

      {v.state === 'none' && (
        <div className="col" style={{ gap: 6, maxWidth: 420 }}>
          <input className="input" type="password" autoComplete="new-password" placeholder="Choose a vault passphrase (8+ characters)" aria-label="Vault passphrase" value={pass} onChange={(e) => setPass(e.target.value)} />
          <input className="input" type="password" autoComplete="new-password" placeholder="Repeat the passphrase" aria-label="Repeat the vault passphrase" value={pass2} onChange={(e) => setPass2(e.target.value)} />
          <label className="row small" style={{ gap: 6 }}><input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} /> Do not ask again on this device</label>
          <div className="row"><button className="btn primary" disabled={v.busy || !pass} onClick={() => void create()}>Create key vault</button></div>
        </div>
      )}

      {v.state === 'locked' && (
        <div className="col" style={{ gap: 6, maxWidth: 420 }}>
          <div className="small muted"><Lock size={12} /> Saved in your account: {v.providers.length ? v.providers.map((p) => LABEL[p] ?? p).join(', ') : 'nothing yet'}.</div>
          <input className="input" type="password" autoComplete="current-password" placeholder="Vault passphrase" aria-label="Vault passphrase" value={pass} onChange={(e) => setPass(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && pass && void unlock()} />
          <label className="row small" style={{ gap: 6 }}><input type="checkbox" checked={remember} onChange={(e) => setRemember(e.target.checked)} /> Do not ask again on this device</label>
          <div className="row">
            <button className="btn primary" disabled={v.busy || !pass} onClick={() => void unlock()}>Unlock</button>
            <button className="btn ghost" onClick={() => setOpen(open === 'reset' ? null : 'reset')}>Forgot the passphrase?</button>
          </div>
        </div>
      )}

      {v.state === 'unlocked' && (
        <div className="col" style={{ gap: 6 }}>
          <div className="small"><ShieldCheck size={13} color="var(--ok)" /> Saved in your account: <b>{v.providers.length ? v.providers.map((p) => LABEL[p] ?? p).join(', ') : 'nothing yet. Paste a key in Settings → AI provider and it is saved here automatically.'}</b></div>
          <div className="small muted">{v.remembered ? 'This device opens the vault without asking.' : 'This device will ask for the passphrase next time.'}</div>
          <div className="row" style={{ flexWrap: 'wrap' }}>
            <button className="btn" disabled={v.busy} onClick={() => void keyVaultActions.refresh()}>Refresh keys now</button>
            <button className="btn" disabled={v.busy} onClick={() => void keyVaultActions.lock()}>Lock this device</button>
            <button className="btn ghost" onClick={() => setOpen(open === 'change' ? null : 'change')}>Change passphrase</button>
            <button className="btn ghost" onClick={() => setOpen(open === 'reset' ? null : 'reset')}>Reset vault</button>
          </div>
        </div>
      )}

      {open === 'change' && v.state === 'unlocked' && (
        <div className="col" style={{ gap: 6, maxWidth: 420 }}>
          <input className="input" type="password" autoComplete="current-password" placeholder="Current passphrase" aria-label="Current passphrase" value={oldPass} onChange={(e) => setOldPass(e.target.value)} />
          <input className="input" type="password" autoComplete="new-password" placeholder="New passphrase (8+ characters)" aria-label="New passphrase" value={newPass} onChange={(e) => setNewPass(e.target.value)} />
          <div className="row"><button className="btn primary" disabled={v.busy || !oldPass || !newPass} onClick={() => void keyVaultActions.change(oldPass, newPass).then(() => { setOldPass(''); setNewPass(''); setOpen(null); })}>Change passphrase</button></div>
        </div>
      )}
      {open === 'reset' && (v.state === 'unlocked' || v.state === 'locked') && (
        <div className="callout warn" role="alert">
          Resetting deletes the vault and every key in it from your account. Keys stay on the devices that already have them; you can create a new vault afterwards.
          <div className="row" style={{ marginTop: 8 }}><button className="btn" disabled={v.busy} onClick={() => void keyVaultActions.reset().then(() => setOpen(null))}>Delete the vault</button></div>
        </div>
      )}

      {local && <div className="callout bad" role="alert">{local}</div>}
      {v.error && <div className="callout bad" role="alert">{v.error}</div>}
      {v.notice && <div className="callout ok" role="status">{v.notice}</div>}
    </div>
  );
}
