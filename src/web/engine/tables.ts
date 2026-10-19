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
