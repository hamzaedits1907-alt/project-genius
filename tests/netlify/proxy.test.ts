// The Genius Web proxy and relay: SSRF rules, page text extraction, search parsing, session checks.
// Real HTTP against a local test server for the fetch path; no internet, no Supabase, no provider.
import http from 'node:http';
import type { AddressInfo } from 'node:net';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { checkUrlShape, fetchPage, htmlExtras, htmlToText, ipForbidden, parseDdg, WebError } from '../../netlify/lib/safeweb';
import { checkTarget, forwardHeaders } from '../../netlify/lib/relay';
import { authenticate, AuthError, rateLimit, requireFeature } from '../../netlify/lib/session';

describe('addresses', () => {
  it('refuses every private, local, metadata and mapped form; allows public ones', () => {
    for (const bad of ['127.0.0.1', '10.1.2.3', '172.16.0.1', '172.31.255.255', '192.168.1.1', '169.254.169.254', '100.64.0.1', '0.0.0.0', '224.0.0.1', '255.255.255.255', '::1', '::', 'fe80::1', 'fd00:ec2::254', 'fc00::1', '::ffff:169.254.169.254', '::ffff:127.0.0.1', '2002:7f00:1::', '64:ff9b::a00:1', '2001:db8::1', '2001:0:4136:e378:8000:63bf:3fff:fdd2']) expect(ipForbidden(bad), bad).toBe(true);
    for (const ok of ['8.8.8.8', '1.1.1.1', '93.184.216.34', '172.32.0.1', '2606:4700:4700::1111']) expect(ipForbidden(ok), ok).toBe(false);
  });
  it('refuses unsafe URL shapes before any network use', () => {
    for (const bad of ['file:///etc/passwd', 'ftp://example.com/', 'gopher://x', 'http://user:pw@example.com/', 'http://localhost/', 'http://foo.localhost/', 'http://metadata.google.internal/', 'http://2852039166/', 'http://0x7f000001/', 'http://017700000001/', 'http://127.1/', 'http://example.com:6379/', 'http://[::1]/', 'http://[fd00:ec2::254]/latest', 'http://printer.local/'])
      expect(() => checkUrlShape(new URL(bad)), bad).toThrow(WebError);
    expect(() => checkUrlShape(new URL('https://react.dev/learn'))).not.toThrow();
  });
});

