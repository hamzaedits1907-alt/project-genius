// Accounts for Genius Web (Supabase Auth) and the user's feature access.
//
// The browser only ever holds the public anon key; the service-role key never reaches it. What a user
// may do is decided by the database (row level security + `my_access()`); the hidden-looking admin route
// is only a convenience, not a protection.
import { createClient, type Session, type SupabaseClient } from '@supabase/supabase-js';
import { create } from 'zustand';

const CONFIG_KEY = 'genius.account.config';

/** Why a pasted key must be refused, or null when it is acceptable as the PUBLIC (anon/publishable) key. */
export function badPublicKey(key: string): string | null {
  const k = key.trim();
  if (k.startsWith('sb_secret_')) return 'That is a secret key. Use the public (anon / publishable) key; a secret key must never be put in an app.';
  if (k.startsWith('sb_publishable_')) return null;
  const parts = k.split('.');
  if (parts.length !== 3) return 'That does not look like a Supabase public key.';
  try {
    const payload = JSON.parse(atob(parts[1].replace(/-/g, '+').replace(/_/g, '/'))) as { role?: string };
    if (payload.role === 'service_role') return 'That is the service-role key. It must never be put in an app. Use the anon (public) key.';
    return payload.role === 'anon' ? null : 'That key is not the anon (public) key.';
  } catch {
    return 'That does not look like a Supabase public key.';
  }
}

/** Build-time values first (Netlify / release build); otherwise what the user saved in this app (desktop). */
function resolveConfig(): { url?: string; anon?: string; source: 'build' | 'saved' | 'none' } {
  const bu = import.meta.env.VITE_SUPABASE_URL as string | undefined;
  const ba = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined;
  if (bu && ba && /^https:\/\//.test(bu)) return { url: bu, anon: ba, source: 'build' };
  try {
    const saved = JSON.parse(localStorage.getItem(CONFIG_KEY) ?? 'null') as { url?: string; anon?: string } | null;
    if (saved?.url && saved.anon && /^https:\/\//.test(saved.url) && !badPublicKey(saved.anon)) return { ...saved, source: 'saved' };
  } catch {
    /* storage unavailable: treated as not configured */
  }
  return { source: 'none' };
}
const cfg = resolveConfig();
const url = cfg.url;
const anon = cfg.anon;
export const accountConfigSource = cfg.source;

/** Save the account service address (desktop). Takes effect after the app reloads. Returns an error text or null. */
export function saveAccountConfig(newUrl: string, newAnon: string): string | null {
  const u = newUrl.trim().replace(/\/+$/, '');
  if (!/^https:\/\/[a-z0-9-]+\.supabase\.co$/i.test(u)) return 'The address must look like https://your-project.supabase.co';
  const bad = badPublicKey(newAnon);
  if (bad) return bad;
  try {
    localStorage.setItem(CONFIG_KEY, JSON.stringify({ url: u, anon: newAnon.trim() }));
  } catch {
    return 'This app could not save the setting.';
  }
  return null;
}
export function clearAccountConfig(): void {
  try {
    localStorage.removeItem(CONFIG_KEY);
  } catch {
    /* nothing to clear */
  }
}

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
  reason: 'ok' | 'suspended' | 'maintenance' | 'switched_off' | 'not_rolled_out' | 'needs_pro';
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
  /** Set while the owner has put the service into maintenance (administrators never see it). */
  maintenance?: { enabled: boolean; message: string } | null;
  features: Record<string, FeatureState>;
}
export interface Limits {
  maxModelCalls: number | null;
  maxToolCalls: number | null;
  maxTaskMinutes: number | null;
  maxConcurrentTasks: number | null;
  maxRetries: number | null;
  contextChars: number | null;
  dailyAgentMissions: number | null;
  dailyWebToolCalls: number | null;
  dailyResearch: number | null;
}
export interface Announcement { id: string; title: string; body: string; level: 'info' | 'warning' | 'critical' }
export interface AccountContext { limits: Limits | null; announcements: Announcement[] }

interface AuthState {
  ready: boolean;
  session: Session | null;
  recovery: boolean;
  access: Access | null;
  context: AccountContext | null;
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
  context: null,
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
      else set({ access: null, context: null });
    });
  },
  refreshAccess: async () => {
    try {
      const { data, error } = await sb().rpc('my_access');
      if (error) throw error;
      set({ access: data as Access, error: null });
      // Limits and announcements are a convenience: failing to load them must never block the app.
      const ctx = await sb().rpc('my_context');
      if (!ctx.error && ctx.data) set({ context: ctx.data as AccountContext });
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String((e as { message?: string })?.message ?? e) });
    }
  },
  signOut: async () => {
    await sb().auth.signOut();
    set({ session: null, access: null, context: null });
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
