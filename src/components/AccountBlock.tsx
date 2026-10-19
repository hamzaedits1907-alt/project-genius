// Sidebar footer: on the web it shows who is signed in (the account lives in src/web/auth.ts and
// registers itself here); the desktop app has no accounts, so it renders nothing.
import { useSyncExternalStore } from 'react';
import { LogOut } from 'lucide-react';
import { capabilities } from '../lib/platform';

export interface AccountView {
  email: string;
  plan: string;
  isAdmin: boolean;
  signOut: () => void;
  openAdmin: () => void;
}

let current: AccountView | null = null;
const listeners = new Set<() => void>();

/** Called by the web shell when the signed-in account changes. */
export function setAccountView(a: AccountView | null) {
  current = a;
  listeners.forEach((l) => l());
}

export function AccountBlock() {
  const a = useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current,
  );
  if (!capabilities.accounts || !a) return null;
  return (
    <div className="row" style={{ flex: 1, minWidth: 0 }}>
      <button className="gx-account" onClick={a.isAdmin ? a.openAdmin : undefined} title={a.isAdmin ? 'Open the owner admin panel' : a.email} aria-label={`Signed in as ${a.email}`}>
        <span className="av" aria-hidden>{a.email.slice(0, 1).toUpperCase()}</span>
        <span className="who">
          <b className="ellipsis">{a.email}</b>
          <span>{a.plan}{a.isAdmin ? ' · owner' : ''}</span>
        </span>
      </button>
      <button className="icon-btn" aria-label="Sign out" title="Sign out" onClick={a.signOut}>
        <LogOut size={14} />
      </button>
    </div>
  );
}
