// Settings → Account on the desktop: is this computer's history in step with the account?
import { useState } from 'react';
import { relinkToAccount, syncNow, useSync } from './desktopSync';

const label = { off: 'Not signed in', idle: 'Up to date', syncing: 'Syncing…', error: 'Not syncing', paused: 'Paused' } as const;

export default function SyncCard() {
  const s = useSync();
  const [busy, setBusy] = useState(false);
  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    try {
      await fn();
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="card pad col" style={{ gap: 8, marginTop: 12 }} aria-label="Conversation sync">
      <div className="row">
        <b className="grow">Conversation sync</b>
        <span className={`badge ${s.state === 'error' ? 'bad' : s.state === 'idle' ? 'ok' : ''}`}>{label[s.state]}</span>
      </div>
      <p className="small muted" style={{ margin: 0 }}>
        Your Chat and Research conversations are kept in your account, so they also appear in Genius Web and on your phone, and the other way round. Code projects, attachments and your AI key stay on this computer.
      </p>
      <div className="small muted">
        {s.lastSyncAt ? `Last synced ${new Date(s.lastSyncAt).toLocaleTimeString()}.` : 'Not synced yet.'} {s.pending > 0 ? `${s.pending} change(s) waiting to upload.` : ''}
      </div>
      {s.state === 'paused' && (
        <div className="callout warn" role="alert">
          The conversations on this computer belong to a different Genius account, so nothing is merged. Sign in with that account, or move this computer's conversations to the account you are using now.
          <div className="row" style={{ marginTop: 8 }}>
            <button className="btn" disabled={busy} onClick={() => void act(relinkToAccount)}>Move them to this account</button>
          </div>
        </div>
      )}
      {s.error && <div className="callout bad" role="alert">{s.error}</div>}
      {s.warnings.map((w) => <div key={w} className="callout warn small">{w}</div>)}
      <div className="row"><button className="btn" disabled={busy || s.state === 'syncing'} onClick={() => void act(syncNow)}>Sync now</button></div>
    </div>
  );
}
