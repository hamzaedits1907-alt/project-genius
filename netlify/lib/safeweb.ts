// Safe web access for the Genius Web proxy: keyless search (DuckDuckGo's HTML endpoint) and a guarded page
// fetcher. This is the server-side counterpart of crates/genius-core/src/web.rs and keeps the same rules:
//  * only http(s); no credentials in the URL; no well-known service ports;
//  * the host is resolved HERE and every resolved address must be public (no loopback, private,
//    link-local, CGNAT, unspecified, multicast, documentation, cloud-metadata, IPv4-mapped/6to4/NAT64
//    forms of those, or numeric host spellings); a single forbidden address refuses the whole name;
//  * the connection is pinned to the checked address, so DNS cannot change between check and connect;
//  * redirects are followed manually (max 5) and every hop is checked again;
//  * 20 s timeout, 2 MB body, text only. Everything returned is untrusted data.

import dns from 'node:dns/promises';
import net from 'node:net';
import { Agent, fetch as ufetch } from 'undici';

const BLOCKED_PORTS = new Set([22, 23, 25, 110, 135, 139, 143, 445, 465, 587, 993, 995, 1433, 1521, 2375, 2376, 3306, 3389, 5432, 5900, 6379, 9200, 11211, 27017]);
const MAX_BODY = 2 * 1024 * 1024;
const MAX_REDIRECTS = 5;
const TIMEOUT_MS = 20_000;
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';

export class WebError extends Error {
  constructor(message: string, public status = 400) {
    super(message);
  }
}

// ---------------------------------------------------------------------------------------- addresses

function v4Forbidden(a: number[]): boolean {
  const [x, y] = a;
  return (
    x === 0 || x === 10 || x === 127 || (x === 100 && y >= 64 && y <= 127) || (x === 169 && y === 254) || (x === 172 && y >= 16 && y <= 31) || (x === 192 && y === 0) || (x === 192 && y === 168) ||
    (x === 192 && y === 88) || (x === 198 && (y === 18 || y === 19)) || (x === 198 && y === 51) || (x === 203 && y === 0) || x >= 224
  );
}

function expandV6(ip: string): number[] | null {
  let s = ip.toLowerCase().replace(/^\[|\]$/g, '').split('%')[0];
  const v4 = /(\d+\.\d+\.\d+\.\d+)$/.exec(s);
  if (v4) {
    const o = v4[1].split('.').map(Number);
    s = s.slice(0, -v4[1].length) + `${((o[0] << 8) | o[1]).toString(16)}:${((o[2] << 8) | o[3]).toString(16)}`;
  }
  const [head, tail] = s.split('::');
  const h = head ? head.split(':') : [];
  const t = tail !== undefined && tail ? tail.split(':') : [];
  const fill = s.includes('::') ? 8 - h.length - t.length : 0;
  if (fill < 0) return null;
  const parts = [...h, ...Array(fill).fill('0'), ...t].map((p) => parseInt(p || '0', 16));
  return parts.length === 8 && parts.every((n) => Number.isInteger(n) && n >= 0 && n <= 0xffff) ? parts : null;
}

/** Whether a literal IP address may NOT be connected to. */
export function ipForbidden(ip: string): boolean {
  if (net.isIPv4(ip)) return v4Forbidden(ip.split('.').map(Number));
  const p = expandV6(ip);
  if (!p) return true;
  const embedded = (hi: number, lo: number) => v4Forbidden([hi >> 8, hi & 255, lo >> 8, lo & 255]);
  if (p.every((n) => n === 0) || (p.slice(0, 7).every((n) => n === 0) && p[7] === 1)) return true; // :: and ::1
  if ((p[0] & 0xfe00) === 0xfc00 || (p[0] & 0xffc0) === 0xfe80 || (p[0] & 0xff00) === 0xff00) return true; // ULA, link-local, multicast
  if (p[0] === 0x2001 && p[1] === 0x0db8) return true; // documentation
  if (p.slice(0, 5).every((n) => n === 0) && (p[5] === 0xffff || p[5] === 0)) return embedded(p[6], p[7]); // IPv4-mapped / -compatible
  if (p[0] === 0x2002) return embedded(p[1], p[2]); // 6to4
  if (p[0] === 0x2001 && p[1] === 0) return true; // Teredo
  if (p[0] === 0x64 && p[1] === 0xff9b) return embedded(p[6], p[7]); // NAT64
  return false;
}

/** Numeric host spellings (decimal, hex, octal, short forms) are refused outright. */
function numericHost(host: string): boolean {
  if (net.isIP(host)) return false;
  return /^(0x[0-9a-f]+|\d+)(\.(0x[0-9a-f]+|\d+)){0,3}$/i.test(host);
}

