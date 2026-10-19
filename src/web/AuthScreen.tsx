// Sign-in, sign-up, magic link and password recovery for Genius Web.
import { useState, type FormEvent } from 'react';
import { Loader2, Mail } from 'lucide-react';
import { GeniusAvatar } from '../components/GeniusAvatar';
import { redirectTo, sb, useAuth } from './auth';

type Mode = 'in' | 'up' | 'magic' | 'forgot' | 'recover';

const message = (e: unknown) => (e && typeof e === 'object' && 'message' in e ? String((e as { message: unknown }).message) : String(e));

export default function AuthScreen({ embedded = false }: { embedded?: boolean }) {
  const recovery = useAuth((s) => s.recovery);
  const [mode, setMode] = useState<Mode>(recovery ? 'recover' : 'in');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const m: Mode = recovery ? 'recover' : mode;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    setNotice(null);
    if (m !== 'recover' && !/^\S+@\S+\.\S+$/.test(email.trim())) return setError('Enter a valid email address.');
    if ((m === 'in' || m === 'up' || m === 'recover') && password.length < 8) return setError('The password must have at least 8 characters.');
    setBusy(true);
    try {
      const auth = sb().auth;
      if (m === 'in') {
        const { error } = await auth.signInWithPassword({ email: email.trim(), password });
        if (error) throw error;
        if (embedded) setNotice('Signed in.');
      } else if (m === 'up') {
        const { data, error } = await auth.signUp({ email: email.trim(), password, options: { emailRedirectTo: redirectTo() } });
        if (error) throw error;
        // With email confirmation off (the supported setup) a session is returned and the app opens at once.
        if (data.user && (data.user.identities?.length ?? 1) === 0) {
          setMode('in');
          return setError('An account with this email already exists. Sign in instead, or use "Forgot password".');
        }
        if (!data.session) setNotice('Your account was created, but this service still asks for email confirmation, so no session was opened. Open the link we emailed you, then sign in. (The owner can turn confirmation off in Supabase → Authentication → Sign In / Providers → Email.)');
      } else if (m === 'magic') {
        const { error } = await auth.signInWithOtp({ email: email.trim(), options: { emailRedirectTo: redirectTo() } });
        if (error) throw error;
        setNotice('We sent you a sign-in link. Open it on this device.');
      } else if (m === 'forgot') {
        const { error } = await auth.resetPasswordForEmail(email.trim(), { redirectTo: redirectTo() });
        if (error) throw error;
        setNotice('If an account exists for this email, a reset link is on its way.');
      } else {
        const { error } = await auth.updateUser({ password });
        if (error) throw error;
        useAuth.setState({ recovery: false });
        setNotice('Your password was changed.');
      }
    } catch (err) {
      setError(friendly(message(err)));
    } finally {
      setBusy(false);
    }
  };

  const friendly = (t: string) => {
    if (/invalid login credentials/i.test(t)) return 'Wrong email or password.';
    if (/email not confirmed/i.test(t)) return 'This account has not confirmed its email yet. Open the confirmation link, or ask the owner to turn email confirmation off.';
    if (/already registered|already been registered/i.test(t)) return 'An account with this email already exists. Sign in instead.';
    if (/failed to fetch|networkerror|load failed/i.test(t)) return 'No connection to the account service. Check your internet connection and try again.';
    if (/registrations are closed/i.test(t)) return 'New registrations are closed right now.';
    if (/rate limit|too many/i.test(t)) return 'Too many attempts. Wait a minute and try again.';
    return t;
  };
  const title = { in: 'Welcome back', up: 'Create your account', magic: 'Sign in with a link', forgot: 'Reset your password', recover: 'Choose a new password' }[m];
  return (
    <main className={embedded ? undefined : 'au-root'}>
      <form className={embedded ? 'card pad col' : 'au-card'} style={embedded ? { gap: 10, maxWidth: 420 } : undefined} onSubmit={(e) => void submit(e)} aria-labelledby="au-title">
        {!embedded && <GeniusAvatar size={52} mood="idle" />}
        <h1 id="au-title" style={embedded ? { fontSize: 18, margin: 0 } : undefined}>{title}</h1>
        <p className="muted small" style={{ margin: 0 }}>
          {embedded ? 'The same account works in Genius Web and in this app. Your AI provider key is never stored in it.' : 'Genius Web keeps your conversations in your account. Your AI provider key is never stored in it.'}
        </p>
        {m !== 'recover' && (
          <label className="field">
            <span>Email</span>
            <input className="input" type="email" autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} required />
          </label>
        )}
        {(m === 'in' || m === 'up' || m === 'recover') && (
          <label className="field">
            <span>{m === 'recover' ? 'New password' : 'Password'}</span>
            <input className="input" type="password" autoComplete={m === 'in' ? 'current-password' : 'new-password'} value={password} onChange={(e) => setPassword(e.target.value)} minLength={8} required />
          </label>
        )}
        {error && <div className="callout bad" role="alert">{error}</div>}
        {notice && <div className="callout ok" role="status"><Mail size={15} /> <span>{notice}</span></div>}
        <button className="btn primary lg" type="submit" disabled={busy}>
          {busy && <Loader2 size={15} className="spin" />} {{ in: 'Sign in', up: 'Create account', magic: 'Send me a link', forgot: 'Send reset link', recover: 'Save password' }[m]}
        </button>
        {m !== 'recover' && (
          <div className="au-links">
            {m !== 'in' && <button type="button" className="link-btn" onClick={() => setMode('in')}>Sign in with password</button>}
            {m !== 'up' && <button type="button" className="link-btn" onClick={() => setMode('up')}>Create an account</button>}
            {m !== 'magic' && <button type="button" className="link-btn" onClick={() => setMode('magic')}>Email me a sign-in link</button>}
            {m === 'in' && <button type="button" className="link-btn" onClick={() => setMode('forgot')}>Forgot password?</button>}
          </div>
        )}
        <p className="faint tiny" style={{ margin: 0 }}>By continuing you agree that Genius Web is provided as is, and that you pay your AI provider directly for the models you use.</p>
      </form>
    </main>
  );
}
