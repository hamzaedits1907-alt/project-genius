// /api/relay — forwards a signed-in user's request to a provider that blocks browsers. Stores nothing.
import type { Config } from '@netlify/functions';
import { checkTarget, forwardHeaders } from '../lib/relay.ts';
import { WebError } from '../lib/safeweb.ts';
import { authenticate, AuthError, json, rateLimit } from '../lib/session.ts';

export default async (req: Request): Promise<Response> => {
  if (req.method !== 'POST') return json(405, { error: 'Use POST.' });
  try {
    const caller = await authenticate(req);
    if (caller.status === 'suspended') throw new AuthError('Your account is suspended.', 403);
    if (caller.locked) throw new AuthError('Genius is in early access and this account has not been let in yet.', 403);
    rateLimit(`relay:${caller.userId}`, 120, 60_000);
    const target = await checkTarget(req.headers.get('x-genius-target') ?? '');
    const key = req.headers.get('x-genius-key') ?? '';
    if (key.length < 8 || key.length > 400 || /\s/.test(key)) throw new WebError('missing or invalid provider key');
    const method = req.headers.get('x-genius-method') === 'GET' ? 'GET' : 'POST';
    const body = method === 'POST' ? await req.text() : undefined;
    if (body && body.length > 8 * 1024 * 1024) throw new WebError('request too large', 413);
    const upstream = await fetch(target, { method, headers: forwardHeaders(key), body, redirect: 'error', signal: AbortSignal.timeout(120_000) });
    // stream the provider's answer straight back; only safe headers are passed on
    const headers = new Headers({ 'content-type': upstream.headers.get('content-type') ?? 'application/json', 'cache-control': 'no-store', 'x-accel-buffering': 'no' });
    return new Response(upstream.body, { status: upstream.status, headers });
  } catch (e) {
    if (e instanceof AuthError || e instanceof WebError) return json(e.status, { error: e.message });
    return json(502, { error: 'The relay could not reach the provider.' });
  }
};

export const config: Config = { path: '/api/relay' };
