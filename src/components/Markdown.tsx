import { Fragment, useState, type ReactNode } from 'react';
import { Check, Copy } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import katex from 'katex';
import 'katex/dist/katex.min.css';

/**
 * Small, safe Markdown renderer for chat messages: builds React elements only (never
 * innerHTML, except for KaTeX output), so model output can't inject markup. Supports fenced code, headings, lists,
 * quotes, bold/italic, inline code and https links. Mathematics (`$…$`, `$$…$$`, `\(…\)`,
 * `\[…\]`) is typeset by KaTeX with `trust: false` (no links, no raw HTML), so a formula in a
 * model answer can't inject markup either.
 */
export function Markdown({ text }: { text: string }) {
  return <div className="md">{blocks(text)}</div>;
}

type Block = { kind: 'code'; lang: string; body: string } | { kind: 'h'; level: number; body: string } | { kind: 'ul' | 'ol'; items: string[] } | { kind: 'quote'; body: string } | { kind: 'p'; body: string } | { kind: 'hr' } | { kind: 'table'; head: string[]; rows: string[][] } | { kind: 'foot'; items: { n: string; text: string }[] } | { kind: 'math'; body: string };

const cells = (l: string) => l.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim());
const isSep = (l: string | undefined) => !!l && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(l) && l.includes('-');
const isTableStart = (lines: string[], i: number) => lines[i].includes('|') && isSep(lines[i + 1]);
const FOOT = /^\s*\[(\d{1,3})\]:?\s+(\S.*)$/;

