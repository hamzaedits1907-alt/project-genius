//! Preview element inspector.
//!
//! When the dev server starts, Genius points Vite at a wrapper config stored in
//! `node_modules/.genius/` (never committed, never exported, never shown to the model).
//! The wrapper loads the project's own Vite config unchanged and adds a serve-only plugin
//! that injects a small script into the page (first thing in `<head>`, before the app's own
//! scripts). The scripts have these jobs, all one-way reports to the parent window as data:
//!
//! * Element picker: inert until the Genius window asks it (via `postMessage`) to enter pick
//!   mode; it then reports a description of the clicked element (tag, classes, text, ancestry).
//! * Runtime errors: uncaught errors, unhandled rejections, console.error/warn (original
//!   behaviour kept), failed resource loads, Vite's error overlay and blank pages are
//!   deduplicated, rate-limited, clipped and posted as `genius:runtime` (see `runtime_errors.rs`).
//!   After a load or HMR update with no errors and a visible page it posts
//!   `genius:runtime-healthy`, so stale problems can be cleared.
//!
//! * Page inspection (`inspect_page.js`, a second script): answers a `genius:inspect` request
//!   from the parent with a bounded report of what rendered, the outline, layout and
//!   accessibility problems (see `inspection.rs`).
//!
//! It has no access to Genius itself: the preview runs on its own origin.

use std::path::{Path, PathBuf};

use crate::error::Result;

const CONFIG_CANDIDATES: &[&str] = &["vite.config.ts", "vite.config.mts", "vite.config.js", "vite.config.mjs"];

