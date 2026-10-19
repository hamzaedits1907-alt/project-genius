// Account support for the desktop app. Signing in is optional (Genius works without an account); it
// uses the same Supabase project as Genius Web, so one account works in both. Nothing here touches the
// provider key, which stays in the operating system credential store.
import { setAccountView } from '../components/AccountBlock';
import { extraViews } from '../lib/extraViews';
import { useApp } from '../store/app';
import AccountSettings from './AccountSettings';
import { supabaseConfigured, useAuth } from './auth';

export function initDesktopAccount(): void {
  extraViews.account = AccountSettings;
  const openAccount = () => useApp.getState().navigate({ name: 'settings', tab: 'account' });
  const publish = () => {
    const { session, access, signOut } = useAuth.getState();
    setAccountView(
      session
        ? { email: session.user.email ?? 'account', plan: access?.plan === 'pro' ? 'Pro' : 'Free', isAdmin: false, signOut: () => void signOut(), openAdmin: openAccount, openAccount }
        : { email: '', plan: '', isAdmin: false, signOut: () => undefined, openAdmin: openAccount, openAccount },
    );
  };
  publish();
  useAuth.subscribe(publish);
  if (supabaseConfigured) void useAuth.getState().init();
}
