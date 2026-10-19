// Reading the files a user attaches in Genius Web, in the browser. Same rules as the desktop core
// (crates/genius-core/src/attachments.rs): the BYTES decide what an image is, unsupported formats are
// refused instead of "analysed", scanned PDFs are not pretended to be text, and everything extracted is
// untrusted data. Nothing is executed.

import { unzipSync, type Unzipped } from 'fflate';
import type { AttachmentRec } from '../store';

export const MAX_FILE_BYTES = 25 * 1024 * 1024;
export const MAX_IMAGE_BYTES = 5 * 1024 * 1024;
const MAX_TEXT_CHARS = 2_000_000;
const ZIP_MAX_ENTRY = 30 * 1024 * 1024;

const CODE_EXT = ['ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'css', 'scss', 'html', 'htm', 'vue', 'svelte', 'py', 'rs', 'go', 'java', 'kt', 'swift', 'c', 'h', 'cpp', 'cs', 'rb', 'php', 'sql', 'sh', 'ps1', 'yml', 'yaml', 'toml', 'xml', 'svg', 'ini', 'graphql'];
export const SUPPORTED_EXTENSIONS = ['txt', 'log', 'md', 'markdown', 'csv', 'tsv', 'json', 'pdf', 'docx', 'xlsx', 'png', 'jpg', 'jpeg', 'gif', 'webp', ...CODE_EXT];

type Parsed = Pick<AttachmentRec, 'name' | 'kind' | 'mime' | 'size' | 'status' | 'error' | 'text' | 'imageDataUrl' | 'meta'>;

export function classify(name: string): { kind: string; mime: string } | null {
  const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
  const table: Record<string, [string, string]> = {
    txt: ['text', 'text/plain'], log: ['text', 'text/plain'], md: ['markdown', 'text/markdown'], markdown: ['markdown', 'text/markdown'],
    csv: ['csv', 'text/csv'], tsv: ['csv', 'text/tab-separated-values'], json: ['json', 'application/json'], pdf: ['pdf', 'application/pdf'],
    docx: ['docx', 'application/vnd.openxmlformats-officedocument.wordprocessingml.document'], xlsx: ['csv', 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet'],
    png: ['image', 'image/png'], jpg: ['image', 'image/jpeg'], jpeg: ['image', 'image/jpeg'], gif: ['image', 'image/gif'], webp: ['image', 'image/webp'],
  };
  if (table[ext]) return { kind: table[ext][0], mime: table[ext][1] };
  return CODE_EXT.includes(ext) ? { kind: 'code', mime: 'text/plain' } : null;
}

export function sniffImage(b: Uint8Array): string | null {
  const eq = (s: number[], at = 0) => s.every((v, i) => b[at + i] === v);
  if (eq([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return 'image/png';
  if (eq([0xff, 0xd8, 0xff])) return 'image/jpeg';
  if (eq([0x47, 0x49, 0x46, 0x38])) return 'image/gif';
  if (eq([0x52, 0x49, 0x46, 0x46]) && eq([0x57, 0x45, 0x42, 0x50], 8)) return 'image/webp';
  return null;
}

const unescapeXml = (s: string) => s.replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');
const utf8 = (b: Uint8Array) => new TextDecoder('utf-8', { fatal: false }).decode(b).replace(/^\uFEFF/, '');

function unzipSafe(bytes: Uint8Array, want: (name: string) => boolean): Unzipped {
  return unzipSync(bytes, { filter: (f) => {
    if (!want(f.name)) return false;
    if (f.originalSize > ZIP_MAX_ENTRY) throw new Error('an entry of the archive is too large');
    return true;
  } });
}

export function docxText(bytes: Uint8Array): string {
  const z = unzipSafe(bytes, (n) => n === 'word/document.xml');
  const xml = z['word/document.xml'];
  if (!xml) throw new Error('DOCX has no word/document.xml');
  let out = '';
  for (const m of utf8(xml).matchAll(/<w:t(?:\s[^>]*)?>([\s\S]*?)<\/w:t>|<w:tab\/>|<\/w:p>|<w:br\/>/g)) out += m[1] !== undefined ? unescapeXml(m[1]) : m[0] === '<w:tab/>' ? '\t' : '\n';
  return out.trim();
}

const csvCell = (v: string) => (/[,"\n\r]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v);
const colIndex = (ref: string) => {
  let n = 0;
  for (const ch of ref.match(/^[A-Za-z]+/)?.[0] ?? '') n = n * 26 + (ch.toUpperCase().charCodeAt(0) - 64);
  return Math.max(0, n - 1);
};

/** Sheets of an XLSX as CSV text (cached values; dates stay serial numbers). */
export function xlsxSheets(bytes: Uint8Array): { name: string; csv: string }[] {
  const z = unzipSafe(bytes, (n) => n === 'xl/workbook.xml' || n === 'xl/_rels/workbook.xml.rels' || n === 'xl/sharedStrings.xml' || /^xl\/worksheets\/[^/]+\.xml$/.test(n));
  if (!z['xl/workbook.xml']) throw new Error('the XLSX has no xl/workbook.xml (is it an .xls file or encrypted?)');
  const attrs = (s: string) => Object.fromEntries([...s.matchAll(/([\w:]+)="([^"]*)"/g)].map((m) => [m[1], unescapeXml(m[2])]));
  const rels = new Map([...utf8(z['xl/_rels/workbook.xml.rels'] ?? new Uint8Array()).matchAll(/<Relationship\b([^>]*)\/?>/g)].map((m) => { const a = attrs(m[1]); return [a.Id, a.Target] as const; }));
  const text = (frag: string) => [...frag.matchAll(/<t(?:\s[^>]*)?>([\s\S]*?)<\/t>/g)].map((m) => unescapeXml(m[1])).join('');
  const shared = [...utf8(z['xl/sharedStrings.xml'] ?? new Uint8Array()).matchAll(/<si\b[^>]*>([\s\S]*?)<\/si>/g)].map((m) => text(m[1]));
  const out: { name: string; csv: string }[] = [];
  [...utf8(z['xl/workbook.xml']).matchAll(/<sheet\b([^>]*)\/?>/g)].forEach((m, i) => {
    const a = attrs(m[1]);
    const target = rels.get(a['r:id']) ?? `worksheets/sheet${i + 1}.xml`;
    const xml = z[target.startsWith('/') ? target.slice(1) : `xl/${target}`];
    if (!xml) return;
    const lines: string[] = [];
    for (const row of utf8(xml).matchAll(/<row\b[^>]*>([\s\S]*?)<\/row>/g)) {
      const cells: string[] = [];
      for (const c of row[1].matchAll(/<c\b([^>]*?)(?:\/>|>([\s\S]*?)<\/c>)/g)) {
        const ca = attrs(c[1]);
        const inner = c[2] ?? '';
        const raw = /<v>([\s\S]*?)<\/v>/.exec(inner)?.[1];
        const v = raw === undefined ? '' : unescapeXml(raw);
        const value = ca.t === 's' ? (shared[parseInt(v, 10)] ?? '') : ca.t === 'inlineStr' ? text(inner) : ca.t === 'b' ? (v.trim() === '1' ? 'TRUE' : 'FALSE') : v;
        const col = Math.min(ca.r ? colIndex(ca.r) : cells.length, 255);
        while (cells.length <= col) cells.push('');
        cells[col] = value;
      }
      while (cells.length && cells[cells.length - 1] === '') cells.pop();
      lines.push(cells.map(csvCell).join(','));
    }
    while (lines.length && lines[lines.length - 1] === '') lines.pop();
    out.push({ name: a.name ?? `Sheet${i + 1}`, csv: lines.join('\n') });
  });
  if (!out.length) throw new Error('the XLSX contains no readable worksheet');
  return out;
}

async function pdfText(bytes: Uint8Array): Promise<string> {
  const pdfjs = await import('pdfjs-dist');
  const worker = (await import('pdfjs-dist/build/pdf.worker.min.mjs?url')).default;
  pdfjs.GlobalWorkerOptions.workerSrc = worker;
  const doc = await pdfjs.getDocument({ data: bytes.slice(), isEvalSupported: false, disableFontFace: true }).promise;
  let out = '';
  for (let p = 1; p <= Math.min(doc.numPages, 300); p++) {
    const page = await doc.getPage(p);
    const content = await page.getTextContent();
    out += `${content.items.map((i) => ('str' in i ? i.str : '')).join(' ')}\n\n`;
    if (out.length > MAX_TEXT_CHARS) break;
  }
  return out.trim();
}

const toDataUrl = (mime: string, b: Uint8Array) => {
  let bin = '';
  for (let i = 0; i < b.length; i += 0x8000) bin += String.fromCharCode(...b.subarray(i, i + 0x8000));
  return `data:${mime};base64,${btoa(bin)}`;
};

/** Parse a browser File. Never throws: a file that cannot be used comes back with status "failed" and the reason. */
export async function parseFile(file: { name: string; size: number; arrayBuffer(): Promise<ArrayBuffer> }): Promise<Parsed> {
  const base = { name: file.name, size: file.size, text: '', imageDataUrl: null as string | null, meta: {} as Record<string, unknown> };
  const failed = (error: string, kind = 'unknown', mime = 'application/octet-stream'): Parsed => ({ ...base, kind, mime, status: 'failed', error });
  const c = classify(file.name);
  if (!c) return failed(`unsupported file type: ${file.name}. Supported: ${SUPPORTED_EXTENSIONS.join(', ')}`);
  if (file.size > MAX_FILE_BYTES) return failed(`files are limited to ${MAX_FILE_BYTES / 1_048_576} MB`, c.kind, c.mime);
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const meta: Record<string, unknown> = {};
    let text = '';
    switch (c.kind) {
      case 'image': {
        if (bytes.length > MAX_IMAGE_BYTES) return failed(`images are limited to ${MAX_IMAGE_BYTES / 1_048_576} MB`, c.kind, c.mime);
        const real = sniffImage(bytes);
        if (!real) return failed(`${file.name} is not a PNG, JPEG, GIF or WebP image (the file contents do not match its extension), so it cannot be analysed`, c.kind, c.mime);
        if (real !== c.mime) meta.mimeCorrected = { from: c.mime, to: real };
        return { ...base, kind: 'image', mime: real, status: 'ready', error: null, imageDataUrl: toDataUrl(real, bytes), meta };
      }
      case 'json':
        text = utf8(bytes);
        JSON.parse(text);
        break;
      case 'pdf':
        text = await pdfText(bytes);
        if (!text.trim()) return failed('the PDF has no extractable text layer, so it is probably a scan and Genius cannot read it as text. Attach its pages as images (PNG or JPEG) instead: a vision-capable model can read them', c.kind, c.mime);
        break;
      case 'docx':
        text = docxText(bytes);
        break;
      case 'csv':
        if (file.name.toLowerCase().endsWith('.xlsx')) {
          const sheets = xlsxSheets(bytes);
          const first = sheets.find((s) => s.csv.trim()) ?? sheets[0];
          text = first.csv;
          meta.format = 'xlsx';
          meta.sheets = sheets.map((s) => s.name);
          meta.sheetUsed = first.name;
          const dates = 'Dates are shown as the serial numbers stored in the file.';
          meta.note = sheets.length > 1 ? `Only the sheet "${first.name}" is read as a table; the other ${sheets.length - 1} sheet(s) are not included: ${sheets.filter((s) => s !== first).map((s) => s.name).join(', ')}. ${dates}` : dates;
        } else text = utf8(bytes);
        meta.rows = text.split('\n').length;
        break;
      default:
        text = utf8(bytes);
    }
    if (text.length > MAX_TEXT_CHARS) {
      text = text.slice(0, MAX_TEXT_CHARS);
      meta.truncated = true;
    }
    return { ...base, kind: c.kind, mime: c.mime, status: 'ready', error: null, text, meta };
  } catch (e) {
    return failed(e instanceof Error ? e.message : String(e), c.kind, c.mime);
  }
}
