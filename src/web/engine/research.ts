// Source-grounded research for Genius Web: the sources ledger rules, the citation validator and the
// safety helpers around untrusted web text. Mirrors crates/genius-core/src/research.rs.
//  * Only pages that were REALLY fetched are sources. Search results are leads, never citations.
//  * A report passes only when every factual sentence cites a ledger entry (or is labelled an
//    assumption / uncertain), every citation number exists, and every URL in it was fetched.
//  * Nothing is auto-"fixed" by inventing sources.

import { isPublicHttpUrl } from '../../lib/platform';

export interface Source {
  n: number;
  url: string;
  requestedUrl: string;
  title: string;
  status: number;
  fetchedAt: string;
  contentHash: string;
  snippet: string;
  usedFor: string;
  official: boolean;
  flags: string[];
}

export function normUrl(u: string): string {
  const t = u.trim().replace(/[.,;:!?]+$/, '');
  try {
    const url = new URL(t);
    url.hash = '';
    let s = url.toString();
    if (s.endsWith('/') && !url.search && (url.pathname === '/' || url.pathname.length > 1)) s = s.slice(0, -1);
    return s;
  } catch {
    return t;
  }
}

export async function contentHash(text: string): Promise<string> {
  const data = new TextEncoder().encode(text);
  const d = await crypto.subtle.digest('SHA-256', data);
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, '0')).join('');
}

export function snippetOf(text: string, max = 280): string {
  const one = text.split(/\s+/).filter(Boolean).join(' ');
  return one.length <= max ? one : `${one.slice(0, max)}…`;
}

// ---------------------------------------------------------------------------------------- official hosts

const OFFICIAL: [string[], string][] = [
  [['react', 'jsx', 'hook', 'usestate', 'useeffect'], 'react.dev'],
  [['vite'], 'vite.dev'],
  [['typescript', 'tsconfig'], 'www.typescriptlang.org'],
  [['javascript', 'css', 'html', 'web api', 'dom', 'mdn', 'flexbox', 'grid'], 'developer.mozilla.org'],
  [['node', 'node.js', 'nodejs', 'npm'], 'nodejs.org'],
  [['rust', 'cargo', 'crate', 'tokio'], 'doc.rust-lang.org'],
  [['python', 'pip', 'asyncio'], 'docs.python.org'],
  [['tauri'], 'tauri.app'],
  [['supabase'], 'supabase.com'],
  [['postgres', 'postgresql'], 'www.postgresql.org'],
  [['git', 'rebase'], 'git-scm.com'],
  [['github', 'actions'], 'docs.github.com'],
  [['docker', 'dockerfile'], 'docs.docker.com'],
  [['netlify'], 'docs.netlify.com'],
];

export function officialDomains(question: string): [string, string][] {
  const q = ` ${question.toLowerCase().replace(/[(),?:;"]/g, ' ')} `;
  const out: [string, string][] = [];
  for (const [words, domain] of OFFICIAL) {
    const w = words.find((x) => q.includes(` ${x} `) || (x.length > 4 && q.includes(x)));
    if (w && !out.some(([, d]) => d === domain)) out.push([w, domain]);
  }
  return out.slice(0, 6);
}

export function isOfficialHost(host: string): boolean {
  const h = host.toLowerCase().replace(/\.$/, '');
  return (
    OFFICIAL.some(([, d]) => h === d || h.endsWith(`.${d}`) || (d.startsWith('www.') && h === d.slice(4))) ||
    /^(docs|developer|developers|learn)\./.test(h) ||
    /\.(gov|edu)$/.test(h) ||
    /\.(gov|ac)\.uk$/.test(h) ||
    ['w3.org', 'ietf.org', 'www.rfc-editor.org', 'datatracker.ietf.org', 'whatwg.org', 'html.spec.whatwg.org', 'ecma-international.org', 'arxiv.org'].includes(h)
  );
}

// ---------------------------------------------------------------------------------------- untrusted content

