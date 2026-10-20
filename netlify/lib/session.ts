// Checking who calls a Genius Web function. The function never trusts the page: it asks Supabase (with the
// caller's own access token) who they are and what they may use. No service-role key is involved.

export interface Caller {
  userId: string;
  status: 'active' | 'suspended';
  /** Genius is locked for this account (early access / closed). */
  locked: boolean;
  features: Record<string, { available: boolean; reason: string }>;
}

export class AuthError extends Error {
  constructor(message: string, public status: number) {
    super(message);
  }
}

const env = (k: string) => (typeof process !== 'undefined' ? process.env[k] : undefined);

export async function authenticate(req: Request, fetchImpl: typeof fetch = fetch): Promise<Caller> {
  const url = env('SUPABASE_URL') ?? env('VITE_SUPABASE_URL');
  const anon = env('SUPABASE_ANON_KEY') ?? env('VITE_SUPABASE_ANON_KEY');
  if (!url || !anon) throw new AuthError('The account service is not configured on this deployment.', 503);
  const token = /^Bearer\s+(.+)$/i.exec(req.headers.get('authorization') ?? '')?.[1];
  if (!token) throw new AuthError('Sign in first.', 401);
  const headers = { apikey: anon, authorization: `Bearer ${token}`, 'content-type': 'application/json' };
  const me = await fetchImpl(`${url}/auth/v1/user`, { headers }).catch(() => null);
  if (!me || !me.ok) throw new AuthError('Your session is not valid. Sign in again.', 401);
  const user = (await me.json()) as { id?: string };
  if (!user.id) throw new AuthError('Your session is not valid. Sign in again.', 401);
  const acc = await fetchImpl(`${url}/rest/v1/rpc/my_access`, { method: 'POST', headers, body: '{}' }).catch(() => null);
  if (!acc || !acc.ok) throw new AuthError('Could not check your access.', 502);
  const a = (await acc.json()) as { status?: string; access?: { locked?: boolean }; features?: Caller['features'] };
  return { userId: user.id, status: a.status === 'suspended' ? 'suspended' : 'active', locked: a.access?.locked === true, features: a.features ?? {} };
}

export function requireFeature(c: Caller, feature: string) {
  if (c.status === 'suspended') throw new AuthError('Your account is suspended.', 403);
  if (c.locked) throw new AuthError('Genius is in early access and this account has not been let in yet.', 403);
  const f = c.features[feature];
  if (!f?.available) throw new AuthError(f?.reason === 'maintenance' ? 'Genius is in maintenance right now. Try again later.' : f?.reason === 'switched_off' ? 'This feature is switched off at the moment.' : f?.reason === 'needs_pro' ? 'This feature needs Genius Pro.' : 'This feature is not available to your account.', 403);
}

/** A tiny in-memory rate limit per user (best effort: a function instance may be recycled at any time). */
const hits = new Map<string, number[]>();
export function rateLimit(key: string, max: number, windowMs: number) {
  const now = Date.now();
  const list = (hits.get(key) ?? []).filter((t) => now - t < windowMs);
  if (list.length >= max) throw new AuthError('Too many requests. Wait a moment and try again.', 429);
  list.push(now);
  hits.set(key, list);
  if (hits.size > 5000) for (const [k, v] of hits) if (!v.some((t) => now - t < windowMs)) hits.delete(k);
}

export const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json', 'cache-control': 'no-store' } });