describe('guarded fetch against a real server', () => {
  let server: http.Server;
  let base = '';
  beforeAll(async () => {
    server = http.createServer((req, res) => {
      if (req.url === '/page') {
        res.setHeader('content-type', 'text/html; charset=utf-8');
        res.end('<html><head><title>Hooks – React</title><meta name="description" content="About hooks"></head><body><h1>Built-in Hooks</h1><p>Call hooks at the top level.</p><script>alert(1)</script></body></html>');
      } else if (req.url === '/spa') {
        res.setHeader('content-type', 'text/html');
        res.end('<html><head><title>Connect</title></head><body><div id="root"></div><script type="module" src="/assets/app.js"></script></body></html>');
      } else if (req.url === '/to-metadata') {
        res.statusCode = 302;
        res.setHeader('location', 'http://169.254.169.254/latest/meta-data/');
        res.end();
      } else if (req.url === '/loop') {
        res.statusCode = 302;
        res.setHeader('location', '/loop');
        res.end();
      } else if (req.url === '/big') {
        res.setHeader('content-type', 'text/plain');
        res.end('a'.repeat(3 * 1024 * 1024));
      } else if (req.url === '/img') {
        res.setHeader('content-type', 'image/png');
        res.end(Buffer.alloc(16));
      } else res.end('ok');
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    base = `http://public.test:${(server.address() as AddressInfo).port}`;
  });
  afterAll(() => new Promise((r) => server.close(() => r(null))));
  // test-only: this name maps to the local server; production resolution never allows loopback
  const lookup = async (h: string) => {
    if (h === 'public.test') return [{ address: '127.0.0.1', family: 4 as const }];
    throw new WebError('unexpected host', 400);
  };

  it('production resolution refuses the local server itself', async () => {
    await expect(fetchPage(`http://127.0.0.1:${new URL(base).port}/page`)).rejects.toThrow(/private and local/);
    await expect(fetchPage(`http://localhost:${new URL(base).port}/page`)).rejects.toThrow(/local and internal/);
  });
  it('reads a page as text, drops scripts, keeps the title and description', async () => {
    const p = await fetchPage(`${base}/page`, lookup);
    expect(p.status).toBe(200);
    expect(p.title).toBe('Hooks – React');
    expect(p.text).toContain('## Built-in Hooks');
    expect(p.text).toContain('Call hooks at the top level.');
    expect(p.text).not.toContain('alert(1)');
    expect(p.text).toContain('description: About hooks');
  });
  it('labels a script-built page instead of letting a model guess', async () => {
    const p = await fetchPage(`${base}/spa`, lookup);
    expect(p.text).toMatch(/built in the browser by JavaScript/);
    expect(p.text).toMatch(/do not state or guess/);
  });
  it('checks every redirect hop, bounds redirects, size and content type', async () => {
    await expect(fetchPage(`${base}/to-metadata`, lookup)).rejects.toThrow(/private and local/);
    await expect(fetchPage(`${base}/loop`, lookup)).rejects.toThrow(/too many redirects/);
    const big = await fetchPage(`${base}/big`, lookup);
    expect(big.truncated).toBe(true);
    expect(big.text.length).toBeLessThanOrEqual(2 * 1024 * 1024);
    await expect(fetchPage(`${base}/img`, lookup)).rejects.toThrow(/not a text page/);
  });
});

describe('page text and search parsing', () => {
  it('extracts headings, lists and table cells', () => {
    const { title, text } = htmlToText('<title>T &amp; U</title><h2>Head</h2><ul><li>one</li><li>two</li></ul><table><tr><td>a</td><td>b</td></tr></table>');
    expect(title).toBe('T & U');
    expect(text).toContain('## Head');
    expect(text).toContain('- one');
    expect(text).toContain('a | b |');
    expect(htmlExtras('<html><body>' + 'Real text. '.repeat(80) + '</body></html>', 'Real text. '.repeat(80))).toBeNull();
  });
  it('parses DuckDuckGo results and unwraps redirect links', () => {
    const html = `<div class="result"><h2 class="result__title"><a class="result__a" rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Freact.dev%2Flearn&rut=x">Learn React</a></h2><a class="result__snippet" href="x">Hooks &amp; state</a></div><div><a class="result__a" href="javascript:alert(1)">bad</a></div>`;
    expect(parseDdg(html)).toEqual([{ title: 'Learn React', url: 'https://react.dev/learn', snippet: 'Hooks & state' }]);
  });
});

describe('relay target rules', () => {
  it('relays only HTTPS chat/model requests to allowed or public hosts', async () => {
    expect((await checkTarget('https://integrate.api.nvidia.com/v1/chat/completions')).host).toBe('integrate.api.nvidia.com');
    expect((await checkTarget('https://integrate.api.nvidia.com/v1/models')).pathname).toBe('/v1/models');
    for (const bad of ['http://integrate.api.nvidia.com/v1/chat/completions', 'https://integrate.api.nvidia.com/v1/files', 'https://127.0.0.1/v1/chat/completions', 'https://localhost/v1/models', 'https://169.254.169.254/v1/models', 'ftp://x/v1/models', 'nonsense'])
      await expect(checkTarget(bad), bad).rejects.toThrow(WebError);
  });
  it('forwards only the key as authorization', () => {
    expect(forwardHeaders('k-12345678')).toEqual({ authorization: 'Bearer k-12345678', 'content-type': 'application/json', accept: 'text/event-stream, application/json' });
  });
});

describe('session checks', () => {
  const env = process.env;
  beforeAll(() => {
    process.env.SUPABASE_URL = 'https://example.supabase.co';
    process.env.SUPABASE_ANON_KEY = 'anon';
  });
  afterAll(() => {
    process.env = env;
  });
  const fakeFetch = (access: object, userOk = true): typeof fetch =>
    (async (url: string) => {
      if (String(url).endsWith('/auth/v1/user')) return userOk ? new Response(JSON.stringify({ id: 'u1' })) : new Response('no', { status: 401 });
      return new Response(JSON.stringify(access));
    }) as unknown as typeof fetch;
  const req = (auth?: string) => new Request('https://x/api/web', { headers: auth ? { authorization: auth } : {} });

  it('rejects missing, invalid and suspended callers and switched-off features', async () => {
    await expect(authenticate(req(), fakeFetch({}))).rejects.toMatchObject({ status: 401 });
    await expect(authenticate(req('Bearer t'), fakeFetch({}, false))).rejects.toMatchObject({ status: 401 });
    const ok = await authenticate(req('Bearer t'), fakeFetch({ status: 'active', features: { web_tools: { available: true, reason: 'ok' } } }));
    expect(ok.userId).toBe('u1');
    expect(() => requireFeature(ok, 'web_tools')).not.toThrow();
    const off = await authenticate(req('Bearer t'), fakeFetch({ status: 'active', features: { web_tools: { available: false, reason: 'switched_off' } } }));
    expect(() => requireFeature(off, 'web_tools')).toThrow(/switched off/);
    const sus = await authenticate(req('Bearer t'), fakeFetch({ status: 'suspended', features: {} }));
    expect(() => requireFeature(sus, 'web_tools')).toThrow(AuthError);
  });
  it('rate limits a user', () => {
    for (let i = 0; i < 3; i++) rateLimit('k', 3, 1000);
    expect(() => rateLimit('k', 3, 1000)).toThrow(/Too many/);
  });
});