const INJECTION: RegExp[] = [
  /ignore\s+(all\s+|any\s+|the\s+)?(previous|prior|above|earlier|system)\s+(instructions?|rules?|prompts?|messages?)/i,
  /disregard\s+(all\s+|any\s+|the\s+|your\s+)?(previous|prior|above|earlier|system|safety)/i,
  /forget\s+(everything|all)\s+(you|above|previous)/i,
  /you\s+are\s+now\s+(a|an|in)\b/i,
  /(new|updated|override)\s+(system\s+)?(instructions?|prompt|rules?)\s*:/i,
  /(reveal|print|show|repeat|leak)\s+(your|the)\s+(system\s+prompt|hidden|instructions|api\s*key|secret)/i,
  /(send|post|upload|exfiltrate|transmit)\s+.{0,60}(api[\s_-]*key|password|secret|token|credentials|\.env)/i,
  /do\s+not\s+(tell|inform|mention\s+this\s+to)\s+the\s+user/i,
  /<\s*\/?\s*(tool|tool_call|function_call|final|invoke)\b/i,
];

/** Phrases in web text that address the AI instead of the reader. The text stays data either way. */
export function injectionSignals(text: string): string[] {
  const out: string[] = [];
  for (const re of INJECTION) {
    const m = re.exec(text);
    if (m) {
      const s = m[0].split(/\s+/).join(' ').slice(0, 80);
      if (!out.includes(s)) out.push(s);
    }
    if (out.length >= 5) break;
  }
  return out;
}

/** Break up tool/final markup inside page text, so a quote of it can never be parsed as a call. */
export function defang(text: string): string {
  return text.replace(/<\s*(\/?\s*(?:tool|tool_call|tool_result|function_call|function|final|invoke|ask)\b)/gi, '‹$1').replace(/<<<\s*(FILE|DELETE|END FILE)/g, '‹‹‹$1');
}

// ---------------------------------------------------------------------------------------- validator

export interface Validation {
  ok: boolean;
  facts: number;
  assumptions: number;
  uncertain: number;
  uncited: string[];
  unknownCitations: number[];
  unfetchedUrls: string[];
  cited: number[];
  repaired: boolean;
}

const CITE = /\[(\d{1,3}(?:\s*,\s*\d{1,3})*)\]/g;
const URL_RE = /https?:\/\/[^\s<>"'`)\]]+/g;
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/;
const LIST_MARK = /^\s*(?:[-*+•]|\d+[.)])\s+/;
const ASSUME = /^\W*(assumption|assuming|assumed|my assumption)\b|[[(]\s*assum(?:ption|ed)\s*[\])]/i;
const UNCERTAIN = /^\W*(uncertain|unverified|unclear|unknown|not verified|could not verify|couldn.t verify)\b|[[(]\s*(?:uncertain|unverified)\s*[\])]/i;

type Section = 'normal' | 'assumptions' | 'uncertain' | 'sources';
function sectionOf(heading: string): Section {
  const h = heading.toLowerCase().replace(/^[^a-z0-9]+|[^a-z0-9]+$/g, '');
  if (/^(source|reference|bibliograph)/.test(h)) return 'sources';
  if (h.includes('assumption')) return 'assumptions';
  if (['uncertain', 'open question', 'limitation', 'caveat', 'not verified', 'unknown', 'gaps'].some((k) => h.includes(k))) return 'uncertain';
  return 'normal';
}

