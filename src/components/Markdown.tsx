import { Fragment, useState, type ReactNode } from 'react';
import { Check, Copy } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';

/**
 * Small, safe Markdown renderer for chat messages: builds React elements only (never
 * innerHTML), so model output can't inject markup. Supports fenced code, headings, lists,
 * quotes, bold/italic, inline code and https links.
 */
export function Markdown({ text }: { text: string }) {
  return <div className="md">{blocks(text)}</div>;
}

type Block = { kind: 'code'; lang: string; body: string } | { kind: 'h'; level: number; body: string } | { kind: 'ul' | 'ol'; items: string[] } | { kind: 'quote'; body: string } | { kind: 'p'; body: string } | { kind: 'hr' };

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
    const listKind = /^\s*[-*+]\s+/.test(line) ? 'ul' : /^\s*\d+[.)]\s+/.test(line) ? 'ol' : null;
    if (listKind) {
      const items: string[] = [];
      const re = listKind === 'ul' ? /^\s*[-*+]\s+(.*)$/ : /^\s*\d+[.)]\s+(.*)$/;
      while (i < lines.length && lines[i].trim()) {
        const m = lines[i].match(re);
        if (m) items.push(m[1]);
        else if (items.length && /^\s{2,}\S/.test(lines[i])) items[items.length - 1] += ' ' + lines[i].trim();
        else break;
        i++;
      }
      out.push({ kind: listKind, items });
      continue;
    }
    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) body.push(lines[i++].replace(/^\s*>\s?/, ''));
      out.push({ kind: 'quote', body: body.join('\n') });
      continue;
    }
    const body: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^\s*```/.test(lines[i]) && !/^#{1,4}\s/.test(lines[i]) && !/^\s*([-*+]|\d+[.)])\s+/.test(lines[i])) body.push(lines[i++]);
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
      default:
        return <p key={i}>{inline(b.body)}</p>;
    }
  });
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

const TOKEN = /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*\s][^*]*\*)|(_[^_\s][^_]*_)|(\[[^\]]+\]\((https?:\/\/[^)\s]+)\))|(https?:\/\/[^\s)]+)/g;

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
