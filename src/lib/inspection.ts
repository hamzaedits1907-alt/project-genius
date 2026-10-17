// Visual verification of the live preview: answers the backend's `genius://inspect-request`.
//
// The page inspector injected into the preview (crates/genius-core/src/inspect_page.js) computes
// a report inside the real page when it receives `genius:inspect`. This module asks it at the
// current width, then at each requested width by temporarily resizing the preview's device
// frame (restored afterwards), and sends the combined report to the backend.
import { api, errorMessage } from './api';
import type { InspectionIssue, InspectionReport, InspectRequest, PageReport, ViewportReport } from './types';

/** A preview iframe that can be inspected, and the element whose width sets the page width. */
export interface PreviewTarget {
  frame: HTMLIFrameElement;
  container: HTMLElement;
  /** The preview's origin: messages are only sent to and accepted from it. */
  origin: string;
  projectId: string | null;
}

type TargetGetter = () => PreviewTarget | null;
const targets = new Set<TargetGetter>();

/** Called by each mounted preview panel; returns the unregister function. */
export function registerPreviewTarget(get: TargetGetter): () => void {
  targets.add(get);
  return () => {
    targets.delete(get);
  };
}

/** The element takes up space on screen (not display:none, e.g. a hidden tab). */
export const isShown = (el: Element) => el.isConnected && el.getClientRects().length > 0;

export const labelFor = (w: number): ViewportReport['label'] => (w <= 480 ? 'mobile' : w < 1024 ? 'tablet' : 'desktop');

const str = (v: unknown, n: number) => (typeof v === 'string' ? v.slice(0, n) : '');
const int = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? Math.max(0, Math.round(v)) : 0);
const obj = (v: unknown): Record<string, unknown> => (v && typeof v === 'object' ? (v as Record<string, unknown>) : {});
const issues = (v: unknown): InspectionIssue[] =>
  Array.isArray(v)
    ? v.slice(0, 60).flatMap((i) => {
        const o = obj(i);
        return typeof o.kind === 'string' && typeof o.message === 'string' ? [{ kind: o.kind.slice(0, 30), message: o.message.slice(0, 300), selector: typeof o.selector === 'string' ? o.selector.slice(0, 200) : null }] : [];
      })
    : [];

/** Accept only a well-formed report from the preview page (it is untrusted code). */
export function parsePageReport(data: unknown): PageReport | null {
  if (!data || typeof data !== 'object') return null;
  const d = obj(data);
  const vp = obj(d.viewport);
  const r = obj(d.rendered);
  const c = obj(d.console);
  return {
    url: str(d.url, 300),
    title: str(d.title, 200),
    lang: typeof d.lang === 'string' && d.lang ? d.lang.slice(0, 20) : null,
    viewport: { width: int(vp.width), height: int(vp.height) },
    scrollWidth: int(d.scrollWidth),
    scrollHeight: int(d.scrollHeight),
    rendered: { root: str(r.root, 60) || 'body', visibleElements: int(r.visibleElements), textLength: int(r.textLength), blank: r.blank === true },
    headings: Array.isArray(d.headings) ? d.headings.slice(0, 40).flatMap((h) => (typeof obj(h).text === 'string' ? [{ level: Math.min(6, Math.max(1, int(obj(h).level))), text: str(obj(h).text, 120) }] : [])) : [],
    landmarks: Array.isArray(d.landmarks) ? d.landmarks.slice(0, 30).filter((l): l is string => typeof l === 'string').map((l) => l.slice(0, 80)) : [],
    text: str(d.text, 1000),
    layout: issues(d.layout),
    a11y: issues(d.a11y),
    console: { errors: int(c.errors), warnings: int(c.warnings) },
    truncated: d.truncated === true,
  };
}

let seq = 0;

