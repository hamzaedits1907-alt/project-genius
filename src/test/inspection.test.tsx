// Visual verification of the live preview: the shipped page inspector (inspect_page.js, the same
// text inspector.rs injects) evaluated for real in jsdom, and the window-side responder that
// resizes the preview, collects reports and answers the backend.
//
// jsdom has no layout engine: boxes are stubbed per element (`data-box`) as a function of the
// viewport width, and `postMessage` is bridged by hand because jsdom sets neither `source` nor
// `origin` on message events. Everything else (DOM walk, computed styles, colour maths,
// accessible names, the request/response protocol) is the real code.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';
import pageSrc from '../../crates/genius-core/src/inspect_page.js?raw';
import { calls, clearMocks, installIpc, project, settings } from './ipc';
import { handleInspectRequest, inspectPreview, labelFor, parsePageReport, registerPreviewTarget, type PreviewTarget } from '../lib/inspection';
import type { InspectionReport, PageReport } from '../lib/types';
import PreviewPanel from '../views/workspace/PreviewPanel';
import { useApp } from '../store/app';
import { useWorkspace } from '../store/workspace';

type Rgba = { r: number; g: number; b: number; a: number };
type Rect = { left: number; top: number; right: number; bottom: number; width: number; height: number };
interface Lib {
  parseColor(v: string): Rgba | null;
  contrastRatio(a: Rgba, b: Rgba): number;
  blend(top: Rgba, bottom: Rgba): Rgba;
  effectiveBackground(el: Element, styleOf: (el: Element) => Partial<CSSStyleDeclaration> | null): Rgba | null;
  isLargeText(px: number, weight: string): boolean;
  checkContrast(fg: Rgba, bg: Rgba, px: number, weight: string): { ratio: number; required: number; ok: boolean };
  horizontalOverflow(rect: Rect, vw: number): number;
  intersectionArea(a: Rect, b: Rect): number;
  headingSkips(levels: number[]): { from: number; to: number }[];
  accessibleName(el: Element): string;
  inspectPage(win: Window, opts?: { consoleCounts?: { errors: number; warnings: number } }): PageReport;
}
type Factory = (win: Window, install: boolean) => Lib;
const factory = new Function(`return (${pageSrc})`)() as Factory;
const lib = factory(window, false);

const rgb = (r: number, g: number, b: number, a = 1): Rgba => ({ r, g, b, a });
const rect = (left: number, top: number, width: number, height: number): Rect => ({ left, top, width, height, right: left + width, bottom: top + height });
const close = (c: Rgba | null, r: number, g: number, b: number, a = 1) => {
  expect(c).not.toBeNull();
  expect(Math.abs(c!.r - r)).toBeLessThan(1.5);
  expect(Math.abs(c!.g - g)).toBeLessThan(1.5);
  expect(Math.abs(c!.b - b)).toBeLessThan(1.5);
  expect(c!.a).toBeCloseTo(a, 3);
};

