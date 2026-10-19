// The provider relay: for providers whose API refuses calls from a browser (NVIDIA NIM, most custom
// endpoints). The browser sends the request here with the user's OWN key in a header; the relay checks
// the target, forwards the request over HTTPS and streams the answer back. It keeps nothing: no key, no
// prompt, no answer is stored, logged or cached. Only an allow-list of provider hosts can be reached
// (custom endpoints must be public HTTPS hosts and pass the same address checks as the web proxy).

import { checkUrlShape, ipForbidden, WebError } from './safeweb.ts';
import dns from 'node:dns/promises';
import net from 'node:net';

export const ALLOWED_HOSTS = ['integrate.api.nvidia.com', 'api.nvcf.nvidia.com'];

export async function checkTarget(raw: string): Promise<URL> {
  let u: URL;
  try {
    u = new URL(raw);
  } catch {
    throw new WebError('invalid target address');
  }
  if (u.protocol !== 'https:') throw new WebError('only https providers can be reached');
  checkUrlShape(u);
  if (!/\/(chat\/completions|models)(\?.*)?$/.test(u.pathname + u.search)) throw new WebError('only chat completion and model list requests can be relayed');
  const host = u.hostname.toLowerCase();
  if (!ALLOWED_HOSTS.includes(host)) {
    // custom endpoint: must resolve to public addresses only
    const addrs = net.isIP(host) ? [{ address: host }] : await dns.lookup(host, { all: true });
    if (!addrs.length || addrs.some((a) => ipForbidden(a.address))) throw new WebError('private and local network addresses are not allowed');
  }
  return u;
}

export function forwardHeaders(key: string): Record<string, string> {
  return { authorization: `Bearer ${key}`, 'content-type': 'application/json', accept: 'text/event-stream, application/json' };
}