export function checkUrlShape(u: URL): void {
  if (u.protocol !== 'http:' && u.protocol !== 'https:') throw new WebError('only http and https addresses can be read');
  if (u.username || u.password) throw new WebError('addresses with a user name or password are not allowed');
  const host = u.hostname.replace(/^\[|\]$/g, '').replace(/\.$/, '').toLowerCase();
  if (!host) throw new WebError('the address has no host');
  if (host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local') || host.endsWith('.internal') || host === 'metadata.google.internal') throw new WebError('local and internal addresses are not allowed');
  if (numericHost(host)) throw new WebError('numeric host spellings are not allowed');
  if (net.isIP(host) && ipForbidden(host)) throw new WebError('private and local network addresses are not allowed');
  const port = u.port ? Number(u.port) : u.protocol === 'https:' ? 443 : 80;
  if (BLOCKED_PORTS.has(port)) throw new WebError(`port ${port} is not allowed`);
}

async function resolvePublic(host: string): Promise<{ address: string; family: 4 | 6 }[]> {
  const h = host.replace(/^\[|\]$/g, '');
  const list = net.isIP(h) ? [{ address: h, family: net.isIP(h) as 4 | 6 }] : ((await dns.lookup(h, { all: true, verbatim: true })) as { address: string; family: 4 | 6 }[]);
  if (!list.length) throw new WebError('the host could not be resolved', 502);
  if (list.some((a) => ipForbidden(a.address))) throw new WebError('private and local network addresses are not allowed');
  return list;
}

// ---------------------------------------------------------------------------------------- fetching

export interface FetchedPage {
  url: string;
  status: number;
  contentType: string;
  title: string | null;
  text: string;
  truncated: boolean;
}

async function rawFetch(start: URL, lookupOverride?: (host: string) => Promise<{ address: string; family: 4 | 6 }[]>): Promise<{ status: number; url: URL; type: string; body: Buffer; truncated: boolean }> {
  let url = start;
  for (let hop = 0; hop <= MAX_REDIRECTS; hop++) {
    checkUrlShape(url);
    const addrs = await (lookupOverride ?? resolvePublic)(url.hostname);
    const pinned = addrs[0];
    const agent = new Agent({ connect: { lookup: (_h, _o, cb) => (_o && (_o as { all?: boolean }).all ? (cb as unknown as (e: null, a: unknown) => void)(null, [{ address: pinned.address, family: pinned.family }]) : cb(null, pinned.address, pinned.family)) } });
    const ac = new AbortController();
    const timer = setTimeout(() => ac.abort(), TIMEOUT_MS);
    try {
      const res = await ufetch(url, { redirect: 'manual', signal: ac.signal, dispatcher: agent, headers: { 'user-agent': UA, accept: 'text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.5', 'accept-language': 'en' } });
      if (res.status >= 300 && res.status < 400) {
        const loc = res.headers.get('location');
        if (!loc) throw new WebError('the page redirected without saying where', 502);
        url = new URL(loc, url);
        await res.body?.cancel();
        continue;
      }
      const chunks: Buffer[] = [];
      let size = 0;
      let truncated = false;
      const reader = res.body?.getReader();
      while (reader) {
        const { done, value } = await reader.read();
        if (done) break;
        size += value.length;
        if (size > MAX_BODY) {
          chunks.push(Buffer.from(value.subarray(0, value.length - (size - MAX_BODY))));
          truncated = true;
          await reader.cancel();
          break;
        }
        chunks.push(Buffer.from(value));
      }
      return { status: res.status, url, type: (res.headers.get('content-type') ?? '').toLowerCase(), body: Buffer.concat(chunks), truncated };
    } catch (e) {
      if (e instanceof WebError) throw e;
      if ((e as Error).name === 'AbortError') throw new WebError('the page took too long to answer', 504);
      throw new WebError(`could not reach the page (${(e as Error).message})`, 502);
    } finally {
      clearTimeout(timer);
      void agent.close();
    }
  }
  throw new WebError('too many redirects', 502);
}

export async function fetchPage(rawUrl: string, lookup?: (host: string) => Promise<{ address: string; family: 4 | 6 }[]>): Promise<FetchedPage> {
  let u: URL;
  try {
    u = new URL(rawUrl.trim());
  } catch {
    throw new WebError('invalid URL');
  }
  const r = await rawFetch(u, lookup);
  if (/^(image|video|audio)\//.test(r.type) || r.type.includes('octet-stream') || r.type.includes('pdf') || r.type.includes('zip')) throw new WebError(`${r.url} is not a text page (content type ${r.type})`);
  const raw = r.body.toString('utf8');
  const looksHtml = r.type.includes('html') || (!r.type && /^\s*<(!doctype html|html)/i.test(raw.slice(0, 20)));
  let title: string | null = null;
  let text = raw;
  if (looksHtml) {
    const t = htmlToText(raw);
    title = t.title;
    text = t.text;
    const extra = htmlExtras(raw, text);
    if (extra) text = `${text.trimEnd()}\n\n${extra}`;
  }
  return { url: r.url.toString(), status: r.status, contentType: r.type, title, text, truncated: r.truncated };
}

// ---------------------------------------------------------------------------------------- html → text

const ENTITIES: [string, string][] = [['&nbsp;', ' '], ['&lt;', '<'], ['&gt;', '>'], ['&quot;', '"'], ['&apos;', "'"], ['&mdash;', '—'], ['&ndash;', '–'], ['&hellip;', '…'], ['&rsquo;', '’'], ['&lsquo;', '‘'], ['&rdquo;', '”'], ['&ldquo;', '“'], ['&copy;', '©'], ['&middot;', '·'], ['&bull;', '•'], ['&times;', '×'], ['&amp;', '&']];
export function decodeEntities(s: string): string {
  if (!s.includes('&')) return s;
  let out = s.replace(/&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});/g, (_m, n: string) => {
    const code = /^[xX]/.test(n) ? parseInt(n.slice(1), 16) : parseInt(n, 10);
    try {
      return String.fromCodePoint(code);
    } catch {
      return '';
    }
  });
  for (const [e, r] of ENTITIES) out = out.split(e).join(r);
  return out;
}

