// /api/web — web search and page reading for signed-in Genius Web users.
import type { Config } from '@netlify/functions';
import { fetchPage, search, WebError } from '../lib/safeweb.ts';
import { authenticate, AuthError, json, rateLimit, requireFeature } from '../lib/session.ts';

export default async (req: Request): Promise<Response> => {
  if (req.method !== 'POST') return json(405, { error: 'Use POST.' });
  try {
    const caller = await authenticate(req);
    requireFeature(caller, 'web_tools');
    rateLimit(`web:${caller.userId}`, 60, 60_000);
    const body = (await req.json().catch(() => ({}))) as { action?: string; query?: string; url?: string };
    if (body.action === 'search') return json(200, { results: await search(String(body.query ?? '')) });
    if (body.action === 'fetch') return json(200, { page: await fetchPage(String(body.url ?? '')) });
    return json(400, { error: 'Unknown action.' });
  } catch (e) {
    if (e instanceof AuthError || e instanceof WebError) return json(e.status, { error: e.message });
    return json(502, { error: 'The web proxy failed. Try again.' });
  }
};

export const config: Config = { path: '/api/web' };