/** Split prose into sentences; a citation group after the terminator stays with its sentence. */
export function sentences(line: string): string[] {
  const out: string[] = [];
  let start = 0;
  let i = 0;
  const closers = new Set([')', '"', '*', '_', '”', '’']);
  while (i < line.length) {
    const c = line[i];
    const nx = line[i + 1];
    if ((c === '.' || c === '!' || c === '?') && (nx === undefined || /\s/.test(nx) || '[)"*_'.includes(nx))) {
      let j = i + 1;
      for (;;) {
        while (j < line.length && closers.has(line[j])) j++;
        let k = j;
        while (line[k] === ' ') k++;
        if (line[k] === '[') {
          const end = line.indexOf(']', k);
          const inner = end > k ? line.slice(k + 1, end) : '';
          if (inner && /^[\d, ]+$/.test(inner)) {
            j = end + 1;
            continue;
          }
        }
        break;
      }
      let n = j;
      while (n < line.length && /\s/.test(line[n])) n++;
      const boundary = n >= line.length || /[A-Z0-9[(*_"`•-]/.test(line[n]) || line[n] !== line[n].toLowerCase();
      if (boundary && (n > j || n >= line.length)) {
        const s = line.slice(start, j).trim();
        if (s) out.push(s);
        start = n;
        i = n;
        continue;
      }
    }
    i++;
  }
  const tail = line.slice(start).trim();
  if (tail) out.push(tail);
  return out;
}

const wordCount = (s: string) => s.replace(CITE, ' ').split(/\s+/).filter((w) => /[\p{L}\p{N}]/u.test(w)).length;

/** Citation numbers in a sentence, ignoring Markdown links like `[1](url)`. */
export function citationsIn(s: string): number[] {
  const out: number[] = [];
  for (const m of s.matchAll(CITE)) {
    if (s[m.index! + m[0].length] === '(') continue;
    for (const p of m[1].split(',')) {
      const n = parseInt(p.trim(), 10);
      if (!Number.isNaN(n)) out.push(n);
    }
  }
  return out;
}

export interface SourceRef {
  n: number;
  urls: string[];
}

export function validate(report: string, sources: SourceRef[]): Validation {
  const known = new Set(sources.map((s) => s.n));
  const fetched = new Set(sources.flatMap((s) => s.urls.map(normUrl)));
  const v: Validation = { ok: false, facts: 0, assumptions: 0, uncertain: 0, uncited: [], unknownCitations: [], unfetchedUrls: [], cited: [], repaired: false };
  const cited = new Set<number>();
  const unknown = new Set<number>();
  let section: Section = 'normal';
  let inCode = false;
  const lines = report.split(/\r?\n/);
  const isSep = (l: string) => {
    const t = l.trim();
    return t.includes('-') && t.includes('|') && /^[|\-: ]+$/.test(t);
  };
  lines.forEach((raw, idx) => {
    const line = raw.trimEnd();
    if (line.trimStart().startsWith('```')) {
      inCode = !inCode;
      return;
    }
    if (inCode || !line.trim()) return;
    const h = HEADING.exec(line);
    if (h) {
      section = sectionOf(h[1]);
      return;
    }
    for (const m of line.matchAll(URL_RE)) {
      const u = normUrl(m[0]);
      if (!fetched.has(u) && !v.unfetchedUrls.includes(u)) v.unfetchedUrls.push(u);
    }
    if (section === 'sources') return;
    let text = line.trim();
    const isTable = text.startsWith('|');
    if (isTable) {
      if (isSep(text) || (lines[idx + 1] !== undefined && isSep(lines[idx + 1]))) return;
      text = text.replace(/^\||\|$/g, '').split('|').map((c) => c.trim()).join(' — ');
    } else {
      text = text.replace(LIST_MARK, '').replace(/^>+/, '').trim();
    }
    for (const sent of isTable ? [text] : sentences(text)) {
      const nums = citationsIn(sent);
      for (const n of nums) (known.has(n) ? cited : unknown).add(n);
      const plain = sent.replace(/^[*_ ]+|[*_ ]+$/g, '');
      const urlOnly = /https?:\/\//.test(plain) && wordCount(plain.replace(URL_RE, ' ')) < 4;
      const exempt = wordCount(sent) < 6 || plain.endsWith('?') || plain.endsWith(':') || urlOnly;
      if (nums.some((n) => known.has(n))) v.facts++;
      else if (section === 'assumptions' || ASSUME.test(plain)) v.assumptions++;
      else if (section === 'uncertain' || UNCERTAIN.test(plain)) v.uncertain++;
      else if (!exempt && nums.length === 0) v.uncited.push(sent.slice(0, 220));
    }
  });
  v.cited = [...cited].sort((a, b) => a - b);
  v.unknownCitations = [...unknown].sort((a, b) => a - b);
  v.ok = v.uncited.length === 0 && v.unknownCitations.length === 0 && v.unfetchedUrls.length === 0;
  return v;
}

export function validationFeedback(v: Validation): string {
  let s = 'CITATION CHECK FAILED. Rewrite the complete report so that it passes. Rules: every factual sentence ends with a citation like [2] that refers to a SOURCE number you were given by a fetch_url result; a sentence that is your own inference starts with "Assumption:"; a sentence you could not verify starts with "Uncertain:". Do not cite anything you did not fetch, do not invent sources, and do not write a Sources section (Genius adds it). You may not call tools now; if a claim cannot be supported by the fetched pages, delete it or label it.\n';
  if (v.uncited.length) s += `\nClaims without a source:\n${v.uncited.map((u) => `- ${u}`).join('\n')}\n`;
  if (v.unknownCitations.length) s += `\nCitation numbers that do not exist: ${v.unknownCitations.join(', ')}\n`;
  if (v.unfetchedUrls.length) s += `\nURLs you never fetched:\n${v.unfetchedUrls.map((u) => `- ${u}`).join('\n')}\n`;
  return s;
}

export function validationSummary(v: Validation): string {
  if (v.ok) return `${v.facts} cited claim(s), every citation matches a fetched source`;
  const parts: string[] = [];
  if (v.uncited.length) parts.push(`${v.uncited.length} claim(s) without a source`);
  if (v.unknownCitations.length) parts.push(`${v.unknownCitations.length} citation(s) to sources that were never fetched`);
  if (v.unfetchedUrls.length) parts.push(`${v.unfetchedUrls.length} URL(s) that were never fetched`);
  return parts.join('; ');
}

export const issues = (v: Validation) => v.uncited.length + v.unknownCitations.length + v.unfetchedUrls.length;

// ---------------------------------------------------------------------------------------- report text

/** Links that point at the application itself are not sources (`[1](#src-1)`, `http://localhost…/#src-1`). */
export function stripAppLinks(report: string): string {
  const internal = (target: string) => {
    const t = target.trim();
    if (t.startsWith('#')) return true;
    try {
      const u = new URL(t);
      const sameApp = typeof location !== 'undefined' && u.host === location.host && /^#?src-/.test(u.hash.slice(1)) ;
      return sameApp || /(^|\.)(tauri|ipc|asset)\.localhost$/i.test(u.hostname) || /^#?src-\d+$/.test(u.hash.slice(1));
    } catch {
      return false;
    }
  };
  const linked = report.replace(/\[([^\]]*)\]\(\s*<?([^)\s>]*)>?[^)]*\)/g, (all, label: string, target: string) => (!internal(target) ? all : /^[\d, ]+$/.test(label) && label ? `[${label}]` : label));
  return linked.replace(/\(?https?:\/\/(?:[a-z0-9-]+\.)?(?:tauri|ipc|asset)\.localhost[^\s)>\]]*\)?/gi, '');
}

