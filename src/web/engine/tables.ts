// Statistics for the `data_summary` tool: computed from the whole attached table (CSV, TSV or a JSON
// array of records), never estimated by the model.

export interface Column {
  name: string;
  values: string[];
}

function splitCsvLine(line: string, delim: string): string[] {
  const out: string[] = [];
  let cur = '';
  let q = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (q) {
      if (c === '"' && line[i + 1] === '"') {
        cur += '"';
        i++;
      } else if (c === '"') q = false;
      else cur += c;
    } else if (c === '"') q = true;
    else if (c === delim) {
      out.push(cur);
      cur = '';
    } else cur += c;
  }
  out.push(cur);
  return out;
}

/** Records of a delimited text, honouring quoted fields (including quoted newlines). */
export function parseDelimited(text: string): Column[] {
  const t = text.replace(/^\uFEFF/, '');
  const first = t.split(/\r?\n/, 1)[0] ?? '';
  const delim = (first.match(/\t/g)?.length ?? 0) > (first.match(/,/g)?.length ?? 0) ? '\t' : (first.match(/;/g)?.length ?? 0) > (first.match(/,/g)?.length ?? 0) ? ';' : ',';
  const rows: string[][] = [];
  let buf = '';
  let q = false;
  for (const line of t.split(/\r?\n/)) {
    buf = buf ? `${buf}\n${line}` : line;
    q = ((buf.match(/"/g)?.length ?? 0) % 2) === 1;
    if (q) continue;
    if (buf.trim()) rows.push(splitCsvLine(buf, delim));
    buf = '';
  }
  if (!rows.length) return [];
  const head = rows[0].map((h, i) => h.trim() || `column${i + 1}`);
  return head.map((name, i) => ({ name, values: rows.slice(1).map((r) => r[i] ?? '') }));
}

export function columnsOf(kind: string, text: string): Column[] {
  if (kind === 'json') {
    const v = JSON.parse(text) as unknown;
    const arr = Array.isArray(v) ? v : v && typeof v === 'object' ? Object.values(v as object).find((x) => Array.isArray(x) && x.every((i) => i && typeof i === 'object')) : null;
    if (!Array.isArray(arr) || !arr.length || !arr.every((r) => r && typeof r === 'object' && !Array.isArray(r))) throw new Error('the JSON is not a non-empty list of records (an array of objects)');
    const names: string[] = [];
    for (const r of arr) for (const k of Object.keys(r)) if (!names.includes(k) && names.length < 200) names.push(k);
    return names.map((n) => ({ name: n, values: arr.map((r) => (r[n] === null || r[n] === undefined ? '' : typeof r[n] === 'string' ? r[n] : JSON.stringify(r[n]))) }));
  }
  if (kind === 'csv') return parseDelimited(text);
  throw new Error(`data_summary reads CSV, TSV and JSON files, not '${kind}'`);
}

const asNumber = (s: string): number | null => {
  const t = s.trim().replace(/^[$€£]\s?/, '').replace(/,(?=\d{3}(\D|$))/g, '').replace(/%$/, '');
  return t !== '' && /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i.test(t) ? Number(t) : null;
};

function fmt(v: number): string {
  return Number.isInteger(v) ? String(v) : String(parseFloat(v.toPrecision(12)));
}

function describe(col: Column, detail: boolean): string {
  const filled = col.values.filter((v) => v.trim() !== '');
  const missing = col.values.length - filled.length;
  const nums = filled.map(asNumber).filter((n): n is number => n !== null);
  if (filled.length && nums.length >= filled.length * 0.9) {
    const sorted = [...nums].sort((a, b) => a - b);
    const sum = nums.reduce((a, b) => a + b, 0);
    const mean = sum / nums.length;
    const median = sorted.length % 2 ? sorted[(sorted.length - 1) / 2] : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2;
    const sd = nums.length > 1 ? Math.sqrt(nums.reduce((a, b) => a + (b - mean) ** 2, 0) / (nums.length - 1)) : 0;
    let s = `- ${col.name} (number): count ${nums.length}, sum ${fmt(sum)}, mean ${fmt(mean)}, median ${fmt(median)}, min ${fmt(sorted[0])}, max ${fmt(sorted[sorted.length - 1])}, stdev ${fmt(sd)}${missing ? `, missing ${missing}` : ''}`;
    if (nums.length < filled.length) s += `, ${filled.length - nums.length} non-numeric value(s) ignored`;
    return s;
  }
  const freq = new Map<string, number>();
  for (const v of filled) freq.set(v, (freq.get(v) ?? 0) + 1);
  const top = [...freq.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, detail ? 15 : 5);
  return `- ${col.name} (text): ${freq.size} distinct, most frequent ${top.map(([v, n]) => `"${v.slice(0, 40)}" ×${n}`).join(', ')}${missing ? `, missing ${missing}` : ''}`;
}

export function summarizeTable(name: string, kind: string, text: string, column?: string): string {
  const cols = columnsOf(kind, text);
  if (!cols.length) throw new Error(`${name} has no rows`);
  const rows = cols[0].values.length;
  if (column) {
    const c = cols.find((x) => x.name.toLowerCase() === column.trim().toLowerCase());
    if (!c) throw new Error(`no column "${column}" in ${name}. Columns: ${cols.map((x) => x.name).join(', ')}`);
    return `TABLE ${name}: ${rows} rows\n${describe(c, true)}`;
  }
  return `TABLE ${name}: ${rows} rows × ${cols.length} columns (computed from the whole file)\n${cols.slice(0, 60).map((c) => describe(c, false)).join('\n')}`;
}

// ------------------------------------------------------------------------------------------ data_query

export interface QueryArgs {
  where?: { column: string; op?: string; value?: unknown }[];
  columns?: string[];
  group_by?: string;
  metrics?: { column?: string; fn?: string }[];
  sort_by?: string;
  desc?: boolean;
  limit?: number;
}

const FNS = ['count', 'sum', 'mean', 'min', 'max', 'median'] as const;
const colOf = (cols: Column[], name: string, file: string) => {
  const c = cols.find((x) => x.name.toLowerCase() === name.trim().toLowerCase());
  if (!c) throw new Error(`no column "${name}" in ${file}. Columns: ${cols.map((x) => x.name).join(', ')}`);
  return c;
};
/** Numbers compare as numbers, everything else as case-insensitive text. */
function cmp(a: string, b: string): number {
  const x = asNumber(a);
  const y = asNumber(b);
  if (x !== null && y !== null) return x === y ? 0 : x < y ? -1 : 1;
  const l = a.trim().toLowerCase();
  const r = b.trim().toLowerCase();
  return l === r ? 0 : l < r ? -1 : 1;
}
const OPS: Record<string, (a: string, b: string) => boolean> = {
  '=': (a, b) => cmp(a, b) === 0,
  '==': (a, b) => cmp(a, b) === 0,
  '!=': (a, b) => cmp(a, b) !== 0,
  '>': (a, b) => cmp(a, b) > 0,
  '>=': (a, b) => cmp(a, b) >= 0,
  '<': (a, b) => cmp(a, b) < 0,
  '<=': (a, b) => cmp(a, b) <= 0,
  contains: (a, b) => a.toLowerCase().includes(b.toLowerCase()),
};
function aggregate(fn: string, values: string[]): { n: number; v: number | null } {
  if (fn === 'count') {
    const n = values.filter((v) => v.trim() !== '').length;
    return { n, v: n };
  }
  const nums = values.map(asNumber).filter((n): n is number => n !== null);
  if (!nums.length) return { n: 0, v: null };
  const sorted = [...nums].sort((a, b) => a - b);
  const sum = nums.reduce((a, b) => a + b, 0);
  const mid = sorted.length % 2 ? sorted[(sorted.length - 1) / 2] : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2;
  const v = fn === 'sum' ? sum : fn === 'mean' ? sum / nums.length : fn === 'min' ? sorted[0] : fn === 'max' ? sorted[sorted.length - 1] : mid;
  return { n: nums.length, v };
}
const cell = (s: string) => s.replace(/\|/g, '\\|').replace(/\s+/g, ' ').trim();

/** Filter, group, aggregate, sort and limit an attached table: every figure is computed here from the whole file. */
export function queryTable(name: string, kind: string, text: string, q: QueryArgs): string {
  const cols = columnsOf(kind, text);
  if (!cols.length) throw new Error(`${name} has no rows`);
  const total = cols[0].values.length;
  let idx = Array.from({ length: total }, (_, i) => i);
  const notes: string[] = [];
  for (const w of q.where ?? []) {
    const c = colOf(cols, String(w.column ?? ''), name);
    const op = String(w.op ?? '=').trim().toLowerCase();
    const f = OPS[op];
    if (!f) throw new Error(`unknown filter operator "${op}". Use one of: ${Object.keys(OPS).filter((k) => k !== '==').join(' ')}`);
    const val = w.value === undefined || w.value === null ? '' : String(w.value);
    idx = idx.filter((i) => f(c.values[i], val));
    notes.push(`${c.name} ${op} ${val}`);
  }
  const limit = Math.min(Math.max(Math.trunc(Number(q.limit ?? 20)) || 20, 1), 200);
  const header = `TABLE ${name}: ${total} rows${notes.length ? `; filter ${notes.join(' AND ')} → ${idx.length} rows` : ''} (computed from the whole file)`;

  const metrics = (q.metrics ?? []).map((m) => {
    const fn = String(m.fn ?? 'count').toLowerCase();
    if (!(FNS as readonly string[]).includes(fn)) throw new Error(`unknown metric "${fn}". Use one of: ${FNS.join(', ')}`);
    if (fn !== 'count' && !m.column) throw new Error(`metric "${fn}" needs a "column"`);
    return { fn, col: m.column ? colOf(cols, m.column, name) : null };
  });
  const label = (m: { fn: string; col: Column | null }) => (m.col ? `${m.fn}(${m.col.name})` : 'count');

  if (q.group_by) {
    const g = colOf(cols, q.group_by, name);
    const ms = metrics.length ? metrics : [{ fn: 'count', col: null as Column | null }];
    const groups = new Map<string, number[]>();
    for (const i of idx) {
      const k = g.values[i].trim() === '' ? '(empty)' : g.values[i].trim();
      groups.set(k, [...(groups.get(k) ?? []), i]);
    }
    const rows = [...groups.entries()].map(([k, is]) => ({ k, n: is.length, vals: ms.map((m) => aggregate(m.fn, m.col ? is.map((i) => m.col!.values[i]) : is.map(() => 'x')).v) }));
    const sortKey = (q.sort_by ?? label(ms[0])).toLowerCase();
    const si = ms.findIndex((m) => label(m).toLowerCase() === sortKey);
    const byGroup = sortKey === g.name.toLowerCase();
    const dir = q.desc === false ? 1 : -1; // groups are listed biggest first unless desc is false
    rows.sort((a, b) => {
      const d = byGroup ? a.k.localeCompare(b.k) : (a.vals[si < 0 ? 0 : si] ?? -Infinity) - (b.vals[si < 0 ? 0 : si] ?? -Infinity);
      return dir * d || a.k.localeCompare(b.k);
    });
    const shown = rows.slice(0, limit);
    return `${header}\nGrouped by ${g.name}: ${groups.size} groups${groups.size > limit ? ` (showing ${limit})` : ''}\n| ${g.name} | rows | ${ms.map(label).join(' | ')} |\n|---|---|${ms.map(() => '---').join('|')}|\n${shown.map((r) => `| ${cell(r.k)} | ${r.n} | ${r.vals.map((v) => (v === null ? '—' : fmt(v))).join(' | ')} |`).join('\n')}`;
  }

  if (metrics.length) {
    const out = metrics.map((m) => {
      const a = aggregate(m.fn, m.col ? idx.map((i) => m.col!.values[i]) : idx.map(() => 'x'));
      return `- ${label(m)} = ${a.v === null ? 'no numeric values' : fmt(a.v)}${m.col && m.fn !== 'count' ? ` (over ${a.n} numeric values)` : ''}`;
    });
    return `${header}\n${out.join('\n')}`;
  }

  const pick = q.columns?.length ? q.columns.map((c) => colOf(cols, c, name)) : cols.slice(0, 12);
  if (q.sort_by) {
    const s = colOf(cols, q.sort_by, name);
    idx.sort((a, b) => (q.desc ? -1 : 1) * cmp(s.values[a], s.values[b]) || a - b);
  }
  const shown = idx.slice(0, limit);
  return `${header}\nShowing ${shown.length} of ${idx.length} rows${q.sort_by ? ` sorted by ${q.sort_by}${q.desc ? ' (descending)' : ''}` : ''}\n| ${pick.map((c) => c.name).join(' | ')} |\n|${pick.map(() => '---').join('|')}|\n${shown.map((i) => `| ${pick.map((c) => cell(c.values[i])).join(' | ')} |`).join('\n')}`;
}
