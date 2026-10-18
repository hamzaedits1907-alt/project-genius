function geniusPageInspector(win, install) {
  // Visual verification of the live preview, computed inside the real page.
  //
  // This file is shipped verbatim: `inspector.rs` injects `(<this file>)(window, true)` into the
  // preview, and the Vitest suite evaluates the same text, so tested code == shipped code.
  // It only reads the page (DOM, computed styles, layout boxes) and answers a `genius:inspect`
  // request from the parent window with a bounded report. It never throws into the app.
  'use strict';
  var LIMITS = { elements: 4000, perKind: 8, issues: 60, contrastSamples: 250, headings: 40, landmarks: 30, text: 800, label: 120, fixed: 30 };

  var clip = function (s, n) {
    s = String(s == null ? '' : s).replace(/\s+/g, ' ').trim();
    return s.length > n ? s.slice(0, n) + '…' : s;
  };

  // ---------------------------------------------------------------- colour (WCAG 2.x)
  var clamp255 = function (x) { return Math.max(0, Math.min(255, x)); };
  var num = function (p, scale) {
    if (p === 'none') return 0;
    if (/%$/.test(p)) return (parseFloat(p) / 100) * scale;
    return parseFloat(p);
  };
  // linear-light sRGB (0..1) -> 8-bit sRGB
  var encode = function (c) {
    c = Math.max(0, Math.min(1, c));
    return 255 * (c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055);
  };
  var fromOklab = function (L, a, b, alpha) {
    var l = Math.pow(L + 0.3963377774 * a + 0.2158037573 * b, 3);
    var m = Math.pow(L - 0.1055613458 * a - 0.0638541728 * b, 3);
    var s = Math.pow(L - 0.0894841775 * a - 1.291485548 * b, 3);
    return {
      r: encode(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s),
      g: encode(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s),
      b: encode(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s),
      a: alpha,
    };
  };

  /** Parse a CSS colour as returned by getComputedStyle: rgb()/rgba() (comma or space
   *  syntax), #hex, transparent, color(srgb …), oklab(), oklch(). Returns {r,g,b,a} or null. */
  function parseColor(v) {
    if (!v) return null;
    v = String(v).trim().toLowerCase();
    if (v === 'transparent') return { r: 0, g: 0, b: 0, a: 0 };
    var m = /^#([0-9a-f]{3,8})$/.exec(v);
    if (m) {
      var h = m[1];
      if (h.length === 3 || h.length === 4) h = h.split('').map(function (c) { return c + c; }).join('');
      if (h.length !== 6 && h.length !== 8) return null;
      return { r: parseInt(h.slice(0, 2), 16), g: parseInt(h.slice(2, 4), 16), b: parseInt(h.slice(4, 6), 16), a: h.length === 8 ? parseInt(h.slice(6, 8), 16) / 255 : 1 };
    }
    m = /^(rgba?|oklab|oklch|color)\(\s*([^)]*)\)$/.exec(v);
    if (!m) return null;
    var parts = m[2].split(/\s*[,/]\s*|\s+/).filter(Boolean);
    var fn = m[1];
    var space = null;
    if (fn === 'color') {
      space = parts.shift();
      if (space !== 'srgb') return null;
    }
    if (parts.length < 3) return null;
    var alpha = parts.length > 3 ? num(parts[3], 1) : 1;
    var c;
    if (fn === 'rgb' || fn === 'rgba') c = { r: num(parts[0], 255), g: num(parts[1], 255), b: num(parts[2], 255), a: alpha };
    else if (fn === 'color') c = { r: num(parts[0], 1) * 255, g: num(parts[1], 1) * 255, b: num(parts[2], 1) * 255, a: alpha };
    else if (fn === 'oklab') c = fromOklab(num(parts[0], 1), num(parts[1], 0.4), num(parts[2], 0.4), alpha);
    else {
      var L = num(parts[0], 1), C = num(parts[1], 0.4), H = (parseFloat(parts[2]) || 0) * Math.PI / 180;
      c = fromOklab(L, C * Math.cos(H), C * Math.sin(H), alpha);
    }
    if (![c.r, c.g, c.b, c.a].every(isFinite)) return null;
    return { r: clamp255(c.r), g: clamp255(c.g), b: clamp255(c.b), a: Math.max(0, Math.min(1, c.a)) };
  }

  function luminance(c) {
    var ch = function (x) { x /= 255; return x <= 0.04045 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4); };
    return 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b);
  }

  /** WCAG contrast ratio of two opaque colours (1..21). */
  function contrastRatio(a, b) {
    var la = luminance(a), lb = luminance(b);
    return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
  }

  /** Composite `top` over `bottom` (source-over). */
  function blend(top, bottom) {
    var a = top.a + bottom.a * (1 - top.a);
    if (a <= 0) return { r: 0, g: 0, b: 0, a: 0 };
    var mix = function (t, b) { return (t * top.a + b * bottom.a * (1 - top.a)) / a; };
    return { r: mix(top.r, bottom.r), g: mix(top.g, bottom.g), b: mix(top.b, bottom.b), a: a };
  }

  /** Background behind `el`: its own and its ancestors' background colours composited over
   *  white (the default canvas). null when a background image or gradient is involved, because
   *  the real colour behind the text is then unknown. */
  function effectiveBackground(el, styleOf) {
    var layers = [];
    for (var n = el; n && n.nodeType === 1; n = n.parentElement) {
      var st = styleOf(n);
      if (!st) break;
      if (st.backgroundImage && st.backgroundImage !== 'none') return null;
      var c = parseColor(st.backgroundColor);
      if (c && c.a > 0) {
        layers.push(c);
        if (c.a >= 1) break;
      }
    }
    var out = { r: 255, g: 255, b: 255, a: 1 };
    for (var i = layers.length - 1; i >= 0; i--) out = blend(layers[i], out);
    return out;
  }

  /** WCAG "large text": at least 24px, or 18.66px (14pt) bold. */
  function isLargeText(px, weight) {
    var w = weight === 'bold' ? 700 : parseInt(weight, 10) || 400;
    return px >= 24 || (px >= 18.66 && w >= 700);
  }

  /** Contrast check for text of colour `fg` (may be translucent) on opaque `bg`. */
  function checkContrast(fg, bg, px, weight) {
    var text = fg.a < 1 ? blend(fg, bg) : fg;
    var ratio = contrastRatio(text, bg);
    var required = isLargeText(px, weight) ? 3 : 4.5;
    return { ratio: Math.round(ratio * 100) / 100, required: required, ok: ratio + 1e-9 >= required };
  }

  var hex = function (c) {
    return '#' + [c.r, c.g, c.b].map(function (x) { var s = Math.round(x).toString(16); return s.length < 2 ? '0' + s : s; }).join('');
  };

  // ---------------------------------------------------------------- geometry
  /** Pixels by which a box sticks out of a viewport `vw` wide (left or right), 0 when inside. */
  function horizontalOverflow(rect, vw) {
    return Math.round(Math.max(0, rect.right - vw, -rect.left));
  }

  function intersectionArea(a, b) {
    var w = Math.min(a.right, b.right) - Math.max(a.left, b.left);
    var h = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    return w > 0 && h > 0 ? w * h : 0;
  }

  /** Heading levels in document order -> skipped levels (h2 followed by h4, …). */
  function headingSkips(levels) {
    var out = [];
    var prev = 0;
    for (var i = 0; i < levels.length; i++) {
      var l = levels[i];
      if (prev && l > prev + 1) out.push({ from: prev, to: l });
      prev = l;
    }
    return out;
  }

  // ---------------------------------------------------------------- DOM helpers
  function selectorOf(el) {
    var css = function (n) {
      var s = n.tagName.toLowerCase();
      if (n.id) return s + '#' + n.id;
      var cls = (typeof n.className === 'string' ? n.className : '').trim().split(/\s+/).filter(Boolean).slice(0, 2);
      if (cls.length) s += '.' + cls.join('.');
      return s;
    };
    var path = [];
    for (var n = el; n && n.nodeType === 1 && n.tagName !== 'BODY' && n.tagName !== 'HTML' && path.length < 4; n = n.parentElement) {
      path.unshift(css(n));
      if (n.id) break;
    }
    if (!path.length) path.push(el.tagName.toLowerCase());
    return clip(path.join(' > '), 160);
  }

  var hiddenFromAT = function (el) { return !!(el.closest && el.closest('[aria-hidden="true"]')); };

  /** Text an assistive technology would use as content: text nodes plus image alts, skipping
   *  aria-hidden subtrees, scripts and styles. */
  function textOf(node, depth) {
    depth = depth || 0;
    if (!node || depth > 30) return '';
    if (node.nodeType === 3) return node.nodeValue || '';
    if (node.nodeType !== 1) return '';
    var tag = node.tagName.toLowerCase();
    if (tag === 'script' || tag === 'style' || tag === 'template' || tag === 'noscript') return '';
    if (node.getAttribute('aria-hidden') === 'true') return '';
    if (node.getAttribute('aria-label')) return node.getAttribute('aria-label');
    if (tag === 'img' || (tag === 'input' && node.type === 'image')) return node.getAttribute('alt') || '';
    if (tag === 'svg') {
      var t = node.querySelector('title');
      return t ? t.textContent || '' : '';
    }
    var s = '';
    for (var c = node.firstChild; c; c = c.nextSibling) s += ' ' + textOf(c, depth + 1);
    return s;
  }

  /** Accessible name (simplified accname): aria-labelledby, aria-label, <label>, alt, content, title. */
  function accessibleName(el) {
    var doc = el.ownerDocument;
    var by = el.getAttribute('aria-labelledby');
    if (by) {
      var t = by.split(/\s+/).map(function (id) { var n = doc.getElementById(id); return n ? textOf(n) : ''; }).join(' ');
      if (t.trim()) return clip(t, LIMITS.label);
    }
    var al = el.getAttribute('aria-label');
    if (al && al.trim()) return clip(al, LIMITS.label);
    var tag = el.tagName.toLowerCase();
    var isField = tag === 'select' || tag === 'textarea' || (tag === 'input' && !/^(submit|button|reset|image)$/.test(el.type));
    if (isField || tag === 'input') {
      var labels = el.labels ? Array.prototype.slice.call(el.labels) : [];
      var lt = labels.map(function (l) { return textOf(l); }).join(' ');
      if (lt.trim()) return clip(lt, LIMITS.label);
    }
    if (tag === 'input' && /^(submit|button|reset)$/.test(el.type)) {
      if (el.value) return clip(el.value, LIMITS.label);
      if (el.type !== 'button') return el.type === 'submit' ? 'Submit' : 'Reset';
    }
    if (tag === 'input' && el.type === 'image' && el.getAttribute('alt')) return clip(el.getAttribute('alt'), LIMITS.label);
    if (!isField) {
      var content = textOf(el);
      if (content.trim()) return clip(content, LIMITS.label);
    }
    var title = el.getAttribute('title');
    if (title && title.trim()) return clip(title, LIMITS.label);
    return '';
  }

  /** Visible text when `innerText` is unavailable: text nodes outside scripts, styles and
   *  display:none subtrees. */
  function plainText(node, styleOf, depth) {
    depth = depth || 0;
    if (!node || depth > 40) return '';
    if (node.nodeType === 3) return node.nodeValue || '';
    if (node.nodeType !== 1 || /^(script|style|template|noscript)$/i.test(node.tagName)) return '';
    var st = styleOf(node);
    if (st && st.display === 'none') return '';
    var s = '';
    for (var c = node.firstChild; c; c = c.nextSibling) s += ' ' + plainText(c, styleOf, depth + 1);
    return s;
  }

  var landmarkOf = function (el) {
    var role = el.getAttribute('role');
    if (role && /^(banner|navigation|main|complementary|contentinfo|search|region|form)$/.test(role)) return role;
    var tag = el.tagName.toLowerCase();
    if (tag === 'main' || tag === 'nav' || tag === 'aside') return tag;
    if (tag === 'header' || tag === 'footer') return el.closest('article,aside,main,nav,section') && el.closest('article,aside,main,nav,section') !== el ? null : tag;
    if ((tag === 'section' || tag === 'form') && (el.getAttribute('aria-label') || el.getAttribute('aria-labelledby'))) return tag;
    return null;
  };

  // ---------------------------------------------------------------- the report
  /** Inspect the page as rendered now. `opts.consoleCounts` overrides the counters kept by the
   *  runtime-error script (window.__geniusCounts). */
  function inspectPage(w, opts) {
    opts = opts || {};
    var doc = w.document;
    var vw = w.innerWidth || doc.documentElement.clientWidth || 0;
    var vh = w.innerHeight || doc.documentElement.clientHeight || 0;
    var cache = new Map();
    var styleOf = function (el) {
      if (cache.has(el)) return cache.get(el);
      var st = null;
      try { st = w.getComputedStyle(el); } catch (_) { st = null; }
      cache.set(el, st);
      return st;
    };
    var layout = [];
    var a11y = [];
    var perKind = {};
    var truncated = false;
    var add = function (list, kind, message, el, extra) {
      perKind[kind] = (perKind[kind] || 0) + 1;
      if (perKind[kind] > LIMITS.perKind || layout.length + a11y.length >= LIMITS.issues) { truncated = true; return; }
      var it = { kind: kind, message: clip(message, 240) };
      if (el) it.selector = selectorOf(el);
      if (extra) for (var k in extra) it[k] = extra[k];
      list.push(it);
    };

    var root = doc.getElementById('root') || doc.getElementById('app') || doc.body;
    var all = doc.body ? doc.body.querySelectorAll('*') : [];
    if (all.length > LIMITS.elements) truncated = true;

    // Per-element pass in document order, so parents are always seen before children.
    var skipped = new Set(); // not rendered: display:none (or inside such a subtree), scripts
    var clipX = new Map(); // el -> true when an ancestor clips/scrolls horizontally
    var reportedOverflow = new Map();
    var visibleElements = 0;
    var mediaVisible = 0;
    var headings = [];
    var levels = [];
    var hasH1 = false;
    var landmarks = [];
    var fixed = [];
    var contrastSamples = 0;
    var contrastFails = new Map();

    for (var i = 0; i < all.length && i < LIMITS.elements; i++) {
      var el = all[i];
      var tag = el.tagName.toLowerCase();
      var parent = el.parentElement;
      // A section hidden with display:none hides its whole subtree (fine, not reported).
      if (parent && skipped.has(parent)) { skipped.add(el); continue; }
      if (tag === 'script' || tag === 'style' || tag === 'template' || tag === 'noscript') { skipped.add(el); continue; }
      var st = styleOf(el);
      if (!st || st.display === 'none') { skipped.add(el); continue; }
      var r = el.getBoundingClientRect();
      var visible = r.width > 0 && r.height > 0 && st.visibility !== 'hidden' && st.opacity !== '0';
      if (visible && root.contains(el)) visibleElements++;
      if (visible && /^(img|svg|canvas|video|picture|iframe|object)$/.test(tag)) mediaVisible++;

      // ---- layout: horizontal overflow (outermost offender only; clipped content is fine)
      var clipped = !!(parent && (clipX.get(parent) || (function () {
        var ps = styleOf(parent);
        return ps && parent !== doc.body && /(auto|scroll|hidden|clip)/.test(ps.overflowX || ps.overflow || '');
      })()));
      clipX.set(el, clipped);
      var inReported = !!(parent && reportedOverflow.get(parent));
      reportedOverflow.set(el, inReported);
      if (visible && !clipped && !inReported && st.position !== 'fixed' && vw > 0) {
        var over = horizontalOverflow(r, vw);
        if (over > 1) {
          reportedOverflow.set(el, true);
          add(layout, 'overflow-x', 'sticks out ' + over + 'px past the ' + (r.left < 0 ? 'left' : 'right') + ' edge of the ' + vw + 'px viewport (width ' + Math.round(r.width) + 'px)', el, { px: over });
        }
      }
      if (visible && (st.position === 'fixed' || st.position === 'sticky') && fixed.length < LIMITS.fixed) fixed.push({ el: el, rect: r, position: st.position });

      // ---- images
      if (tag === 'img') {
        var src = el.currentSrc || el.getAttribute('src') || '';
        if (el.complete && src && el.naturalWidth === 0) add(layout, 'image-broken', 'image failed to load: ' + clip(src, 140), el);
        else if (el.complete && el.naturalWidth > 1 && (r.width === 0 || r.height === 0) && el.getClientRects().length > 0) add(layout, 'image-zero-size', 'image is loaded (' + el.naturalWidth + '×' + el.naturalHeight + ') but displayed at ' + Math.round(r.width) + '×' + Math.round(r.height), el);
        if (!el.hasAttribute('alt') && !hiddenFromAT(el) && !/^(presentation|none)$/.test(el.getAttribute('role') || '')) add(a11y, 'img-alt', 'image has no alt text' + (src ? ' (' + clip(src.split('/').pop(), 60) + ')' : ''), el);
      }

      var aria = hiddenFromAT(el);
      // ---- names of controls, buttons and links
      if (!aria) {
        var role = el.getAttribute('role');
        var isField = (tag === 'input' && !/^(hidden|submit|button|reset|image)$/.test(el.type)) || tag === 'select' || tag === 'textarea';
        if (isField && visible) {
          if (!accessibleName(el)) add(a11y, 'control-label', (tag === 'input' ? el.type + ' field' : tag) + ' has no label' + (el.getAttribute('placeholder') ? ' (only a placeholder: "' + clip(el.getAttribute('placeholder'), 60) + '")' : ''), el);
        } else if ((tag === 'button' || role === 'button' || (tag === 'input' && /^(submit|button|reset|image)$/.test(el.type))) && visible) {
          if (!accessibleName(el)) add(a11y, 'button-name', 'button has no accessible name (icon-only? add aria-label)', el);
        } else if (tag === 'a' && el.hasAttribute('href') && visible) {
          if (!accessibleName(el)) add(a11y, 'link-name', 'link has no accessible name' + (el.getAttribute('href') ? ' (href ' + clip(el.getAttribute('href'), 60) + ')' : ''), el);
        }
      }

      // ---- outline
      var hm = /^h([1-6])$/.exec(tag);
      var level = hm ? +hm[1] : el.getAttribute('role') === 'heading' ? parseInt(el.getAttribute('aria-level'), 10) || 2 : 0;
      if (level && visible && !aria) {
        levels.push(level);
        if (level === 1) hasH1 = true;
        if (level <= 3 && headings.length < LIMITS.headings) headings.push({ level: level, text: clip(textOf(el), LIMITS.label) });
      }
      var lm = landmarkOf(el);
      if (lm && visible && landmarks.length < LIMITS.landmarks) {
        var depth = 0;
        for (var p = el.parentElement; p; p = p.parentElement) if (landmarkOf(p)) depth++;
        var name = el.getAttribute('aria-label') || '';
        landmarks.push(new Array(depth + 1).join('  ') + lm + (name ? ' "' + clip(name, 40) + '"' : ''));
      }

      // ---- contrast of directly contained text (sampled)
      if (visible && contrastSamples < LIMITS.contrastSamples) {
        var own = '';
        for (var c = el.firstChild; c; c = c.nextSibling) if (c.nodeType === 3) own += c.nodeValue;
        own = own.trim();
        if (own) {
          contrastSamples++;
          var fg = parseColor(st.color);
          var bg = effectiveBackground(el, styleOf);
          if (fg && bg && fg.a > 0) {
            var res = checkContrast(fg, bg, parseFloat(st.fontSize) || 16, st.fontWeight);
            if (!res.ok) {
              var key = hex(fg.a < 1 ? blend(fg, bg) : fg) + '/' + hex(bg) + '/' + res.required;
              var prev = contrastFails.get(key);
              if (prev) prev.count++;
              else contrastFails.set(key, { el: el, text: own, res: res, fg: fg.a < 1 ? blend(fg, bg) : fg, bg: bg, count: 1 });
            }
          }
        }
      }
    }

    contrastFails.forEach(function (f) {
      add(a11y, 'contrast', 'text "' + clip(f.text, 50) + '" has contrast ' + f.res.ratio + ':1 (' + hex(f.fg) + ' on ' + hex(f.bg) + '), needs ' + f.res.required + ':1' + (f.count > 1 ? ' — ' + f.count + ' elements' : ''), f.el, { count: f.count });
    });

    // ---- overlapping fixed/sticky elements (not nested in each other)
    for (var x = 0; x < fixed.length; x++) {
      for (var y = x + 1; y < fixed.length; y++) {
        var A = fixed[x], B = fixed[y];
        if (A.el.contains(B.el) || B.el.contains(A.el)) continue;
        var area = intersectionArea(A.rect, B.rect);
        var smaller = Math.min(A.rect.width * A.rect.height, B.rect.width * B.rect.height);
        if (area > 0 && smaller > 0 && area / smaller > 0.25) add(layout, 'fixed-overlap', A.position + ' ' + selectorOf(A.el) + ' overlaps ' + B.position + ' ' + selectorOf(B.el) + ' (' + Math.round((100 * area) / smaller) + '% of the smaller one)', A.el);
      }
    }

    var scrollWidth = Math.max(doc.documentElement.scrollWidth || 0, doc.body ? doc.body.scrollWidth || 0 : 0);
    if (vw > 0 && scrollWidth > vw + 1) add(layout, 'page-overflow-x', 'the page scrolls horizontally: content is ' + scrollWidth + 'px wide in a ' + vw + 'px viewport', null, { px: scrollWidth - vw });

    headingSkips(levels).forEach(function (s) {
      add(a11y, 'heading-order', 'heading level skips from h' + s.from + ' to h' + s.to, null);
    });
    var lang = (doc.documentElement.getAttribute('lang') || '').trim();
    if (!lang) add(a11y, 'html-lang', '<html> has no lang attribute', null);
    if (!hasH1 && visibleElements > 0) add(a11y, 'no-h1', 'the page has no visible h1 heading', null);

    var text = '';
    try { text = root.innerText; } catch (_) { text = ''; }
    if (typeof text !== 'string') text = plainText(root, styleOf);
    text = text.replace(/\s+/g, ' ').trim();
    var blank = visibleElements === 0 || (!text && mediaVisible === 0);

    var counts = opts.consoleCounts || w.__geniusCounts || {};
    return {
      url: clip(w.location.href, 300),
      title: clip(doc.title, 200),
      lang: lang || null,
      viewport: { width: Math.round(vw), height: Math.round(vh) },
      scrollWidth: Math.round(scrollWidth),
      scrollHeight: Math.round(Math.max(doc.documentElement.scrollHeight || 0, doc.body ? doc.body.scrollHeight || 0 : 0)),
      rendered: { root: root.id ? '#' + root.id : 'body', visibleElements: visibleElements, textLength: text.length, blank: blank },
      headings: headings,
      landmarks: landmarks,
      text: clip(text, LIMITS.text),
      layout: layout,
      a11y: a11y,
      console: { errors: counts.errors | 0, warnings: counts.warnings | 0 },
      truncated: truncated,
    };
  }

  var api = { parseColor: parseColor, luminance: luminance, contrastRatio: contrastRatio, blend: blend, effectiveBackground: effectiveBackground, isLargeText: isLargeText, checkContrast: checkContrast, horizontalOverflow: horizontalOverflow, intersectionArea: intersectionArea, headingSkips: headingSkips, accessibleName: accessibleName, inspectPage: inspectPage, LIMITS: LIMITS };

  // ---------------------------------------------------------------- request listener
  if (install && win.parent && win.parent !== win && !win.__geniusPageInspector) {
    win.__geniusPageInspector = true;
    var nextFrame = function () {
      return new Promise(function (resolve) {
        var done = false;
        var f = function () { if (!done) { done = true; resolve(); } };
        try { win.requestAnimationFrame(function () { win.requestAnimationFrame(f); }); } catch (_) { /* no rAF */ }
        win.setTimeout(f, 120);
      });
    };
    // Only the embedding Genius window may ask (same check as the element picker).
    win.addEventListener('message', function (e) {
      var d = e.data;
      if (e.source !== win.parent || !d || d.type !== 'genius:inspect') return;
      var id = String(d.requestId || '').slice(0, 100);
      var wait = Math.max(0, Math.min(3000, +d.wait || 0));
      nextFrame()
        .then(function () { return new Promise(function (r) { win.setTimeout(r, wait); }); })
        .then(nextFrame)
        .then(function () {
          var msg;
          try { msg = { type: 'genius:inspection', requestId: id, report: inspectPage(win) }; } catch (err) { msg = { type: 'genius:inspection', requestId: id, error: clip(err && err.message ? err.message : err, 300) }; }
          try { win.parent.postMessage(msg, '*'); } catch (_) { /* ignore */ }
        });
    });
  }
  return api;
}
