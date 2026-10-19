// Accounts for Genius Web (Supabase Auth) and the user's feature access.
//
// The browser only ever holds the public anon key; the service-role key never reaches it. What a user
// may do is decided by the database (row level security + `my_access()`); the hidden-looking admin route
// is only a convenience, not a protection.
import { createClient, type Session, type SupabaseClient } from '@supabase/supabase-js';
import { create } from 'zustand';

const url = import.meta.env.VITE_SUPABASE_URL as string | undefined;
const anon = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined;

export const supabaseConfigured = !!url && !!anon && /^https:\/\//.test(url);
/** Development only: run without accounts, with conversations kept in this browser. Never enabled in a production build. */
export const localOnly = !supabaseConfigured && !!import.meta.env.DEV && import.meta.env.VITE_GENIUS_LOCAL === '1';

let client: SupabaseClient | null = null;
export function sb(): SupabaseClient {
  if (!supabaseConfigured) throw new Error('Accounts are not configured on this deployment.');
  client ??= createClient(url!, anon!, { auth: { persistSession: true, autoRefreshToken: true, detectSessionInUrl: true, flowType: 'pkce' } });
  return client;
}

export interface FeatureState {
  available: boolean;
  reason: 'ok' | 'suspended' | 'switched_off' | 'not_rolled_out' | 'needs_pro';
  name: string;
  description: string;
  minPlan: 'free' | 'pro';
}
export interface Access {
  signedIn: boolean;
  status: 'active' | 'suspended';
  plan: 'free' | 'pro';
  planExpiresAt: string | null;
  isAdmin: boolean;
  features: Record<string, FeatureState>;
}

interface AuthState {
  ready: boolean;
  session: Session | null;
  recovery: boolean;
  access: Access | null;
  error: string | null;
  init: () => Promise<void>;
  refreshAccess: () => Promise<void>;
  signOut: () => Promise<void>;
}

export const useAuth = create<AuthState>((set, get) => ({
  ready: false,
  session: null,
  recovery: false,
  access: null,
  error: null,
  init: async () => {
    if (!supabaseConfigured) {
      set({ ready: true });
      return;
    }
    const s = sb();
    const { data } = await s.auth.getSession();
    set({ session: data.session, ready: true });
    if (data.session) void get().refreshAccess();
    s.auth.onAuthStateChange((event, session) => {
      set({ session, recovery: event === 'PASSWORD_RECOVERY' ? true : get().recovery && !!session });
      if (session) void get().refreshAccess();
      else set({ access: null });
    });
  },
  refreshAccess: async () => {
    try {
      const { data, error } = await sb().rpc('my_access');
      if (error) throw error;
      set({ access: data as Access, error: null });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String((e as { message?: string })?.message ?? e) });
    }
  },
  signOut: async () => {
    await sb().auth.signOut();
    set({ session: null, access: null });
  },
}));

export async function accessToken(): Promise<string> {
  const { data } = await sb().auth.getSession();
  if (!data.session) throw { kind: 'auth', message: 'You are signed out. Sign in again.' };
  return data.session.access_token;
}

/** The callback address for emails (confirmation, magic link, password reset). Must be listed in Supabase's redirect URLs. */
export const redirectTo = () => `${location.origin}/app`;

/** Whether a feature is usable for this user (always true for local-only development and the desktop app). */
export function featureOf(access: Access | null, key: string): FeatureState {
  const f = access?.features?.[key];
  if (f) return f;
  return { available: !supabaseConfigured, reason: 'ok', name: key, description: '', minPlan: 'free' };
}
