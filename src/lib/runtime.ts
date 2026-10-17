// Runtime problems seen in the live preview: friendly summaries for the problem bar and the
// text Genius receives when asked to fix them (mirrors `render_preview_errors` in the core).
import type { PreviewRuntimeError } from './types';

export type Severity = 'crash' | 'blank' | 'error' | 'warn';

export function severity(e: PreviewRuntimeError): Severity {
  if (e.kind === 'blank') return 'blank';
  if (e.kind === 'console') return e.source === 'console.warn' ? 'warn' : 'error';
  if (e.kind === 'resource') return 'error';
  return 'crash';
}

const RANK: Record<Severity, number> = { crash: 0, blank: 1, error: 2, warn: 3 };

const base = (p: string) => p.split(/[\\/]/).pop() ?? p;

/** Where to look: the first stack frame in the project's own code, else the reported source. */
export function errorLocation(e: PreviewRuntimeError): string | null {
  const m = /(?:^|[\s(@/])(src\/[^\s():?]+):(\d+)/.exec(e.stack ?? '');
  if (m) return `${base(m[1])}:${m[2]}`;
  if (!e.source || e.source.startsWith('console.')) return null;
  return e.line ? `${base(e.source)}:${e.line}` : base(e.source);
}

const firstLine = (s: string) => s.split('\n').find((l) => l.trim())?.trim() ?? '';

/** Headline for the problem bar, or null when there is nothing worth interrupting for (warnings only). */
export function problemSummary(errors: PreviewRuntimeError[]): { severity: Severity; title: string; detail: string; location: string | null; count: number } | null {
  const shown = errors.filter((e) => severity(e) !== 'warn');
  if (!shown.length) return null;
  // Stable sort keeps the earliest error of the worst kind: usually the root cause.
  const top = shown.slice().sort((a, b) => RANK[severity(a)] - RANK[severity(b)])[0];
  const sev = severity(top);
  const title = sev === 'blank' ? 'Your app shows a blank page' : sev === 'crash' ? 'Your app crashed' : 'Your app reported a problem';
  return { severity: sev, title, detail: sev === 'blank' ? '' : firstLine(top.message), location: errorLocation(top), count: shown.reduce((n, e) => n + e.count, 0) };
}

export function kindLabel(e: PreviewRuntimeError): string {
  switch (e.kind) {
    case 'rejection':
      return 'Unhandled promise rejection';
    case 'console':
      return e.source === 'console.warn' ? 'console.warn' : 'console.error';
    case 'resource':
      return 'Failed to load resource';
    case 'blank':
      return 'Blank page';
    case 'vite':
      return 'Vite error overlay';
    default:
      return 'Uncaught error';
  }
}

/** Compact text for the model: one block per error with count, location and first stack lines. */
export function renderRuntimeErrors(errors: PreviewRuntimeError[]): string {
  return errors
    .map((e, i) => {
      const src = e.source && !e.source.startsWith('console.') ? e.source : null;
      const loc = src ? [src, e.line, e.line ? e.column : null].filter((x) => x != null).join(':') : null;
      let out = `[${i + 1}] ${kindLabel(e)}${e.count > 1 ? ` (×${e.count})` : ''}${loc ? ` at ${loc}` : ''} on page ${e.page}\n`;
      out += e.message.split('\n').slice(0, 20).map((l) => `  ${l.trimEnd()}\n`).join('');
      const head = firstLine(e.message);
      const stack = (e.stack ?? '').split('\n').map((l) => l.trim()).filter((l) => l && l !== head).slice(0, 15);
      if (stack.length) out += `  Stack:\n${stack.map((l) => `    ${l}\n`).join('')}`;
      return out;
    })
    .join('\n');
}

export function fixMessage(errors: PreviewRuntimeError[]): string {
  return `The preview has runtime errors. Find the cause and fix it:\n${renderRuntimeErrors(errors)}`;
}

/** Accept only well-formed reports from the preview page (it is untrusted code). */
export function parseRuntimeReport(data: unknown): PreviewRuntimeError[] {
  const list = (data as { errors?: unknown })?.errors;
  if (!Array.isArray(list)) return [];
  const str = (v: unknown, n: number) => (typeof v === 'string' ? v.slice(0, n) : undefined);
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.floor(v) : undefined);
  const kinds = ['error', 'rejection', 'console', 'resource', 'blank', 'vite'];
  return list.slice(0, 50).flatMap((x: Record<string, unknown>) => {
    if (!x || typeof x !== 'object' || typeof x.kind !== 'string' || !kinds.includes(x.kind) || typeof x.message !== 'string') return [];
    return [{ kind: x.kind as PreviewRuntimeError['kind'], message: x.message.slice(0, 2000), source: str(x.source, 400), line: num(x.line), column: num(x.column), stack: str(x.stack, 4000), page: str(x.page, 300) ?? '/', at: str(x.at, 40) ?? new Date().toISOString(), count: Math.max(1, num(x.count) ?? 1) }];
  });
}