describe('page inspector: colour and contrast maths', () => {
  it('parses computed colour syntaxes', () => {
    close(lib.parseColor('rgb(255, 0, 0)'), 255, 0, 0);
    close(lib.parseColor('rgba(0, 0, 0, 0.5)'), 0, 0, 0, 0.5);
    close(lib.parseColor('rgb(10 20 30 / 25%)'), 10, 20, 30, 0.25);
    close(lib.parseColor('#abc'), 170, 187, 204);
    close(lib.parseColor('#11223380'), 17, 34, 51, 128 / 255);
    close(lib.parseColor('transparent'), 0, 0, 0, 0);
    close(lib.parseColor('color(srgb 1 0.5 0)'), 255, 127.5, 0);
    // Tailwind v4 colours are oklch(); browsers return them as authored.
    close(lib.parseColor('oklch(1 0 0)'), 255, 255, 255);
    close(lib.parseColor('oklch(0 0 0)'), 0, 0, 0);
    close(lib.parseColor('oklch(62.7955% 0.257683 29.2339)'), 255, 0, 0);
    close(lib.parseColor('oklab(0.519975 -0.140445 0.107676 / 0.5)'), 0, 128, 0, 0.5);
    expect(lib.parseColor('red')).toBeNull();
    expect(lib.parseColor('color(display-p3 1 0 0)')).toBeNull();
    expect(lib.parseColor('')).toBeNull();
  });

  it('computes WCAG contrast with translucency and large-text thresholds', () => {
    expect(lib.contrastRatio(rgb(0, 0, 0), rgb(255, 255, 255))).toBeCloseTo(21, 5);
    expect(lib.contrastRatio(rgb(255, 255, 255), rgb(255, 255, 255))).toBeCloseTo(1, 5);
    // #777 on white is the classic just-failing grey (4.48:1)
    expect(lib.contrastRatio(rgb(119, 119, 119), rgb(255, 255, 255))).toBeCloseTo(4.48, 2);
    expect(lib.checkContrast(rgb(119, 119, 119), rgb(255, 255, 255), 16, '400')).toEqual({ ratio: 4.48, required: 4.5, ok: false });
    expect(lib.checkContrast(rgb(119, 119, 119), rgb(255, 255, 255), 24, '400').ok).toBe(true);
    expect(lib.isLargeText(18.67, '700')).toBe(true);
    expect(lib.isLargeText(18.67, '400')).toBe(false);
    expect(lib.isLargeText(19, 'bold')).toBe(true);
    // black at 30% on white is light grey: fails
    const faint = lib.checkContrast(rgb(0, 0, 0, 0.3), rgb(255, 255, 255), 14, '400');
    expect(faint.ok).toBe(false);
    expect(faint.ratio).toBeCloseTo(lib.contrastRatio(rgb(178.5, 178.5, 178.5), rgb(255, 255, 255)), 2);
    close(lib.blend(rgb(0, 0, 0, 0.5), rgb(255, 255, 255)), 127.5, 127.5, 127.5);
  });

  it('composites ancestor backgrounds and gives up on images/gradients', () => {
    document.body.innerHTML = '<div id="a"><div id="b"><span id="c">x</span></div></div>';
    const styles: Record<string, Partial<CSSStyleDeclaration>> = {
      a: { backgroundColor: 'rgb(0, 0, 0)', backgroundImage: 'none' },
      b: { backgroundColor: 'rgba(255, 255, 255, 0.5)', backgroundImage: 'none' },
      c: { backgroundColor: 'rgba(0, 0, 0, 0)', backgroundImage: 'none' },
    };
    const styleOf = (el: Element) => styles[el.id] ?? { backgroundColor: 'rgba(0, 0, 0, 0)', backgroundImage: 'none' };
    const c = document.getElementById('c')!;
    close(lib.effectiveBackground(c, styleOf), 127.5, 127.5, 127.5);
    styles.a.backgroundColor = 'rgba(0, 0, 0, 0)';
    close(lib.effectiveBackground(c, styleOf), 255, 255, 255, 1);
    styles.a.backgroundImage = 'linear-gradient(red, blue)';
    expect(lib.effectiveBackground(c, styleOf)).toBeNull();
  });
});

describe('page inspector: geometry and structure helpers', () => {
  it('measures horizontal overflow, overlaps and heading skips', () => {
    expect(lib.horizontalOverflow(rect(0, 0, 375, 10), 375)).toBe(0);
    expect(lib.horizontalOverflow(rect(10, 0, 600, 10), 375)).toBe(235);
    expect(lib.horizontalOverflow(rect(-40, 0, 100, 10), 375)).toBe(40);
    expect(lib.intersectionArea(rect(0, 0, 100, 50), rect(50, 25, 100, 50))).toBe(50 * 25);
    expect(lib.intersectionArea(rect(0, 0, 100, 50), rect(100, 0, 10, 10))).toBe(0);
    expect(lib.headingSkips([1, 2, 3, 2, 4, 1, 3])).toEqual([{ from: 2, to: 4 }, { from: 1, to: 3 }]);
    expect(lib.headingSkips([2, 3])).toEqual([]);
  });

  it('computes accessible names like assistive technology', () => {
    document.body.innerHTML = `
      <button id="b1"><svg><title>Search</title></svg></button>
      <button id="b2"><svg></svg></button>
      <button id="b3" aria-label="Close">×</button>
      <span id="lbl">Due date</span><input id="i1" aria-labelledby="lbl">
      <label>Name <input id="i2"></label>
      <label for="i3">Email</label><input id="i3" type="email">
      <input id="i4" placeholder="Search…">
      <input id="i5" type="submit">
      <a id="a1" href="/"><img src="x.png" alt="Home"></a>
      <a id="a2" href="/"><span aria-hidden="true">→</span></a>`;
    const name = (id: string) => lib.accessibleName(document.getElementById(id)!);
    expect(['b1', 'b2', 'b3', 'i1', 'i2', 'i3', 'i4', 'i5', 'a1', 'a2'].map(name)).toEqual(['Search', '', 'Close', 'Due date', 'Name', 'Email', '', 'Submit', 'Home', '']);
  });
});