pub const CLIENT_SCRIPT: &str = r#"(() => {
  if (window.__geniusInspector) return;
  window.__geniusInspector = true;
  let active = false;
  let box = null;
  const css = (el) => {
    let s = el.tagName.toLowerCase();
    if (el.id) s += '#' + el.id;
    const cls = (typeof el.className === 'string' ? el.className : '').trim().split(/\s+/).filter(Boolean).slice(0, 3);
    if (cls.length) s += '.' + cls.join('.');
    return s;
  };
  const describe = (el) => {
    const path = [];
    for (let n = el; n && n.nodeType === 1 && path.length < 6 && n !== document.body; n = n.parentElement) path.unshift(css(n));
    const attrs = {};
    for (const a of ['aria-label', 'role', 'data-testid', 'name', 'type', 'href', 'placeholder', 'alt', 'title']) {
      const v = el.getAttribute && el.getAttribute(a);
      if (v) attrs[a] = String(v).slice(0, 120);
    }
    const r = el.getBoundingClientRect();
    return {
      tag: el.tagName.toLowerCase(),
      selector: path.join(' > '),
      text: (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 160),
      attributes: attrs,
      html: (el.outerHTML || '').slice(0, 600),
      size: { width: Math.round(r.width), height: Math.round(r.height) },
      page: location.pathname,
    };
  };
  const show = (el) => {
    if (!box) {
      box = document.createElement('div');
      box.style.cssText = 'position:fixed;pointer-events:none;z-index:2147483647;border:2px solid #7c6cff;background:rgba(124,108,255,.12);border-radius:3px;transition:all .05s';
      document.documentElement.appendChild(box);
    }
    const r = el.getBoundingClientRect();
    Object.assign(box.style, { left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px', display: 'block' });
  };
  const stop = () => { active = false; if (box) box.style.display = 'none'; document.documentElement.style.cursor = ''; };
  window.addEventListener('message', (e) => {
    if (e.source !== window.parent || !e.data || e.data.type !== 'genius:pick') return;
    active = !!e.data.on;
    if (active) document.documentElement.style.cursor = 'crosshair'; else stop();
  });
  document.addEventListener('mousemove', (e) => { if (active && e.target instanceof Element) show(e.target); }, true);
  document.addEventListener('click', (e) => {
    if (!active || !(e.target instanceof Element)) return;
    e.preventDefault();
    e.stopPropagation();
    const info = describe(e.target);
    stop();
    window.parent.postMessage({ type: 'genius:picked', element: info }, '*');
  }, true);
  document.addEventListener('keydown', (e) => {
    if (active && e.key === 'Escape') { stop(); window.parent.postMessage({ type: 'genius:pick-cancelled' }, '*'); }
  }, true);

  // ---- runtime errors. Never throws into the app; reports are data for the parent only.
  if (window.parent === window) return;
  const post = (m) => { try { window.parent.postMessage(m, '*'); } catch (_) { /* ignore */ } };
  const clip = (v, n) => { const s = String(v == null ? '' : v); return s.length > n ? s.slice(0, n) + '…' : s; };
  const str = (v) => {
    if (v instanceof Error || (v && typeof v === 'object' && typeof v.message === 'string' && 'stack' in v)) return String(v.name || 'Error') + ': ' + v.message;
    if (typeof v === 'string') return v;
    try { return JSON.stringify(v) ?? String(v); } catch (_) { return String(v); }
  };
  // Prefer the first stack frame in the project's own code (src/...) as the location.
  const frame = (stack) => {
    const all = String(stack || '').match(/(?:https?:\/\/[^\s()]+|\/[^\s():]+):\d+:\d+/g) || [];
    const f = all.find((x) => x.indexOf('/src/') >= 0) || all[0];
    const m = f && f.match(/^(.*):(\d+):(\d+)$/);
    return m ? { source: m[1], line: +m[2], column: +m[3] } : {};
  };
  const page = () => location.pathname + location.hash;
  const seen = new Map();
  // Totals since the page loaded, read by the page inspector (`inspect_page.js`).
  const counts = window.__geniusCounts = { errors: 0, warnings: 0 };
  let sent = [];
  let timer = null;
  let errSince = false;
  let loaded = false;
  let checkTimer = null;
  // Each distinct error is posted at once, repeats at most every 5 s (as a count), and no
  // more than 20 errors per 10 s leave the page; the rest wait for the next flush.
  const flush = () => {
    timer = null;
    const now = Date.now();
    sent = sent.filter((t) => now - t < 10000);
    const out = [];
    let later = false;
    for (const s of seen.values()) {
      if (!s.n) continue;
      if ((s.last && now - s.last < 5000) || sent.length >= 20) { later = true; continue; }
      out.push(Object.assign({}, s.e, { count: s.n }));
      s.n = 0;
      s.last = now;
      sent.push(now);
    }
    if (out.length) post({ type: 'genius:runtime', errors: out });
    if (later && !timer) timer = setTimeout(flush, 2000);
  };
  const report = (e) => {
    try {
      e.message = clip(e.message, 1000);
      if (!e.message && e.kind !== 'blank') return;
      if (e.stack) e.stack = clip(e.stack, 2000); else delete e.stack;
      if (e.source) e.source = clip(e.source, 300); else delete e.source;
      if (!(e.line > 0)) { delete e.line; delete e.column; }
      if (e.kind !== 'console') errSince = true;
      if (e.kind === 'console' && e.source === 'console.warn') counts.warnings++; else if (e.kind !== 'blank') counts.errors++;
      const key = [e.kind, e.message, e.source || '', e.line || ''].join('|');
      let s = seen.get(key);
      if (!s) {
        if (seen.size > 500) seen.clear();
        s = { n: 0, last: 0, e: null };
        seen.set(key, s);
      }
      s.n++;
      s.e = Object.assign(e, { page: clip(page(), 300), at: new Date().toISOString() });
      if (!timer) timer = setTimeout(flush, 250);
    } catch (_) { /* never break the app */ }
  };
  const TAGS = { script: 'script', link: 'stylesheet', img: 'image', source: 'media', video: 'video', audio: 'audio' };
  window.addEventListener('error', (ev) => {
    const t = ev.target;
    if (t && t !== window && t.nodeType === 1) {
      const tag = t.tagName.toLowerCase();
      const url = t.currentSrc || t.src || t.href || '';
      if (TAGS[tag]) report({ kind: 'resource', message: 'Failed to load ' + TAGS[tag] + ': ' + url, source: url });
      return;
    }
    const er = ev.error;
    const f = frame(er && er.stack);
    report({ kind: 'error', message: er ? str(er) : ev.message, source: f.source || ev.filename, line: f.line || ev.lineno, column: f.column || ev.colno, stack: er && er.stack });
  }, true);
  window.addEventListener('unhandledrejection', (ev) => {
    const r = ev.reason;
    report(Object.assign({ kind: 'rejection', message: str(r), stack: r && r.stack }, frame(r && r.stack)));
  });
  const fmt = (args) => {
    const rest = args.slice(1);
    if (typeof args[0] !== 'string') return str(args[0]);
    return args[0].replace(/%[sdifoOc]/g, (m) => (rest.length ? (m === '%c' ? (rest.shift(), '') : str(rest.shift())) : m));
  };
  for (const level of ['error', 'warn']) {
    const orig = console[level];
    if (typeof orig !== 'function') continue;
    console[level] = function (...args) {
      try {
        const err = args.find((a) => a instanceof Error);
        report({ kind: 'console', message: fmt(args), source: 'console.' + level, stack: err && err.stack });
      } catch (_) { /* ignore */ }
      return orig.apply(this, args);
    };
  }
  const readOverlay = (el) => {
    const q = (sel) => { const n = el.shadowRoot && el.shadowRoot.querySelector(sel); return n ? (n.textContent || '').trim() : ''; };
    const file = q('.file').match(/^(.*?):(\d+):(\d+)$/);
    report({ kind: 'vite', message: q('.message') || (el.textContent || '').trim() || 'Vite reported an error', source: file ? file[1] : undefined, line: file ? +file[2] : undefined, column: file ? +file[3] : undefined, stack: [q('.frame'), q('.stack')].filter(Boolean).join('\n') });
  };
  const hasContent = (root) => {
    if (!((root.innerText || root.textContent || '').trim() || root.querySelector('img,svg,canvas,video,iframe,input,button,select,textarea,picture,object'))) return false;
    if (root.getBoundingClientRect().height > 0) return true;
    const all = root.querySelectorAll('*');
    for (let i = 0; i < all.length && i < 300; i++) { const r = all[i].getBoundingClientRect(); if (r.width > 0 && r.height > 0) return true; }
    return false;
  };
  // ~2.5 s after load / an HMR update: report a blank page, or that all is well.
  const check = () => {
    checkTimer = null;
    try {
      const overlay = document.querySelector('vite-error-overlay');
      if (overlay) return readOverlay(overlay);
      const root = document.getElementById('root') || document.body;
      if (!root) return;
      if (!hasContent(root)) {
        const what = root.id ? '#' + root.id : 'body';
        report({ kind: 'blank', message: root.childElementCount ? 'The page shows nothing: ' + what + ' has elements but none of them is visible.' : 'The page shows nothing: ' + what + ' is empty (the app rendered nothing or crashed before rendering).' });
      } else if (!errSince) post({ type: 'genius:runtime-healthy', page: page() });
    } catch (_) { /* ignore */ }
  };
  const settle = () => { errSince = false; clearTimeout(checkTimer); checkTimer = setTimeout(check, 2500); };
  // Vite's client logs "[vite] hot updated: …" through console.debug after each HMR update.
  const dbg = console.debug;
  if (typeof dbg === 'function') {
    console.debug = function (...args) {
      try { if (typeof args[0] === 'string' && args[0].indexOf('[vite] hot updated') === 0) settle(); } catch (_) { /* ignore */ }
      return dbg.apply(this, args);
    };
  }
  window.addEventListener('load', () => { loaded = true; settle(); });
  try {
    new MutationObserver((list) => {
      for (const m of list) for (const n of m.addedNodes) if (n.nodeName === 'VITE-ERROR-OVERLAY') setTimeout(() => readOverlay(n), 0);
      const root = document.getElementById('root');
      // The tree was unmounted (e.g. a crash after a click): check whether the page went blank.
      if (loaded && root && !root.firstChild && !checkTimer) checkTimer = setTimeout(check, 2500);
    }).observe(document.documentElement, { childList: true, subtree: true });
  } catch (_) { /* ignore */ }
})();"#;