export function stripSourcesSection(report: string): string {
  const out: string[] = [];
  let skipping = false;
  let inCode = false;
  for (const line of report.split(/\r?\n/)) {
    if (line.trimStart().startsWith('```')) inCode = !inCode;
    if (!inCode) {
      const h = HEADING.exec(line);
      if (h) skipping = sectionOf(h[1]) === 'sources';
    }
    if (!skipping) out.push(line);
  }
  return out.join('\n').trimEnd();
}

export function withSourcesSection(report: string, ledger: Source[], cited: number[]): string {
  const base = stripSourcesSection(report);
  const used = ledger.filter((s) => cited.includes(s.n));
  if (!used.length) return base;
  const lines = used.map((s) => `[${s.n}] ${s.title.trim().replace(/\n/g, ' ') || s.url} — ${s.url} (fetched ${s.fetchedAt.slice(0, 10)})`);
  return `${base}\n\n## Sources\n${lines.join('\n')}`.trimEnd();
}

/** The first sentence of the report that cites `n` (how the source was used). */
export function firstUse(report: string, n: number): string {
  for (const line of report.split(/\r?\n/)) {
    for (const s of sentences(line.trim().replace(LIST_MARK, ''))) {
      if (citationsIn(s).includes(n)) return s.replace(CITE, '').split(/\s+/).filter(Boolean).join(' ').replace(/ \./g, '.').replace(/ ,/g, ',').slice(0, 160);
    }
  }
  return '';
}

/** Whether a URL may be shown to the user as a source link. */
export const linkable = (url: string) => isPublicHttpUrl(url);