// ---------------------------------------------------------------- full page in an iframe

const FIXTURE = `
  <header><nav aria-label="Main"><a href="/">Home</a> <a href="/settings"><svg></svg></a></nav></header>
  <main>
    <h1>Sales</h1>
    <h3>Skipped level</h3>
    <p style="color: rgb(170, 170, 170); background-color: rgb(255, 255, 255)">Faint text</p>
    <p style="color: rgb(20, 20, 20)">Readable text</p>
    <img id="logo" src="/logo.png">
    <img id="hero" src="/hero.png" alt="Hero" data-box="zero">
    <input type="email" placeholder="Email">
    <label for="n">Name</label><input id="n">
    <button><svg></svg></button>
    <button aria-label="Close">x</button>
    <div class="table" data-box="table">Wide table</div>
    <div style="overflow-x: auto"><div class="inner" data-box="table">Inside a scroller</div></div>
    <section style="display: none"><img src="/hidden.png"><button></button><h5>hidden</h5></section>
    <div id="bar1" style="position: fixed" data-box="bar1">Bar one</div>
    <div id="bar2" style="position: fixed" data-box="bar2">Bar two</div>
  </main>`;

type Win = Window & typeof globalThis;
const savedRect = new Map<Win, typeof Element.prototype.getBoundingClientRect>();

/** Fake layout for the iframe's document: boxes depend on the current viewport width. */
function stubLayout(win: Win, width: () => number) {
  Object.defineProperty(win, 'innerWidth', { configurable: true, get: width });
  Object.defineProperty(win, 'innerHeight', { configurable: true, value: 700 });
  const boxes: Record<string, (vw: number) => [number, number, number, number]> = {
    zero: () => [0, 0, 0, 0],
    table: () => [0, 300, 600, 200],
    bar1: (vw) => [0, 0, vw, 60],
    bar2: (vw) => [0, 20, vw, 60],
  };
  if (!savedRect.has(win)) savedRect.set(win, win.Element.prototype.getBoundingClientRect);
  win.Element.prototype.getBoundingClientRect = function (this: Element) {
    const vw = width();
    const key = this.getAttribute('data-box');
    const [l, t, w, h] = key && boxes[key] ? boxes[key](vw) : [0, 0, Math.min(vw, 300), 20];
    return new win.DOMRect(l, t, w, h);
  };
  const doc = win.document;
  Object.defineProperty(doc.documentElement, 'scrollWidth', { configurable: true, get: () => Math.max(width(), 600) });
  Object.defineProperty(doc.documentElement, 'scrollHeight', { configurable: true, value: 1800 });
  const img = (id: string, natural: number) => {
    const el = doc.getElementById(id) as HTMLImageElement | null;
    if (!el) return;
    Object.defineProperty(el, 'complete', { configurable: true, value: true });
    Object.defineProperty(el, 'naturalWidth', { configurable: true, value: natural });
    Object.defineProperty(el, 'naturalHeight', { configurable: true, value: natural / 2 });
    el.getClientRects = () => [new win.DOMRect(0, 0, 0, 0)] as unknown as DOMRectList;
  };
  img('logo', 0);
  img('hero', 800);
}

/** Put the fixture page into an iframe (jsdom leaves a frame with an unreachable src empty). */
function fill(frame: HTMLIFrameElement) {
  const doc = frame.contentDocument!;
  if (!doc.documentElement) {
    const html = doc.appendChild(doc.createElement('html'));
    html.appendChild(doc.createElement('head'));
    html.appendChild(doc.createElement('body'));
  }
  doc.body.innerHTML = FIXTURE;
}

function makeFrame(parent: HTMLElement = document.body): HTMLIFrameElement {
  const frame = document.createElement('iframe');
  parent.appendChild(frame);
  fill(frame);
  return frame;
}

/** jsdom's postMessage sets neither source nor origin: deliver messages the way a browser does.
 *  (`cw.parent` is jsdom's proxy of the test window, not the same object as `window`.) */
function bridge(frame: HTMLIFrameElement, origin: string) {
  const cw = frame.contentWindow as Win;
  const parent = cw.parent as Win;
  cw.postMessage = ((msg: unknown) => {
    setTimeout(() => cw.dispatchEvent(new cw.MessageEvent('message', { data: msg, source: parent, origin: window.location.origin })), 0);
  }) as Window['postMessage'];
  const real = parent.postMessage;
  parent.postMessage = ((msg: unknown) => {
    setTimeout(() => window.dispatchEvent(new MessageEvent('message', { data: JSON.parse(JSON.stringify(msg)), origin, source: cw })), 0);
  }) as Window['postMessage'];
  return () => {
    parent.postMessage = real;
  };
}