/// Page inspection for visual verification (`genius:inspect` → `genius:inspection`), shared
/// verbatim with the frontend test suite. See `inspection.rs` for the Genius side.
pub const PAGE_INSPECTOR_SOURCE: &str = include_str!("inspect_page.js");

/// The injected form of [`PAGE_INSPECTOR_SOURCE`]: called with the page's window, listening.
pub fn page_inspector_script() -> String {
    format!("({})(window, true);", PAGE_INSPECTOR_SOURCE.trim_end())
}

fn wrapper_config(user_config: &str) -> String {
    format!(
        r#"// Generated by Genius for the live preview. Not part of your project; safe to delete.
import {{ mergeConfig }} from 'vite';
import userConfig from '../../{user_config}';

const CLIENT = {client};
const INSPECT = {inspect};

const geniusInspector = {{
  name: 'genius-preview-inspector',
  apply: 'serve',
  transformIndexHtml() {{
    // head-prepend: installed before the app's own scripts so early errors are captured
    return [
      {{ tag: 'script', children: CLIENT, injectTo: 'head-prepend' }},
      {{ tag: 'script', children: INSPECT, injectTo: 'head-prepend' }},
    ];
  }},
}};

export default async (env) => {{
  const base = typeof userConfig === 'function' ? await userConfig(env) : await userConfig;
  return mergeConfig(base ?? {{}}, {{ plugins: [geniusInspector] }});
}};
"#,
        client = serde_json::to_string(CLIENT_SCRIPT).unwrap_or_else(|_| "''".into()),
        inspect = serde_json::to_string(&page_inspector_script()).unwrap_or_else(|_| "''".into())
    )
}