export function htmlToText(html: string): { title: string | null; text: string } {
  const titleRaw = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(html)?.[1];
  const title = titleRaw ? decodeEntities(titleRaw.replace(/<[^>]+>/g, '')).replace(/\s+/g, ' ').trim() || null : null;
  let s = html.replace(/<!--[\s\S]*?-->/g, '');
  for (const t of ['script', 'style', 'noscript', 'svg', 'nav', 'head', 'template', 'iframe', 'form', 'button', 'select']) s = s.replace(new RegExp(`<${t}\\b[^>]*>[\\s\\S]*?</${t}\\s*>`, 'gi'), '\n');
  s = s.replace(/<h[1-6]\b[^>]*>/gi, '\n\n## ').replace(/<li\b[^>]*>/gi, '\n- ').replace(/<\/t[dh]\s*>/gi, ' | ');
  s = s.replace(/<\/?(br|p|div|h[1-6]|ul|ol|tr|table|section|article|header|footer|main|aside|blockquote|pre|hr|dd|dt|figure|figcaption)\b[^>]*>/gi, '\n').replace(/<[^>]+>/g, '');
  s = decodeEntities(s).replace(/[ \t\u00a0]+/g, ' ').replace(/ *\n */g, '\n').replace(/\n{3,}/g, '\n\n').trim();
  return { title, text: s };
}

export function htmlExtras(rawHtml: string, text: string): string | null {
  const lines: string[] = [];
  for (const tag of rawHtml.match(/<meta\b[^>]*>/gi) ?? []) {
    const k = /\b(?:name|property)\s*=\s*["'](description|og:title|og:description|og:site_name|twitter:description)["']/i.exec(tag);
    const v = /\bcontent\s*=\s*["']([^"']*)["']/i.exec(tag);
    if (k && v) {
      const val = decodeEntities(v[1].trim());
      if (val && !text.includes(val)) lines.push(`${k[1]}: ${val}`);
    }
  }
  const scripts = (rawHtml.match(/<script\b[^>]*\bsrc\s*=\s*["'][^"']+["']/gi) ?? []).length;
  if (text.replace(/\s/g, '').length < 400 && (scripts > 0 || /<script/i.test(rawHtml))) {
    lines.push(`NOTE: the readable text of this page is very short and it loads ${scripts} script file(s): its content is probably built in the browser by JavaScript, which Genius does not run. Only the static HTML above could be read. Describe only what appears above; do not state or guess features, accounts, login, databases or technologies that are not visible in it.`);
  }
  return lines.length ? `PAGE METADATA (from the HTML head):\n${lines.join('\n')}` : null;
}

// ---------------------------------------------------------------------------------------- search

export interface SearchResult {
  title: string;
  url: string;
  snippet: string;
}

const stripTags = (s: string) => decodeEntities(s.replace(/<[^>]+>/g, '')).replace(/\s+/g, ' ').trim();

export function parseDdg(html: string): SearchResult[] {
  const out: SearchResult[] = [];
  const re = /<a\s([^>]*class="[^"]*\bresult__a\b[^"]*"[^>]*)>([\s\S]*?)<\/a>/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(html))) {
    const href = /\bhref\s*=\s*"([^"]*)"/i.exec(m[1])?.[1];
    if (!href) continue;
    let url = decodeEntities(href);
    if (url.startsWith('//')) url = `https:${url}`;
    try {
      const u = new URL(url);
      if (u.hostname.endsWith('duckduckgo.com') && u.pathname.startsWith('/l/')) url = u.searchParams.get('uddg') ?? url;
    } catch {
      continue;
    }
    if (!/^https?:\/\//i.test(url)) continue;
    const after = html.slice(m.index + m[0].length, m.index + m[0].length + 4000);
    const sn = /<(?:a|div|td|span)\s[^>]*class="[^"]*\bresult__snippet\b[^"]*"[^>]*>([\s\S]*?)<\/(?:a|div|td|span)>/i.exec(after);
    out.push({ title: stripTags(m[2]), url, snippet: sn ? stripTags(sn[1]) : '' });
    if (out.length >= 8) break;
  }
  return out;
}

export async function search(query: string): Promise<SearchResult[]> {
  const q = query.trim();
  if (!q || q.length > 400) throw new WebError('the query must be 1 to 400 characters');
  const r = await rawFetch(new URL(`https://html.duckduckgo.com/html/?q=${encodeURIComponent(q)}`));
  return parseDdg(r.body.toString('utf8'));
}
