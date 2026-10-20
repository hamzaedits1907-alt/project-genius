// Tool-call syntax for Genius Web: the same text protocol as the desktop core (crates/genius-core/src/agent.rs
// and toolcall.rs), so the models behave identically. Models write tool calls in many dialects; everything
// here turns them into one canonical `{name, args}` BEFORE anything runs, and keeps raw call syntax out of
// answers. Pure functions, no I/O.

export interface ToolCall {
  name: string;
  args: Record<string, unknown>;
}
export interface InvalidCall {
  invalid: string;
  tool?: string;
}
export type Action = ToolCall | InvalidCall;
export const isInvalid = (a: Action): a is InvalidCall => 'invalid' in a;

export interface Turn {
  calls: Action[];
  /** The answer when the turn makes no call. */
  final: string | null;
}

/** Tools the browser runtime can run. */
export const WEB_TOOL_NAMES = ['web_search', 'fetch_url', 'calculate', 'data_summary', 'data_query', 'todo', 'search_references', 'read_reference', 'runtime_info', 'capabilities'] as const;

const ARG_ALIASES: Record<string, Record<string, string[]>> = {
  web_search: { query: ['q', 'text', 'search', 'term', 'keywords', 'queries'] },
  fetch_url: { url: ['link', 'href', 'uri', 'address', 'urls'] },
  calculate: { expression: ['expr', 'formula', 'input', 'query', 'math', 'calculation'] },
  data_summary: { id: ['file', 'file_id', 'fileId', 'reference', 'attachment'], column: ['col', 'field'] },
  data_query: { id: ['file', 'file_id', 'fileId', 'reference', 'attachment'], where: ['filter', 'filters', 'conditions'], group_by: ['groupBy', 'group', 'by'], metrics: ['aggregate', 'aggregates', 'aggregations', 'agg'], sort_by: ['sortBy', 'sort', 'order_by', 'orderBy'], columns: ['select', 'fields'] },
  search_references: { query: ['q', 'text', 'search', 'term'] },
  read_reference: { id: ['reference', 'ref', 'reference_id'] },
  todo: { items: ['todos', 'list', 'tasks', 'steps'] },
};
const PRIMARY: Record<string, string> = { web_search: 'query', fetch_url: 'url', calculate: 'expression', data_summary: 'id', data_query: 'id', search_references: 'query', read_reference: 'id', todo: 'items' };
const TOOL_SYNONYMS: Record<string, string[]> = {
  web_search: ['websearch', 'search_web', 'internet_search', 'google', 'browse_search', 'search_the_web'],
  fetch_url: ['fetchurl', 'open_url', 'read_url', 'visit', 'browse', 'get_url', 'fetch_page', 'read_page', 'open_page'],
  calculate: ['calc', 'calculator', 'math', 'evaluate'],
  search_references: ['searchreferences', 'search_files', 'search_attachments'],
  read_reference: ['readreference', 'read_file', 'open_reference', 'read_attachment'],
};

/** Canonical tool name for what the model wrote (`functions.web_search`, `Web-Search`, `websearch`). */
export function canonicalTool(name: string, known: readonly string[]): string {
  let raw = name.trim().replace(/^["'`]+|["'`]+$/g, '');
  raw = raw.split(/[.:/]/).pop() ?? raw;
  if (known.includes(raw)) return raw;
  const snake = raw.replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/[-\s]+/g, '_').toLowerCase();
  if (known.includes(snake)) return snake;
  const flat = snake.replace(/_/g, '');
  const hit = known.find((k) => k.replace(/_/g, '') === flat);
  if (hit) return hit;
  for (const [canon, aliases] of Object.entries(TOOL_SYNONYMS)) if (known.includes(canon) && aliases.some((a) => a === snake || a.replace(/_/g, '') === flat)) return canon;
  return raw;
}

// ------------------------------------------------------------------------------------------ JSON

/** Length of the balanced JSON object/array starting `s` (string- and escape-aware), or null. */
export function balancedJsonLength(s: string): number | null {
  let depth = 0;
  let inStr = false;
  let esc = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (inStr) {
      if (esc) esc = false;
      else if (c === '\\') esc = true;
      else if (c === '"') inStr = false;
      continue;
    }
    if (c === '"') inStr = true;
    else if (c === '{' || c === '[') depth++;
    else if (c === '}' || c === ']') {
      depth--;
      if (depth < 0) return null;
      if (depth === 0) return i + 1;
    }
  }
  return null;
}