const kinds = (r: PageReport) => ({ layout: r.layout.map((i) => i.kind), a11y: r.a11y.map((i) => i.kind) });

describe('page inspector: report computed in the page', () => {
  afterEach(() => {
    for (const [w, fn] of savedRect) w.Element.prototype.getBoundingClientRect = fn;
    savedRect.clear();
    document.body.innerHTML = '';
  });

  it('reports rendering, outline, layout and accessibility problems', () => {
    const frame = makeFrame();
    const cw = frame.contentWindow as Win;
    let vw = 375;
    stubLayout(cw, () => vw);
    const r = factory(cw, false).inspectPage(cw, { consoleCounts: { errors: 2, warnings: 1 } });
    expect(r.rendered.root).toBe('body');
    expect(r.rendered.blank).toBe(false);
    expect(r.rendered.visibleElements).toBeGreaterThan(15);
    expect(r.viewport).toEqual({ width: 375, height: 700 });
    expect(r.headings).toEqual([{ level: 1, text: 'Sales' }, { level: 3, text: 'Skipped level' }]);
    expect(r.landmarks).toEqual(['header', '  nav "Main"', 'main']);
    expect(r.text).toMatch(/^Home Sales Skipped level Faint text Readable text/);
    expect(r.text).not.toContain('hidden');
    expect(r.console).toEqual({ errors: 2, warnings: 1 });
    const k = kinds(r);
    expect(k.layout.sort()).toEqual(['fixed-overlap', 'image-broken', 'image-zero-size', 'overflow-x', 'page-overflow-x']);
    expect(k.a11y.sort()).toEqual(['button-name', 'contrast', 'control-label', 'heading-order', 'html-lang', 'img-alt', 'link-name']);
    const over = r.layout.find((i) => i.kind === 'overflow-x')!;
    expect(over.selector).toBe('main > div.table');
    expect(over.message).toBe('sticks out 225px past the right edge of the 375px viewport (width 600px)');
    expect(r.layout.find((i) => i.kind === 'fixed-overlap')!.message).toContain('div#bar1 overlaps fixed div#bar2 (67% of the smaller one)');
    expect(r.layout.find((i) => i.kind === 'image-broken')!.message).toContain('/logo.png');
    expect(r.a11y.find((i) => i.kind === 'contrast')!.message).toBe('text "Faint text" has contrast 2.32:1 (#aaaaaa on #ffffff), needs 4.5:1');
    expect(r.a11y.find((i) => i.kind === 'control-label')!.message).toBe('email field has no label (only a placeholder: "Email")');
    expect(r.a11y.find((i) => i.kind === 'heading-order')!.message).toBe('heading level skips from h1 to h3');
    expect(r.a11y.filter((i) => i.kind === 'button-name')).toHaveLength(1);

    // Same page at desktop width: the table fits, the page no longer scrolls sideways.
    vw = 1280;
    cw.document.documentElement.setAttribute('lang', 'en');
    const d = factory(cw, false).inspectPage(cw);
    expect(kinds(d).layout).not.toContain('overflow-x');
    expect(kinds(d).layout).not.toContain('page-overflow-x');
    expect(kinds(d).a11y).not.toContain('html-lang');
    expect(d.lang).toBe('en');
  });

  it('installs a listener in the injected form only inside a frame', async () => {
    const frame = makeFrame();
    const cw = frame.contentWindow as Win & { __geniusPageInspector?: boolean };
    stubLayout(cw, () => 800);
    // exactly what inspector.rs injects: `(<file>)(window, true);`
    new Function('window', `(${pageSrc.trimEnd()})(window, true);`)(cw);
    expect(cw.__geniusPageInspector).toBe(true);
    const top = window as Win & { __geniusPageInspector?: boolean };
    factory(top, true);
    expect(top.__geniusPageInspector).toBeUndefined();
    const parent = cw.parent as Win;
    const real = parent.postMessage;
    const got: unknown[] = [];
    parent.postMessage = ((m: unknown) => void got.push(m)) as Window['postMessage'];
    try {
      cw.dispatchEvent(new cw.MessageEvent('message', { data: { type: 'genius:inspect', requestId: 'x1' }, source: cw }));
      cw.dispatchEvent(new cw.MessageEvent('message', { data: { type: 'genius:inspect', requestId: 'x2' }, source: parent }));
      await new Promise((r) => setTimeout(r, 200));
    } finally {
      parent.postMessage = real;
    }
    expect(got).toHaveLength(1);
    expect(got[0]).toMatchObject({ type: 'genius:inspection', requestId: 'x2', report: { viewport: { width: 800 } } });
  });

  it('detects a blank page and bounds every list', () => {
    const frame = makeFrame();
    const cw = frame.contentWindow as Win;
    cw.document.body.innerHTML = '<div id="root"></div>';
    stubLayout(cw, () => 800);
    cw.document.getElementById('root')!.getBoundingClientRect = () => new cw.DOMRect(0, 0, 0, 0);
    const blank = factory(cw, false).inspectPage(cw);
    expect(blank.rendered).toEqual({ root: '#root', visibleElements: 0, textLength: 0, blank: true });
    expect(kinds(blank).a11y).not.toContain('no-h1');

    cw.document.body.innerHTML = '<div id="root"><h2>No h1 here</h2>' + Array.from({ length: 50 }, (_, i) => `<img src="/i${i}.png"><button></button>`).join('') + '</div>';
    const many = factory(cw, false).inspectPage(cw);
    expect(many.a11y.filter((i) => i.kind === 'img-alt')).toHaveLength(8);
    expect(many.a11y.filter((i) => i.kind === 'button-name')).toHaveLength(8);
    expect(kinds(many).a11y).toContain('no-h1');
    expect(many.truncated).toBe(true);
  });
});