export function parseBlocks(text: string): Block[] {
  const lines = text.replace(/\r\n/g, '\n').split('\n');
  const out: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*```\s*([\w+-]*)/);
    if (fence) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```/.test(lines[i])) body.push(lines[i++]);
      i++;
      out.push({ kind: 'code', lang: fence[1], body: body.join('\n') });
      continue;
    }
    if (!line.trim()) {
      i++;
      continue;
    }
    const mathStart = line.match(/^\s*(\$\$|\\\[)(.*)$/);
    if (mathStart) {
      const close = mathStart[1] === '$$' ? '$$' : '\\]';
      const first = mathStart[2];
      const at = first.indexOf(close);
      if (at >= 0 && !first.slice(at + close.length).trim()) {
        out.push({ kind: 'math', body: first.slice(0, at).trim() });
        i++;
        continue;
      }
      if (at < 0) {
        const body = [first];
        let j = i + 1;
        while (j < lines.length && !lines[j].includes(close)) body.push(lines[j++]);
        if (j < lines.length && !lines[j].slice(lines[j].indexOf(close) + close.length).trim()) {
          body.push(lines[j].slice(0, lines[j].indexOf(close)));
          out.push({ kind: 'math', body: body.join('\n').trim() });
          i = j + 1;
          continue;
        }
      }
    }
    if (isTableStart(lines, i)) {
      const head = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() && lines[i].includes('|')) rows.push(cells(lines[i++]));
      out.push({ kind: 'table', head, rows });
      continue;
    }
    if (FOOT.test(line)) {
      const items: { n: string; text: string }[] = [];
      while (i < lines.length) {
        const m = lines[i].match(FOOT);
        if (!m) break;
        items.push({ n: m[1], text: m[2] });
        i++;
      }
      out.push({ kind: 'foot', items });
      continue;
    }
    const h = line.match(/^(#{1,4})\s+(.*)$/);
    if (h) {
      out.push({ kind: 'h', level: h[1].length, body: h[2] });
      i++;
      continue;
    }
    if (/^\s*(-{3,}|\*{3,})\s*$/.test(line)) {
      out.push({ kind: 'hr' });
      i++;
      continue;
    }
    const listKind = /^\s*[-*+•]\s+/.test(line) ? 'ul' : /^\s*\d+[.)]\s+/.test(line) ? 'ol' : null;
    if (listKind) {
      const items: string[] = [];
      const re = listKind === 'ul' ? /^\s*[-*+•]\s+(.*)$/ : /^\s*\d+[.)]\s+(.*)$/;
      while (i < lines.length && lines[i].trim()) {
        const m = lines[i].match(re);
        const last = items.length - 1;
        if (m) items.push(m[1].trim());
        // Continuation lines, and the text of a bare marker ("1. " then the item on the next line).
        else if (items.length && (/^\s{2,}\S/.test(lines[i]) || (!items[last] && !/^\s*(```|#{1,4}\s)/.test(lines[i])))) items[last] = `${items[last]} ${lines[i].trim()}`.trim();
        else break;
        i++;
      }
      // Never render empty bullets (models sometimes emit "- " or "1. " placeholders).
      const filled = items.filter((it) => it && !/^[-*+•.\s]*$/.test(it));
      if (filled.length) out.push({ kind: listKind, items: filled });
      continue;
    }
    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) body.push(lines[i++].replace(/^\s*>\s?/, ''));
      out.push({ kind: 'quote', body: body.join('\n') });
      continue;
    }
    const body: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^\s*```/.test(lines[i]) && !/^#{1,4}\s/.test(lines[i]) && !/^\s*([-*+•]|\d+[.)])\s+/.test(lines[i]) && !isTableStart(lines, i)) body.push(lines[i++]);
    if (!body.length) body.push(lines[i++]);
    out.push({ kind: 'p', body: body.join('\n') });
  }
  return out;
}

function blocks(text: string): ReactNode {
  return parseBlocks(text).map((b, i) => {
    switch (b.kind) {
      case 'code':
        return <CodeBlock key={i} lang={b.lang} body={b.body} />;
      case 'h':
        return b.level <= 2 ? <h3 key={i}>{inline(b.body)}</h3> : <h4 key={i}>{inline(b.body)}</h4>;
      case 'ul':
        return <ul key={i}>{b.items.map((it, j) => <li key={j}>{inline(it)}</li>)}</ul>;
      case 'ol':
        return <ol key={i}>{b.items.map((it, j) => <li key={j}>{inline(it)}</li>)}</ol>;
      case 'quote':
        return <blockquote key={i}>{inline(b.body)}</blockquote>;
      case 'hr':
        return <hr key={i} />;
      case 'math':
        return <Formula key={i} tex={b.body} display />;
      case 'table':
        return (
          <div className="md-table" key={i}>
            <table>
              <thead>
                <tr>{b.head.map((c, j) => <th key={j}>{inline(c)}</th>)}</tr>
              </thead>
              <tbody>
                {b.rows.map((r, j) => (
                  <tr key={j}>{r.map((c, k) => <td key={k}>{inline(c)}</td>)}</tr>
                ))}
              </tbody>
            </table>
          </div>
        );
      case 'foot':
        return (
          <ol className="footnotes" key={i} aria-label="Sources">
            {b.items.map((it) => (
              <li key={it.n} id={`src-${it.n}`} value={Number(it.n)}>
                {inline(it.text)}
              </li>
            ))}
          </ol>
        );
      default:
        return <p key={i}>{inline(b.body)}</p>;
    }
  });
}

/** One formula. KaTeX renders what it can; on a parse error it shows the source in red instead of throwing. */
export function Formula({ tex, display }: { tex: string; display?: boolean }) {
  const html = katex.renderToString(tex, { displayMode: !!display, throwOnError: false, trust: false, strict: 'ignore', output: 'htmlAndMathml', maxSize: 20, maxExpand: 500 });
  return display ? <div className="md-math" role="math" aria-label={tex} dangerouslySetInnerHTML={{ __html: html }} /> : <span className="md-math-inline" aria-label={tex} dangerouslySetInnerHTML={{ __html: html }} />;
}

function CodeBlock({ lang, body }: { lang: string; body: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="md-code">
      <div className="md-code-head">
        <span>{lang || 'code'}</span>
        <button
          className="icon-btn"
          style={{ width: 22, height: 22 }}
          aria-label="Copy code"
          title="Copy"
          onClick={() => {
            void navigator.clipboard?.writeText(body);
            setCopied(true);
            setTimeout(() => setCopied(false), 1200);
          }}
        >
          {copied ? <Check size={12} /> : <Copy size={12} />}
        </button>
      </div>
      <pre>{body}</pre>
    </div>
  );
}

const TOKEN = /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*\s][^*]*\*)|(_[^_\s][^_]*_)|(\[[^\]]+\]\((https?:\/\/[^)\s]+)\))|(https?:\/\/[^\s)]+)|((?<!\w)\[\d{1,3}\](?!\())|(\$(?![\s$])[^$\n`]+?(?<![\s\\])\$(?!\d))|(\\\([^\n]+?\\\))/g;

export function inline(text: string): ReactNode {
  const parts: ReactNode[] = [];
  let last = 0;
  let k = 0;
  for (const m of text.matchAll(TOKEN)) {
    const at = m.index ?? 0;
    if (at > last) parts.push(withBreaks(text.slice(last, at), k++));
    const t = m[0];
    if (m[1]) parts.push(<code key={k++}>{t.slice(1, -1)}</code>);
    else if (m[2] || m[3]) parts.push(<strong key={k++}>{t.slice(2, -2)}</strong>);
    else if (m[4] || m[5]) parts.push(<em key={k++}>{t.slice(1, -1)}</em>);
    else if (m[6]) parts.push(<Link key={k++} href={m[7]}>{t.slice(1, t.indexOf(']'))}</Link>);
    else if (m[8]) parts.push(<Link key={k++} href={t}>{t}</Link>);
    else if (m[9]) {
      const n = t.slice(1, -1);
      parts.push(
        <sup key={k++} className="cite">
          <a href={`#src-${n}`} title={`Source ${n}`} onClick={(e) => { e.preventDefault(); window.dispatchEvent(new CustomEvent('genius-cite', { detail: Number(n) })); }}>
            {n}
          </a>
        </sup>,
      );
    }
    else if (m[10]) parts.push(<Formula key={k++} tex={t.slice(1, -1)} />);
    else if (m[11]) parts.push(<Formula key={k++} tex={t.slice(2, -2)} />);
    last = at + t.length;
  }
  if (last < text.length) parts.push(withBreaks(text.slice(last), k++));
  return parts;
}

function withBreaks(s: string, key: number) {
  const lines = s.split('\n');
  return (
    <Fragment key={key}>
      {lines.map((l, i) => (
        <Fragment key={i}>
          {i > 0 && <br />}
          {l}
        </Fragment>
      ))}
    </Fragment>
  );
}

function Link({ href, children }: { href: string; children: ReactNode }) {
  return (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        void openUrl(href).catch(() => window.open(href, '_blank', 'noopener'));
      }}
      title={href}
    >
      {children}
    </a>
  );
}