function repairJson(s: string, quotes: boolean): string {
  let out = '';
  let inStr = false;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (!inStr) {
      if (c === '"') inStr = true;
      if (c === ',') {
        const rest = s.slice(i + 1).trimStart();
        if (rest.startsWith('}') || rest.startsWith(']')) continue; // trailing comma
      }
      out += c;
      continue;
    }
    if (c === '\\') {
      const n = s[i + 1];
      if (n !== undefined && '"\\/bfnrtu'.includes(n)) {
        out += c + n;
        i++;
      } else out += '\\\\';
      continue;
    }
    if (c === '"') {
      if (quotes) {
        const next = s.slice(i + 1).trimStart()[0];
        if (next !== undefined && !',}]:'.includes(next)) {
          out += '\\"'; // a quote inside a string
          continue;
        }
      }
      inStr = false;
      out += c;
      continue;
    }
    if (c === '\n') out += '\\n';
    else if (c === '\r') out += '\\r';
    else if (c === '\t') out += '\\t';
    else out += c;
  }
  return out;
}

/** Parse JSON the way models write it: in code fences, followed by prose, with raw newlines, trailing commas, unescaped quotes. */
export function lenientJson(text: string): unknown {
  let t = text.trim();
  t = t.replace(/^```(?:json|JSON)?/, '').replace(/```$/, '').trim();
  const start = t.search(/[{[]/);
  if (start < 0) throw new Error('no JSON value');
  t = t.slice(start);
  const len = balancedJsonLength(t) ?? t.length;
  for (const candidate of [t.slice(0, len), t]) {
    for (const quotes of [false, true]) {
      for (const src of [candidate, repairJson(candidate, quotes)]) {
        try {
          return JSON.parse(src);
        } catch {
          /* try the next repair */
        }
      }
    }
  }
  // a prefix that is a valid JSON value followed by prose
  const len2 = balancedJsonLength(repairJson(t, true));
  if (len2) return JSON.parse(repairJson(t, true).slice(0, len2));
  throw new Error('not valid JSON');
}

const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

// ------------------------------------------------------------------------------------------ arguments

export function cleanUrl(raw: string): string {
  let t = raw.trim().replace(/^["'`]+|["'`]+$/g, '').trim();
  const md = /^\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[^)]*)?\)/.exec(t);
  if (md) t = md[1];
  t = t.replace(/^</, '').replace(/>$/, '');
  const embedded = /https?:\/\/[^\s<>"']+/.exec(t);
  t = embedded && !t.startsWith('http') ? embedded[0] : (t.split(/\s+/)[0] ?? '');
  for (;;) {
    const last = t[t.length - 1];
    const strip = last !== undefined && ('.,;:!?"\''.includes(last) || (last === ')' && (t.match(/\)/g)?.length ?? 0) > (t.match(/\(/g)?.length ?? 0)));
    if (!strip) break;
    t = t.slice(0, -1);
  }
  const schemeLike = /^[a-z][a-z0-9+.-]*:/i.test(t) && (t.includes('://') || /^(mailto|data|javascript):/i.test(t));
  if (t && !schemeLike && t.includes('.') && !t.startsWith('/')) t = `https://${t}`;
  return t;
}

function decodeNested(v: unknown): unknown {
  if (typeof v === 'string' && /^\s*[[{]/.test(v)) {
    try {
      return lenientJson(v);
    } catch {
      return v;
    }
  }
  return v;
}

/** Normalise a tool's arguments to the canonical shape. */
export function normalizeCall(name: string, rawArgs: unknown): ToolCall {
  let args: Record<string, unknown>;
  const decoded = decodeNested(rawArgs);
  if (decoded === null || decoded === undefined || (typeof decoded === 'string' && !decoded.trim())) args = {};
  else if (isObj(decoded)) args = { ...decoded };
  else args = PRIMARY[name] ? { [PRIMARY[name]]: decoded } : {};
  for (const [canon, aliases] of Object.entries(ARG_ALIASES[name] ?? {})) {
    if (canon in args) continue;
    const a = aliases.find((x) => x in args);
    if (a) {
      args[canon] = args[a];
      delete args[a];
    }
  }
  if (name === 'fetch_url') {
    if (Array.isArray(args.url)) args.url = args.url.find((x) => typeof x === 'string');
    if (typeof args.url === 'string') args.url = cleanUrl(args.url);
  }
  if (name === 'web_search') {
    if (Array.isArray(args.query)) args.query = args.query.find((x) => typeof x === 'string');
    if (typeof args.query === 'string') args.query = args.query.trim().replace(/^["']+|["']+$/g, '').trim();
  }
  if (name === 'data_query') {
    for (const k of ['where', 'metrics', 'columns']) {
      const v = decodeNested(args[k]);
      if (v !== undefined) args[k] = v;
    }
    if (isObj(args.where)) args.where = [args.where];
    if (isObj(args.metrics)) args.metrics = [args.metrics];
    if (typeof args.columns === 'string') args.columns = args.columns.split(',').map((x: string) => x.trim()).filter(Boolean);
    if (typeof args.desc === 'string') args.desc = args.desc.toLowerCase() === 'true';
    if (typeof args.limit === 'string') args.limit = Number(args.limit);
  }
  if (name === 'todo') {
    const items = decodeNested(args.items);
    if (Array.isArray(items)) args.items = items.map((i) => (typeof i === 'string' ? { text: i } : isObj(i) ? { ...i, text: i.text ?? i.content ?? i.title ?? i.task ?? i.description ?? i.name } : i));
  }
  return { name, args };
}

/** A protocol problem with a call's arguments (null = fine). */
export function validateCall(c: ToolCall): string | null {
  const str = (k: string) => (typeof c.args[k] === 'string' && (c.args[k] as string).trim() ? null : `"${k}" is missing or empty`);
  switch (c.name) {
    case 'web_search':
      return str('query');
    case 'fetch_url':
      return str('url');
    case 'calculate':
      return str('expression');
    case 'data_summary':
    case 'data_query':
    case 'read_reference':
      return str('id');
    case 'search_references':
      return str('query');
    case 'todo': {
      const it = c.args.items;
      return Array.isArray(it) && it.length > 0 && it.every((i) => isObj(i) && typeof i.text === 'string' && i.text.trim()) ? null : '"items" must be a non-empty list of {"text", "status"}';
    }
    default:
      return null;
  }
}

export function toolExample(name: string): string {
  const ex: Record<string, string> = {
    web_search: '{"query": "react server components"}',
    fetch_url: '{"url": "https://react.dev/reference/react/useState"}',
    calculate: '{"expression": "(1250 * 0.19) + sqrt(144)"}',
    data_summary: '{"id": "<file id>", "column": "price"}',
    data_query: '{"id": "<file id>", "where": [{"column": "year", "op": ">=", "value": 2024}], "group_by": "category", "metrics": [{"column": "amount", "fn": "sum"}], "sort_by": "sum(amount)", "limit": 10}',
    search_references: '{"query": "pricing"}',
    read_reference: '{"id": "<file id>"}',
    todo: '{"items": [{"text": "Search", "status": "active"}, {"text": "Write report", "status": "pending"}]}',
  };
  return `<tool>{"name": "${name}", "args": ${ex[name] ?? '{}'}}</tool>`;
}

// ------------------------------------------------------------------------------------------ call shapes

const NAME_KEYS = ['name', 'tool', 'tool_name', 'toolName', 'action', 'recipient_name'];
const ARGS_KEYS = ['args', 'arguments', 'parameters', 'input', 'params', 'action_input', 'tool_input', 'kwargs'];

function callFrom(v: unknown, known: readonly string[]): Action[] {
  if (Array.isArray(v)) return v.flatMap((x) => callFrom(x, known));
  if (!isObj(v)) return [{ invalid: 'a tool call must be a JSON object with "name" and "args"' }];
  if (!('name' in v)) for (const k of ['tool_calls', 'calls', 'tool_uses', 'function_calls']) if (Array.isArray(v[k])) return callFrom(v[k], known);
  const o = { ...v };
  let funcName: string | undefined;
  let funcArgs: unknown;
  if (typeof o.function === 'string') funcName = o.function;
  else if (isObj(o.function)) {
    funcName = typeof o.function.name === 'string' ? o.function.name : undefined;
    funcArgs = ARGS_KEYS.map((k) => (o.function as Record<string, unknown>)[k]).find((x) => x !== undefined);
  }
  const cands = [...NAME_KEYS.filter((k) => typeof o[k] === 'string').map((k) => o[k] as string), ...(funcName ? [funcName] : [])];
  const picked = cands.find((c) => known.includes(canonicalTool(c, known))) ?? cands[0];
  if (!picked) return [{ invalid: 'tool call JSON needs a string "name"' }];
  const name = canonicalTool(picked, known);
  let args: unknown = ARGS_KEYS.map((k) => o[k]).find((x) => x !== undefined) ?? funcArgs;
  if (typeof args === 'string' && /^\s*[[{]/.test(args)) {
    try {
      args = lenientJson(args);
    } catch (e) {
      return [{ invalid: `arguments of ${picked} are not valid JSON (${String(e)})`, tool: name }];
    }
  }
  if (args === undefined || args === null) {
    const rest = { ...o };
    for (const k of [...NAME_KEYS, 'function', 'type', 'id', 'index']) delete rest[k];
    args = rest;
  }
  return [normalizeCall(name, args)];
}

// ------------------------------------------------------------------------------------------ tags and blocks

/** Text with fenced code and inline code blanked out (offsets preserved). */
export function maskCode(text: string): string {
  let out = '';
  let rest = text;
  const blank = (s: string) => s.replace(/[^\n]/g, ' ');
  for (;;) {
    const i = rest.indexOf('`');
    if (i < 0) break;
    out += rest.slice(0, i);
    const after = rest.slice(i);
    const ticks = /^`+/.exec(after)![0].length;
    const fence = '`'.repeat(ticks);
    const j = after.indexOf(fence, ticks);
    if (j >= 0 && (ticks >= 3 || !after.slice(ticks, j).includes('\n'))) {
      const end = j + ticks;
      out += blank(after.slice(0, end));
      rest = after.slice(end);
    } else {
      out += after.slice(0, ticks);
      rest = after.slice(ticks);
    }
  }
  return out + rest;
}

interface TagCall {
  start: number;
  end: number;
  name: string;
  args: unknown;
  error?: string;
}

function parseAttrs(head: string): Record<string, unknown> | null {
  const re = /([A-Za-z_][\w-]*)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+))/g;
  const o: Record<string, unknown> = {};
  let covered = 0;
  for (const m of head.matchAll(re)) {
    const v = m[2] ?? m[3] ?? m[4] ?? '';
    covered += m[0].length;
    o[m[1]] = v === 'true' ? true : v === 'false' ? false : /^\d{1,9}$/.test(v) ? Number(v) : v;
  }
  return Object.keys(o).length && covered * 2 >= head.trim().length ? o : null;
}

function tagArgs(head: string, name: string): unknown | undefined {
  const h = head.trim().replace(/\/$/, '').trim();
  if (!h) return {};
  if (h.startsWith('{') || h.startsWith('[')) {
    try {
      const v = lenientJson(h);
      return isObj(v) ? v : PRIMARY[name] ? { [PRIMARY[name]]: v } : undefined;
    } catch {
      return undefined;
    }
  }
  const attrs = parseAttrs(h);
  if (attrs) return attrs;
  const p = PRIMARY[name];
  const bare = h.replace(/^["']+|["']+$/g, '');
  return p && bare ? { [p]: bare } : undefined;
}

/**
 * Tool-named tags: `<web_search {"query": "x"}>`, `<websearch {…}` (no `>`), `<fetch_url url="…"/>`,
 * `<todo>{…}</todo>`, `<web_search>text</web_search>`. A one-word name counts only when JSON follows it,
 * because such words are also ordinary HTML and prose.
 */
export function namedTagCalls(masked: string, known: readonly string[]): TagCall[] {
  const out: TagCall[] = [];
  const open = /<([A-Za-z][A-Za-z0-9_-]*)/g;
  let m: RegExpExecArray | null;
  while ((m = open.exec(masked))) {
    const start = m.index;
    const headStart = start + m[0].length;
    const canon = canonicalTool(m[1], known);
    if (!known.includes(canon)) continue;
    const rest = masked.slice(headStart);
    if (rest && !/^[\s>/{[]/.test(rest)) continue;
    const body = rest.trimStart();
    const lead = rest.length - body.length;
    const jsonNext = body.startsWith('{') || body.startsWith('[');
    const jsonAfterGt = /^>\s*[{[]/.test(body);
    if (!canon.includes('_') && !(jsonNext || jsonAfterGt)) continue;
    if (jsonNext) {
      const len = balancedJsonLength(body);
      const args = len ? tagArgs(body.slice(0, len), canon) : undefined;
      if (len && args !== undefined) {
        let end = headStart + lead + len;
        const tail = masked.slice(end);
        const t = tail.trimStart();
        const after = t.startsWith('/>') ? t.slice(2) : t.startsWith('>') ? t.slice(1) : null;
        if (after !== null) {
          end += tail.length - after.length;
          const close = `</${m[1]}>`;
          const trimmed = masked.slice(end).trimStart();
          if (trimmed.startsWith(close)) end = masked.length - (trimmed.length - close.length);
        }
        out.push({ start, end, name: canon, args });
        open.lastIndex = Math.max(end, start + 1);
        continue;
      }
    }
    // <name>inner</name> and attribute forms
    const gts: number[] = [];
    for (let i = headStart, n = 0; n < 60; n++) {
      const g = masked.indexOf('>', i);
      if (g < 0) break;
      gts.push(g);
      i = g + 1;
    }
    let found: { gt: number; args: unknown } | null = null;
    for (const gt of gts) {
      const a = tagArgs(masked.slice(headStart, gt), canon);
      if (a !== undefined) {
        found = { gt, args: a };
        break;
      }
    }
    if (!found) {
      const end = gts.length ? gts[0] + 1 : (() => { const nl = masked.indexOf('\n', headStart); return nl < 0 ? masked.length : nl; })();
      out.push({ start, end, name: canon, args: {}, error: `the arguments of ${canon} are not valid JSON` });
      open.lastIndex = Math.max(end, start + 1);
      continue;
    }
    let { args } = found;
    let end = found.gt + 1;
    const selfClosing = masked.slice(headStart, found.gt).trimEnd().endsWith('/');
    if (!selfClosing) {
      const close = `</${m[1]}>`;
      const c = masked.indexOf(close, end);
      if (c >= 0) {
        const inner = masked.slice(end, c).trim();
        if (inner && isObj(args) && Object.keys(args).length === 0) {
          const a = tagArgs(inner, canon);
          if (a === undefined) {
            out.push({ start, end: c + close.length, name: canon, args: {}, error: `the arguments of ${canon} are not valid JSON` });
            open.lastIndex = c + close.length;
            continue;
          }
          args = a;
        }
        end = c + close.length;
      }
    }
    out.push({ start, end, name: canon, args });
    open.lastIndex = Math.max(end, start + 1);
  }
  return out;
}

/** Replace tool-named tags by the canonical `<tool>{…}</tool>` form (null when there are none). */
function rewriteNamedTags(text: string, masked: string, known: readonly string[]): string | null {
  const calls = namedTagCalls(masked, known);
  if (!calls.length) return null;
  let out = '';
  let at = 0;
  for (const c of calls) {
    out += text.slice(at, c.start);
    out += c.error ? `<tool>${JSON.stringify({ name: c.name, arguments: '{' })}</tool>` : `<tool>${JSON.stringify({ name: c.name, args: c.args })}</tool>`;
    at = c.end;
  }
  return out + text.slice(at);
}

const CALL_TAGS: [string, string][] = [['<tool>', '</tool>'], ['<tool_call>', '</tool_call>'], ['<function_call>', '</function_call>'], ['<function=', '</function>'], ['<invoke', '</invoke>']];

function xmlParams(s: string): Record<string, unknown> {
  const RAW = ['content', 'text', 'query', 'url', 'expression'];
  const o: Record<string, unknown> = {};
  for (const m of s.matchAll(/<parameter(?:\s+name=["']?([\w.-]+)["']?|=([\w.-]+))\s*>([\s\S]*?)(?:<\/parameter>|(?=<parameter)|$)/g)) {
    const key = m[1] ?? m[2];
    const v = m[3].replace(/^\r?\n/, '').replace(/\r?\n$/, '');
    o[key] = RAW.includes(key) ? v : (() => { try { return /^[[{]/.test(v.trim()) || /^(true|false|\d+)$/.test(v.trim()) ? JSON.parse(v.trim()) : v.trim(); } catch { return v.trim(); } })();
  }
  return o;
}

/** First balanced-or-closed final answer markers. */
function finalBody(text: string, masked: string): string {
  const a = masked.indexOf('<final>');
  const b = masked.lastIndexOf('</final>');
  let t = text;
  if (a >= 0 && b > a) t = text.slice(a + 7, b);
  else if (a >= 0) t = text.slice(a + 7);
  else if (b >= 0) t = text.slice(0, b);
  return t.trim();
}

function parseCanonical(text: string, known: readonly string[]): Turn {
  const masked = text;
  const calls: Action[] = [];
  let pos = 0;
  for (;;) {
    let best: { i: number; open: string; close: string } | null = null;
    for (const [o, c] of CALL_TAGS) {
      const i = masked.indexOf(o, pos);
      if (i >= 0 && (!best || i < best.i)) best = { i, open: o, close: c };
    }
    if (!best) break;
    const bodyStart = best.i + best.open.length;
    let bodyEnd = masked.indexOf(best.close, bodyStart);
    let next: number;
    if (bodyEnd >= 0) next = bodyEnd + best.close.length;
    else {
      let nextTag = masked.length;
      for (const [o] of CALL_TAGS) {
        const i = masked.indexOf(o, bodyStart);
        if (i >= 0 && i < nextTag) nextTag = i;
      }
      bodyEnd = nextTag;
      next = nextTag;
    }
    const body = masked.slice(bodyStart, bodyEnd);
    if (best.open === '<function=') {
      const [head, ...restParts] = body.split('>');
      const rest = restParts.join('>');
      const name = canonicalTool(head.trim().replace(/^["']|["']$/g, ''), known);
      if (rest.includes('<parameter')) calls.push(normalizeCall(name, xmlParams(rest)));
      else if (!rest.trim()) calls.push(normalizeCall(name, {}));
      else {
        try {
          calls.push(normalizeCall(name, lenientJson(rest)));
        } catch (e) {
          calls.push({ invalid: `arguments of ${name} are not valid JSON (${String(e)})`, tool: name });
        }
      }
    } else if (best.open === '<invoke') {
      const gt = body.indexOf('>');
      const head = body.slice(0, gt);
      const rest = body.slice(gt + 1);
      const name = canonicalTool(head.replace(/name\s*=\s*/, '').trim().replace(/^["']|["']$/g, ''), known);
      calls.push(normalizeCall(name, rest.includes('<parameter') ? xmlParams(rest) : (() => { try { return lenientJson(rest); } catch { return {}; } })()));
    } else {
      const inner = body.trim();
      if (inner.startsWith('<function=')) {
        const sub = parseCanonical(inner + '</function>', known);
        calls.push(...sub.calls);
      } else {
        try {
          calls.push(...callFrom(lenientJson(body), known));
        } catch (e) {
          const guess = /"(?:name|tool|function)"\s*:\s*"([A-Za-z_][\w.-]*)/.exec(body);
          const tool = guess ? canonicalTool(guess[1], known) : undefined;
          calls.push({ invalid: `tool call is not valid JSON (${String(e)})`, tool: tool && known.includes(tool) ? tool : undefined });
        }
      }
    }
    pos = Math.max(next, bodyStart);
    if (pos >= masked.length) break;
  }
  if (!calls.length && !masked.includes('<final>')) {
    const t = text.trim();
    if (/^[{[`]/.test(t)) {
      try {
        const bare = callFrom(lenientJson(t), known);
        if (bare.length && bare.every((a) => !isInvalid(a) && known.includes(a.name))) return { calls: bare, final: null };
      } catch {
        const guess = /"(?:name|tool|function)"\s*:\s*"([A-Za-z_][\w.-]*)/.exec(t);
        const tool = guess ? canonicalTool(guess[1], known) : undefined;
        if (tool && known.includes(tool)) return { calls: [{ invalid: 'tool call is not valid JSON', tool }], final: null };
      }
    }
  }
  if (!calls.length) return { calls, final: finalBody(text, masked) };
  return { calls, final: null };
}

/** Parse one model turn: tool calls (in order, invalid ones as errors) or the final answer. */
export function parseTurn(text: string, known: readonly string[] = WEB_TOOL_NAMES): Turn {
  const rewritten = rewriteNamedTags(text, maskCode(text), known);
  return parseCanonical(rewritten ?? text, known);
}

// ------------------------------------------------------------------------------------------ residue

const RESIDUE = /<\s*\/?\s*(?:tool|tool_call|tool_result|function_call|function|invoke|parameter)\b/i;

/** Raw tool-call syntax left in a user-facing answer (outside code the user is meant to see). */
export function hasResidue(text: string, known: readonly string[] = WEB_TOOL_NAMES): boolean {
  const masked = maskCode(text);
  return RESIDUE.test(masked) || namedTagCalls(masked, known).length > 0;
}

/** The answer without protocol syntax. Code shown on purpose (fences, inline code) is left alone. */
export function stripResidue(text: string, known: readonly string[] = WEB_TOOL_NAMES): string {
  const masked = maskCode(text);
  const spans: [number, number][] = namedTagCalls(masked, known).map((c) => [c.start, c.end]);
  for (const m of masked.matchAll(/<\s*(tool_result|tool_call|tool|function_call|invoke)\b[\s\S]*?<\s*\/\s*(?:tool_result|tool_call|tool|function_call|invoke)\s*>/gi)) spans.push([m.index!, m.index! + m[0].length]);
  spans.sort((a, b) => a[0] - b[0]);
  let out = '';
  let at = 0;
  for (const [s, e] of spans) {
    if (s < at) continue;
    out += text.slice(at, s);
    at = e;
  }
  out += text.slice(at);
  const m2 = maskCode(out);
  let cleaned = '';
  let last = 0;
  for (const m of m2.matchAll(new RegExp(RESIDUE.source, 'gi'))) {
    if (m.index! < last) continue;
    const gt = m2.indexOf('>', m.index! + m[0].length);
    cleaned += out.slice(last, m.index!);
    last = gt < 0 ? m2.length : gt + 1;
  }
  cleaned += out.slice(last);
  return cleaned.replace(/\n{3,}/g, '\n\n').trim();
}