/** Ask the page for one report. `wait` lets layout settle after a resize (ms). */
export function inspectOnce(t: PreviewTarget, wait = 0, timeoutMs = 5000): Promise<PageReport> {
  const win = t.frame.contentWindow;
  if (!win) return Promise.reject(new Error('The preview page is not loaded.'));
  const requestId = `i${Date.now().toString(36)}${(seq++).toString(36)}`;
  return new Promise<PageReport>((resolve, reject) => {
    const done = () => {
      clearTimeout(timer);
      window.removeEventListener('message', onMsg);
    };
    const onMsg = (e: MessageEvent) => {
      if (e.origin !== t.origin || e.source !== win) return;
      const d = obj(e.data);
      if (d.type !== 'genius:inspection' || d.requestId !== requestId) return;
      done();
      const report = parsePageReport(d.report);
      if (report) resolve(report);
      else reject(new Error(`the page could not inspect itself${typeof d.error === 'string' ? `: ${d.error.slice(0, 300)}` : ''}`));
    };
    const timer = setTimeout(() => {
      done();
      reject(new Error('The preview page did not answer. The page inspector is only installed in Vite projects started by Genius; reload the preview and try again.'));
    }, timeoutMs + wait);
    window.addEventListener('message', onMsg);
    win.postMessage({ type: 'genius:inspect', requestId, wait }, t.origin);
  });
}

/** Inspect at the current width, then at each of `widths`, restoring the frame afterwards. */
export async function inspectPreview(t: PreviewTarget, widths: number[], opts: { wait?: number; timeoutMs?: number } = {}): Promise<InspectionReport> {
  const wait = opts.wait ?? 350;
  const current = await inspectOnce(t, 0, opts.timeoutMs);
  const viewports: ViewportReport[] = [{ ...current, label: 'current', requestedWidth: null }];
  const notes: string[] = [];
  const style = t.container.style;
  const saved = { width: style.width, maxWidth: style.maxWidth, transition: style.transition };
  try {
    for (const w of widths) {
      if (Math.abs(current.viewport.width - w) <= 2) {
        notes.push(`${w}px is the current width (see "current")`);
        continue;
      }
      // No transition: the page must be at its final width when it measures.
      style.transition = 'none';
      style.maxWidth = 'none';
      style.width = `${w}px`;
      try {
        const r = await inspectOnce(t, wait, opts.timeoutMs);
        viewports.push({ ...r, label: labelFor(w), requestedWidth: w });
        if (Math.abs(r.viewport.width - w) > 2) notes.push(`asked for ${w}px but the page measured ${r.viewport.width}px`);
      } catch (e) {
        notes.push(`${w}px: ${errorMessage(e)}`);
      }
    }
  } finally {
    Object.assign(style, saved);
  }
  const last = viewports[viewports.length - 1];
  return { url: current.url, title: current.title, lang: current.lang, console: last.console, viewports, notes };
}

let chain: Promise<unknown> = Promise.resolve();

/** Answer one backend request (serialized: two passes must not resize the frame at once). */
export function handleInspectRequest(req: InspectRequest, report = api.reportPreviewInspection): Promise<void> {
  const run = async () => {
    if (!req || typeof req.requestId !== 'string') return;
    const candidates = [...targets].map((g) => g()).filter((t): t is PreviewTarget => !!t && t.projectId === req.projectId);
    const target = candidates.find((t) => isShown(t.frame));
    const widths = (Array.isArray(req.widths) ? req.widths : []).filter((w) => typeof w === 'number' && w > 0).slice(0, 4);
    try {
      if (!target) {
        throw new Error(
          candidates.length
            ? 'The preview is hidden in the Genius window. Ask the user to open the Preview tab so Genius can look at the app, then try again.'
            : 'The preview of this project is not shown in the Genius window (it may still be starting). Ask the user to open the project and the Preview tab.',
        );
      }
      const result = await inspectPreview(target, widths);
      await report(req.requestId, result, null);
    } catch (e) {
      await report(req.requestId, null, errorMessage(e)).catch(() => undefined);
    }
  };
  const next = chain.then(run, run);
  chain = next.catch(() => undefined);
  return next.catch(() => undefined);
}