/// Write the wrapper config and return its path relative to the project root, or `None`
/// when the project has no recognised Vite config or dependencies are not installed.
pub fn prepare(root: &Path) -> Result<Option<String>> {
    let Some(user) = CONFIG_CANDIDATES.iter().find(|c| root.join(c).is_file()) else {
        return Ok(None);
    };
    let nm = root.join("node_modules");
    if !nm.join("vite").is_dir() {
        return Ok(None);
    }
    let dir: PathBuf = nm.join(".genius");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("preview.config.mjs");
    std::fs::write(&file, wrapper_config(user))?;
    Ok(Some("node_modules/.genius/preview.config.mjs".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_wrapper_only_when_applicable() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(prepare(d.path()).unwrap(), None, "no config");
        std::fs::write(d.path().join("vite.config.ts"), "export default {}").unwrap();
        assert_eq!(prepare(d.path()).unwrap(), None, "vite not installed");
        std::fs::create_dir_all(d.path().join("node_modules/vite")).unwrap();
        let rel = prepare(d.path()).unwrap().unwrap();
        let text = std::fs::read_to_string(d.path().join(&rel)).unwrap();
        assert!(text.contains("import userConfig from '../../vite.config.ts'"));
        assert!(text.contains("apply: 'serve'"));
        assert!(text.contains("genius:picked"));
        assert!(text.contains("genius:runtime") && text.contains("unhandledrejection") && text.contains("vite-error-overlay"));
        assert!(text.contains("injectTo: 'head-prepend'"), "installed before the app's scripts");
        assert!(text.contains("const INSPECT = ") && text.contains("genius:inspect") && text.contains("(window, true);"), "page inspector injected and listening");
        assert!(text.contains("__geniusCounts"), "console counters for the inspector");
    }
}
