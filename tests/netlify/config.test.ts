// Netlify routing and build configuration, and the built output when it exists.
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

const root = join(__dirname, '..', '..');
const toml = readFileSync(join(root, 'netlify.toml'), 'utf8');

describe('netlify.toml', () => {
  it('builds the web app (not the desktop app) into dist-web with the functions folder', () => {
    expect(toml).toMatch(/command = "npm ci && npm run build:web"/);
    expect(toml).toMatch(/publish = "dist-web"/);
    expect(toml).toMatch(/functions = "netlify\/functions"/);
    expect(toml).not.toMatch(/tauri build/);
  });
  it('serves the single-page app for /app/* and /admin', () => {
    for (const from of ['/app/*', '/admin', '/admin/*', '/app']) expect(toml).toContain(`from = "${from}"`);
    expect(toml.match(/to = "\/app\/index.html"\s+status = 200/g)?.length).toBe(4);
  });
  it('sets a strict content security policy for the app and never allows inline scripts', () => {
    expect(toml).toMatch(/script-src 'self';/);
    expect(toml).not.toMatch(/script-src[^;]*unsafe-inline/);
    expect(toml).toMatch(/frame-ancestors 'none'/);
    expect(toml).toMatch(/object-src 'none'/);
  });
  it('needs only public Supabase values, never a service-role key', () => {
    const files = ['docs/DEPLOY-NETLIFY-SUPABASE.md', '.env.example'].map((f) => readFileSync(join(root, f), 'utf8')).join('\n');
    expect(files).toMatch(/VITE_SUPABASE_ANON_KEY/);
    expect(files).toMatch(/service-role/i);
    expect(toml).not.toMatch(/service[_-]role/i);
  });
});

describe('built output (after npm run build:web)', () => {
  const dist = join(root, 'dist-web');
  it.skipIf(!existsSync(dist))('contains the landing page, the app shell and no keys', () => {
    expect(existsSync(join(dist, 'index.html'))).toBe(true);
    expect(existsSync(join(dist, 'app', 'index.html'))).toBe(true);
    const app = readFileSync(join(dist, 'app', 'index.html'), 'utf8');
    expect(app).toMatch(/<script type="module" crossorigin src="\/assets\//);
    expect(app).not.toMatch(/<script>[^<]+<\/script>/); // no inline scripts: the CSP forbids them
    expect(readFileSync(join(dist, 'index.html'), 'utf8')).not.toMatch(/sk-[a-z0-9]{20}|nvapi-/i);
  });
});