// ---------------------------------------------------------------- the window-side responder

describe('preview responder', () => {
  const ORIGIN = 'http://127.0.0.1:5199';
  let unbridge: (() => void) | null = null;
  const savedClientRects = HTMLIFrameElement.prototype.getClientRects;

  afterEach(() => {
    unbridge?.();
    unbridge = null;
    HTMLIFrameElement.prototype.getClientRects = savedClientRects;
    for (const [w, fn] of savedRect) w.Element.prototype.getBoundingClientRect = fn;
    savedRect.clear();
    cleanup();
    clearMocks();
    document.body.innerHTML = '';
  });

  const shown = () => {
    HTMLIFrameElement.prototype.getClientRects = () => [new DOMRect(0, 0, 800, 600)] as unknown as DOMRectList;
  };

  /** The preview page at the width of its device frame (`100%` = an 800px panel). */
  function page(frame: HTMLIFrameElement, container: HTMLElement) {
    const cw = frame.contentWindow as Win;
    stubLayout(cw, () => (container.style.width.endsWith('px') ? parseInt(container.style.width, 10) : 800));
    factory(cw, true);
    unbridge = bridge(frame, ORIGIN);
  }

  it('inspects at the current width, then mobile and desktop, and restores the frame', async () => {
    const container = document.createElement('div');
    container.style.width = '100%';
    container.style.maxWidth = '100%';
    document.body.appendChild(container);
    const frame = makeFrame(container);
    page(frame, container);
    const t: PreviewTarget = { frame, container, origin: ORIGIN, projectId: 'p1' };
    const widthsSeen: string[] = [];
    const obs = new MutationObserver(() => widthsSeen.push(container.style.width));
    obs.observe(container, { attributes: true });
    const r = await inspectPreview(t, [375, 800, 1280], { wait: 0 });
    obs.disconnect();
    expect(r.viewports.map((v) => [v.label, v.requestedWidth, v.viewport.width])).toEqual([
      ['current', null, 800],
      ['mobile', 375, 375],
      ['desktop', 1280, 1280],
    ]);
    expect(r.notes).toEqual(['800px is the current width (see "current")']);
    expect(kinds(r.viewports[1]).layout).toContain('overflow-x');
    expect(kinds(r.viewports[2]).layout).not.toContain('overflow-x');
    expect(r.url).toBe(frame.contentWindow!.location.href);
    expect(widthsSeen).toContain('375px');
    expect([container.style.width, container.style.maxWidth, container.style.transition]).toEqual(['100%', '100%', '']);
  });

  it('notes a width the page did not answer at and ignores foreign messages', async () => {
    const container = document.createElement('div');
    document.body.appendChild(container);
    const frame = makeFrame(container);
    const cw = frame.contentWindow as Win;
    stubLayout(cw, () => (container.style.width ? parseInt(container.style.width, 10) : 800));
    let n = 0;
    // First request answered (plus a forged answer from another origin), the second never.
    cw.postMessage = ((msg: { requestId: string }) => {
      if (n++ > 0) return;
      const report = factory(cw, false).inspectPage(cw);
      setTimeout(() => {
        window.dispatchEvent(new MessageEvent('message', { origin: 'http://evil.test', source: cw, data: { type: 'genius:inspection', requestId: msg.requestId, report: { ...report, title: 'forged' } } }));
        window.dispatchEvent(new MessageEvent('message', { origin: ORIGIN, source: cw, data: { type: 'genius:inspection', requestId: msg.requestId, report } }));
      }, 0);
    }) as unknown as Window['postMessage'];
    const r = await inspectPreview({ frame, container, origin: ORIGIN, projectId: 'p1' }, [375], { wait: 0, timeoutMs: 50 });
    expect(r.title).not.toBe('forged');
    expect(r.viewports).toHaveLength(1);
    expect(r.notes[0]).toMatch(/^375px: The preview page did not answer/);
    expect(container.style.width).toBe('');
  });

  it('validates reports from the page', () => {
    expect(parsePageReport(null)).toBeNull();
    const p = parsePageReport({ url: 1, viewport: { width: 'x', height: 600.4 }, rendered: { blank: 'yes' }, headings: [{ level: 9, text: 'h' }, { level: 1 }], layout: [{ kind: 'overflow-x', message: 'm', selector: 3 }, { kind: 1 }], landmarks: ['main', 5], text: 'x'.repeat(5000) })!;
    expect(p.url).toBe('');
    expect(p.viewport).toEqual({ width: 0, height: 600 });
    expect(p.rendered).toEqual({ root: 'body', visibleElements: 0, textLength: 0, blank: false });
    expect(p.headings).toEqual([{ level: 6, text: 'h' }]);
    expect(p.layout).toEqual([{ kind: 'overflow-x', message: 'm', selector: null }]);
    expect(p.landmarks).toEqual(['main']);
    expect(p.text).toHaveLength(1000);
    expect([375, 820, 1280].map(labelFor)).toEqual(['mobile', 'tablet', 'desktop']);
  });

  it('answers the backend through the real preview panel, or explains why it cannot', async () => {
    installIpc();
    useApp.setState({ settings });
    useWorkspace.setState({ projectId: 'p1', project, tasks: [], previewErrors: [], activeConversationId: null, preview: { status: 'ready', url: `${ORIGIN}/`, nonce: 0 } });
    render(<PreviewPanel onStart={() => undefined} />);
    const frame = screen.getByTitle('Application preview') as HTMLIFrameElement;
    const container = frame.parentElement!;
    fill(frame);
    page(frame, container);
    const answers = () => calls.filter((c) => c.cmd === 'report_preview_inspection').map((c) => c.args as { requestId: string; report: InspectionReport | null; error: string | null });

    // Hidden (e.g. the Assistant tab is in front): a clear error, the frame untouched.
    await act(() => handleInspectRequest({ projectId: 'p1', requestId: 'r0', widths: [375] }));
    expect(answers()[0]).toMatchObject({ requestId: 'r0', report: null, error: expect.stringContaining('The preview is hidden') });

    // Another project: nothing of it is shown.
    shown();
    await act(() => handleInspectRequest({ projectId: 'p2', requestId: 'r1', widths: [] }));
    expect(answers()[1].error).toContain('not shown in the Genius window');

    await act(() => handleInspectRequest({ projectId: 'p1', requestId: 'r2', widths: [375, 1280] }));
    const ok = answers()[2];
    expect(ok.error).toBeNull();
    expect(ok.report!.viewports.map((v) => v.label)).toEqual(['current', 'mobile', 'desktop']);
    expect(ok.report!.viewports[1].layout.some((i) => i.kind === 'overflow-x' && i.selector === 'main > div.table')).toBe(true);
    expect([container.style.width, container.style.maxWidth]).toEqual(['100%', '100%']);
  });

  it('unregisters targets', async () => {
    installIpc();
    shown();
    const frame = document.createElement('iframe');
    document.body.appendChild(frame);
    const off = registerPreviewTarget(() => ({ frame, container: document.body, origin: ORIGIN, projectId: 'p9' }));
    off();
    await handleInspectRequest({ projectId: 'p9', requestId: 'r9', widths: [] });
    expect(calls.find((c) => c.cmd === 'report_preview_inspection')!.args.error).toContain('not shown');
  });
});

beforeEach(() => {
  document.documentElement.removeAttribute('lang');
});
