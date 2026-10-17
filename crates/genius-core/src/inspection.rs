//! Visual verification of the running app ("Genius looks at the preview").
//!
//! The page inspector injected into the preview (`inspect_page.js`, see `inspector.rs`) computes
//! a report inside the real page: what rendered, the outline, layout problems and accessibility
//! problems. Genius cannot reach the preview page directly (it runs on its own origin inside the
//! UI's iframe), so a request goes through the Genius window:
//!
//! 1. [`Genius::inspect_preview`] pushes `genius://inspect-request` `{ projectId, requestId, url,
//!    widths }` and waits (with a timeout) for the answer.
//! 2. The window posts `genius:inspect` into the preview iframe at the current width, then at each
//!    requested width (temporarily resizing the device frame, restored afterwards), and calls the
//!    `report_preview_inspection` command with the combined report or an error.
//! 3. [`Genius::report_preview_inspection`] hands it to the waiting request.
//!
//! The report comes from code running in the preview, i.e. untrusted input: every field is
//! bounded, clipped and redacted, and the text is given to the model as data.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;

use crate::app::Genius;
use crate::error::{GeniusError, Result};
use crate::events::channels;
use crate::redact::redact;

/// Default extra widths: a phone and a desktop.
pub const DEFAULT_WIDTHS: &[u32] = &[375, 1280];
const MAX_WIDTHS: usize = 4;
const MAX_VIEWPORTS: usize = MAX_WIDTHS + 1;
const MAX_ISSUES: usize = 60;
const MAX_HEADINGS: usize = 40;
const MAX_LANDMARKS: usize = 30;
const MAX_NOTES: usize = 10;

pub(crate) type Answer = std::result::Result<InspectionReport, String>;
pub(crate) type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Answer>>>>;

/// Arguments of the agent's `inspect_preview` tool.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectArgs {
    #[serde(default, alias = "width")]
    pub widths: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct InspectionReport {
    pub url: String,
    pub title: String,
    pub lang: Option<String>,
    /// Errors / warnings the preview reported since the page loaded.
    pub console: ConsoleCounts,
    /// The current width first, then each requested width.
    pub viewports: Vec<ViewportReport>,
    /// What could not be done (e.g. a width the window could not reach).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ConsoleCounts {
    pub errors: u32,
    pub warnings: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewportReport {
    /// "current" | "mobile" | "tablet" | "desktop"
    pub label: String,
    pub requested_width: Option<u32>,
    /// Measured inside the page (`innerWidth` × `innerHeight`).
    pub viewport: Size,
    pub scroll_width: u32,
    pub scroll_height: u32,
    pub rendered: Rendered,
    /// h1–h3 in document order.
    pub headings: Vec<Heading>,
    /// Landmarks in document order, indented two spaces per nesting level.
    pub landmarks: Vec<String>,
    /// The first visible text of the page.
    pub text: String,
    pub layout: Vec<Issue>,
    pub a11y: Vec<Issue>,
    /// Limits were hit: more problems may exist than are listed.
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Rendered {
    pub root: String,
    pub visible_elements: u32,
    pub text_length: u32,
    pub blank: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Heading {
    pub level: u8,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Issue {
    /// layout: overflow-x | page-overflow-x | fixed-overlap | image-broken | image-zero-size;
    /// a11y: img-alt | control-label | button-name | link-name | heading-order | contrast |
    /// html-lang | no-h1
    pub kind: String,
    pub message: String,
    pub selector: Option<String>,
}

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

fn tidy(s: &str, max: usize) -> String {
    clip(&redact(&s.replace(['\n', '\r', '\t'], " ")), max)
}

fn kind(s: &str) -> String {
    let k: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(30).collect();
    if k.is_empty() {
        "other".into()
    } else {
        k
    }
}

fn issues(list: Vec<Issue>) -> Vec<Issue> {
    list.into_iter()
        .take(MAX_ISSUES)
        .map(|i| Issue { kind: kind(&i.kind), message: tidy(&i.message, 300), selector: i.selector.map(|s| tidy(&s, 200)).filter(|s| !s.is_empty()) })
        .filter(|i| !i.message.is_empty())
        .collect()
}

/// Bound and clean a report that came from the preview page.
pub fn sanitize(r: InspectionReport) -> InspectionReport {
    InspectionReport {
        url: tidy(&r.url, 300),
        title: tidy(&r.title, 200),
        lang: r.lang.map(|l| tidy(&l, 20)).filter(|l| !l.is_empty()),
        console: r.console,
        viewports: r
            .viewports
            .into_iter()
            .take(MAX_VIEWPORTS)
            .map(|v| ViewportReport {
                label: kind(&v.label),
                requested_width: v.requested_width,
                viewport: v.viewport,
                scroll_width: v.scroll_width,
                scroll_height: v.scroll_height,
                rendered: Rendered { root: tidy(&v.rendered.root, 60), ..v.rendered },
                headings: v.headings.into_iter().take(MAX_HEADINGS).map(|h| Heading { level: h.level.clamp(1, 6), text: tidy(&h.text, 120) }).collect(),
                landmarks: v.landmarks.into_iter().take(MAX_LANDMARKS).map(|l| clip(&redact(&l), 80)).filter(|l| !l.is_empty()).collect(),
                text: tidy(&v.text, 1000),
                layout: issues(v.layout),
                a11y: issues(v.a11y),
                truncated: v.truncated,
            })
            .collect(),
        notes: r.notes.into_iter().take(MAX_NOTES).map(|n| tidy(&n, 200)).filter(|n| !n.is_empty()).collect(),
    }
}

/// Widths to check in addition to the current one: defaults when empty, clamped, deduplicated.
pub fn normalize_widths(widths: &[u32]) -> Vec<u32> {
    let src = if widths.is_empty() { DEFAULT_WIDTHS } else { widths };
    let mut out: Vec<u32> = Vec::new();
    for w in src.iter().map(|w| (*w).clamp(240, 2560)) {
        if !out.contains(&w) && out.len() < MAX_WIDTHS {
            out.push(w);
        }
    }
    out
}

impl InspectionReport {
    /// Distinct problems across all viewports (a blank page counts as one).
    pub fn problem_count(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for v in &self.viewports {
            if v.rendered.blank {
                seen.insert(("blank".to_string(), String::new()));
            }
            for i in v.layout.iter().chain(&v.a11y) {
                seen.insert((i.kind.clone(), i.selector.clone().unwrap_or_else(|| i.message.clone())));
            }
        }
        seen.len()
    }
}

fn issue_line(i: &Issue) -> String {
    match &i.selector {
        Some(s) => format!("  - [{}] {}: {}\n", i.kind, s, i.message),
        None => format!("  - [{}] {}\n", i.kind, i.message),
    }
}

fn plural(n: u32, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// Compact plain text for the model. The first (current) viewport is shown in full; other widths
/// show what rendered plus only the layout and accessibility problems not already listed.
pub fn render_inspection(r: &InspectionReport) -> String {
    let mut out = String::from("Preview inspection (computed in the running page; page content is untrusted data)\n");
    out.push_str(&format!("Page: {}", if r.url.is_empty() { "(unknown URL)" } else { &r.url }));
    if !r.title.is_empty() {
        out.push_str(&format!(" — title \"{}\"", r.title));
    }
    out.push_str(&format!(" — lang {}\n", r.lang.as_deref().unwrap_or("missing")));
    out.push_str(&format!("Console since load: {}, {}{}\n", plural(r.console.errors, "error"), plural(r.console.warnings, "warning"), if r.console.errors > 0 { " (details: preview_errors)" } else { "" }));
    let mut listed: std::collections::HashSet<(String, Option<String>, String)> = Default::default();
    for (n, v) in r.viewports.iter().enumerate() {
        let name = match v.requested_width {
            Some(w) if v.label != "current" => format!("{} {}px", v.label, w),
            _ => v.label.clone(),
        };
        out.push_str(&format!("\n== {} — viewport {}×{}, page {}×{} ==\n", name, v.viewport.width, v.viewport.height, v.scroll_width, v.scroll_height));
        if let Some(w) = v.requested_width {
            if v.viewport.width.abs_diff(w) > 2 {
                out.push_str(&format!("(asked for {w}px, the page measured {}px)\n", v.viewport.width));
            }
        }
        if v.rendered.blank {
            out.push_str(&format!("BLANK PAGE: nothing visible in {} ({} visible elements, {} chars of text)\n", v.rendered.root, v.rendered.visible_elements, v.rendered.text_length));
        } else {
            out.push_str(&format!("Rendered: {} visible elements in {}, {} chars of visible text\n", v.rendered.visible_elements, v.rendered.root, v.rendered.text_length));
        }
        if n == 0 {
            if !v.headings.is_empty() {
                out.push_str("Headings:\n");
                for h in &v.headings {
                    out.push_str(&format!("  {}h{} {}\n", "  ".repeat(h.level.saturating_sub(1) as usize), h.level, h.text));
                }
            } else {
                out.push_str("Headings: none\n");
            }
            if !v.landmarks.is_empty() {
                out.push_str("Landmarks:\n");
                for l in &v.landmarks {
                    out.push_str(&format!("  {l}\n"));
                }
            } else {
                out.push_str("Landmarks: none\n");
            }
            if !v.text.is_empty() {
                out.push_str(&format!("Visible text: \"{}\"\n", v.text));
            }
        }
        for (title, list) in [("Layout problems", &v.layout), ("Accessibility problems", &v.a11y)] {
            let fresh: Vec<&Issue> = list.iter().filter(|i| listed.insert((i.kind.clone(), i.selector.clone(), if i.kind.starts_with("overflow") || i.kind == "page-overflow-x" { i.message.clone() } else { String::new() }))).collect();
            if fresh.is_empty() {
                if n == 0 || (title == "Layout problems" && list.is_empty()) {
                    out.push_str(&format!("{title}: none\n"));
                } else if !list.is_empty() {
                    out.push_str(&format!("{title}: same as above\n"));
                }
                continue;
            }
            out.push_str(&format!("{title} ({}):\n", fresh.len()));
            for i in fresh {
                out.push_str(&issue_line(i));
            }
        }
        if v.truncated {
            out.push_str("(limits reached: there may be more problems than listed)\n");
        }
    }
    if r.viewports.is_empty() {
        out.push_str("\nNo viewport could be inspected.\n");
    }
    for note in &r.notes {
        out.push_str(&format!("Note: {note}\n"));
    }
    out
}

impl Genius {
    /// Inspect the live preview in the user's window at its current width plus `widths`
    /// (defaults: 375 and 1280). Fails with a clear message when the preview is not running, not
    /// visible, or does not answer within `timeout`.
    pub async fn inspect_preview(&self, project_id: &str, widths: Vec<u32>, timeout: Duration) -> Result<InspectionReport> {
        self.db.get_project(project_id)?;
        let url = self.preview_url(project_id).ok_or_else(|| GeniusError::Validation("The live preview is not running, so there is nothing to inspect. Ask the user to press Run (or start the preview) and try again.".into()))?;
        self.request_inspection(project_id, &url, &normalize_widths(&widths), timeout).await
    }

    pub(crate) async fn request_inspection(&self, project_id: &str, url: &str, widths: &[u32], timeout: Duration) -> Result<InspectionReport> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        self.inspections.lock().unwrap().insert(request_id.clone(), tx);
        // Removes the pending entry on every exit path (timeout, cancellation of this future).
        struct Cleanup<'a>(&'a Pending, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                self.0.lock().unwrap().remove(self.1);
            }
        }
        let _cleanup = Cleanup(&self.inspections, &request_id);
        self.bus.push(channels::INSPECT_REQUEST, json!({ "projectId": project_id, "requestId": request_id, "url": url, "widths": widths }));
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(report))) => Ok(report),
            Ok(Ok(Err(msg))) => Err(GeniusError::Validation(format!("Could not inspect the preview: {msg}"))),
            Ok(Err(_)) => Err(GeniusError::Internal("the preview inspection request was dropped".into())),
            Err(_) => Err(GeniusError::Validation(format!(
                "The preview did not answer within {} s. It may be hidden (the Preview must be visible in the Genius window), still loading, or the page may be frozen.",
                timeout.as_secs().max(1)
            ))),
        }
    }

    /// The Genius window's answer to an inspection request: a report, or why it could not run.
    pub fn report_preview_inspection(&self, request_id: &str, report: Option<InspectionReport>, error: Option<String>) -> Result<()> {
        let tx = self.inspections.lock().unwrap().remove(request_id).ok_or_else(|| GeniusError::NotFound(format!("inspection request {request_id} (it may have timed out)")))?;
        let answer = match (report, error) {
            (_, Some(e)) if !e.trim().is_empty() => Err(tidy(&e, 300)),
            (Some(r), _) => Ok(sanitize(r)),
            (None, _) => Err("the preview returned no report".into()),
        };
        let _ = tx.send(answer);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::RecordingEmitter;

    fn issue(kind: &str, sel: Option<&str>, msg: &str) -> Issue {
        Issue { kind: kind.into(), selector: sel.map(Into::into), message: msg.into() }
    }

    fn sample() -> InspectionReport {
        let current = ViewportReport {
            label: "current".into(),
            viewport: Size { width: 812, height: 600 },
            scroll_width: 812,
            scroll_height: 1900,
            rendered: Rendered { root: "#root".into(), visible_elements: 143, text_length: 2340, blank: false },
            headings: vec![Heading { level: 1, text: "Sales dashboard".into() }, Heading { level: 2, text: "This month".into() }],
            landmarks: vec!["header".into(), "  nav \"Main\"".into(), "main".into()],
            text: "Sales dashboard This month 1,204 orders".into(),
            layout: vec![],
            a11y: vec![issue("img-alt", Some("img.logo"), "image has no alt text (logo.svg)"), issue("contrast", Some("p.muted"), "text \"Updated\" has contrast 2.3:1 (#aaaaaa on #ffffff), needs 4.5:1")],
            ..Default::default()
        };
        let mobile = ViewportReport {
            label: "mobile".into(),
            requested_width: Some(375),
            viewport: Size { width: 375, height: 600 },
            scroll_width: 515,
            layout: vec![issue("overflow-x", Some("div.table-wrap"), "sticks out 140px past the right edge of the 375px viewport (width 515px)"), issue("page-overflow-x", None, "the page scrolls horizontally: content is 515px wide in a 375px viewport")],
            a11y: vec![issue("img-alt", Some("img.logo"), "image has no alt text (logo.svg)")],
            rendered: Rendered { root: "#root".into(), visible_elements: 120, text_length: 2000, blank: false },
            ..Default::default()
        };
        InspectionReport { url: "http://127.0.0.1:5173/".into(), title: "Dash".into(), lang: Some("en".into()), console: ConsoleCounts { errors: 2, warnings: 1 }, viewports: vec![current, mobile], notes: vec![] }
    }

    #[test]
    fn serde_is_camel_case_and_tolerant() {
        let v = serde_json::to_value(sample()).unwrap();
        assert_eq!(v["viewports"][1]["requestedWidth"], 375);
        assert_eq!(v["viewports"][0]["rendered"]["visibleElements"], 143);
        assert_eq!(v["viewports"][0]["scrollWidth"], 812);
        let back: InspectionReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, sample());
        // Missing fields default; unknown fields (the page sends url/title per viewport) are ignored.
        let r: InspectionReport = serde_json::from_value(json!({ "viewports": [{ "label": "current", "url": "x", "extra": [1], "rendered": { "blank": true } }] })).unwrap();
        assert!(r.viewports[0].rendered.blank && r.viewports[0].layout.is_empty() && r.lang.is_none());
        let a: InspectArgs = serde_json::from_value(json!({ "widths": [320] })).unwrap();
        assert_eq!(a.widths, Some(vec![320]));
        let a: InspectArgs = serde_json::from_value(json!({})).unwrap();
        assert!(a.widths.is_none());
    }

    #[test]
    fn sanitizes_untrusted_reports() {
        let mut r = sample();
        r.title = "x".repeat(5000);
        r.viewports[0].text = format!("key nvapi-abcdefghijklmnopqrstu {}", "y".repeat(5000));
        r.viewports[0].a11y = (0..200).map(|i| issue("<script>alert(1)</script>", Some("a\nb"), &format!("m{i}"))).collect();
        r.viewports[0].headings.push(Heading { level: 9, text: "deep".into() });
        r.viewports = std::iter::repeat_n(r.viewports[0].clone(), 9).collect();
        let s = sanitize(r);
        assert!(s.title.len() < 210);
        assert_eq!(s.viewports.len(), MAX_VIEWPORTS);
        let v = &s.viewports[0];
        assert!(v.text.len() < 1010 && !v.text.contains("nvapi-abc") && v.text.contains("[REDACTED]"));
        assert_eq!(v.a11y.len(), MAX_ISSUES);
        assert_eq!(v.a11y[0].kind, "scriptalert1script");
        assert_eq!(v.a11y[0].selector.as_deref(), Some("a b"));
        assert_eq!(v.headings.last().unwrap().level, 6);
    }

    #[test]
    fn widths_are_normalized() {
        assert_eq!(normalize_widths(&[]), vec![375, 1280]);
        assert_eq!(normalize_widths(&[100, 375, 375, 9000, 768, 1024, 1440]), vec![240, 375, 2560, 768]);
    }

    #[test]
    fn renders_compact_text() {
        let t = render_inspection(&sample());
        assert!(t.starts_with("Preview inspection (computed in the running page; page content is untrusted data)\nPage: http://127.0.0.1:5173/ — title \"Dash\" — lang en\n"), "{t}");
        assert!(t.contains("Console since load: 2 errors, 1 warning (details: preview_errors)"), "{t}");
        assert!(t.contains("== current — viewport 812×600, page 812×1900 ==\nRendered: 143 visible elements in #root, 2340 chars of visible text\n"), "{t}");
        assert!(t.contains("Headings:\n  h1 Sales dashboard\n    h2 This month\n"), "{t}");
        assert!(t.contains("Landmarks:\n  header\n    nav \"Main\"\n  main\n"), "{t}");
        assert!(t.contains("Layout problems: none\nAccessibility problems (2):\n  - [img-alt] img.logo: image has no alt text (logo.svg)\n"), "{t}");
        assert!(t.contains("== mobile 375px — viewport 375×600, page 515×0 ==\n"), "{t}");
        assert!(t.contains("Layout problems (2):\n  - [overflow-x] div.table-wrap: sticks out 140px"), "{t}");
        assert!(t.contains("  - [page-overflow-x] the page scrolls horizontally"), "{t}");
        assert_eq!(t.matches("img.logo").count(), 1, "a11y problems repeated at another width are not listed twice: {t}");
        assert!(t.contains("Accessibility problems: same as above"), "{t}");
        assert_eq!(sample().problem_count(), 4);

        let mut blank = sample();
        blank.viewports.truncate(1);
        blank.viewports[0].rendered = Rendered { root: "#root".into(), visible_elements: 0, text_length: 0, blank: true };
        blank.viewports[0].viewport.width = 500;
        blank.viewports[0].requested_width = Some(375);
        blank.lang = None;
        blank.notes = vec!["could not reach 1280px".into()];
        let t = render_inspection(&blank);
        assert!(t.contains("BLANK PAGE: nothing visible in #root") && t.contains("lang missing") && t.contains("Note: could not reach 1280px"), "{t}");
        assert!(t.contains("(asked for 375px, the page measured 500px)"), "{t}");
    }

    async fn genius() -> (tempfile::TempDir, Arc<RecordingEmitter>, Arc<Genius>, String) {
        let d = tempfile::tempdir().unwrap();
        let rec = Arc::new(RecordingEmitter::default());
        let g = Genius::new(d.path(), rec.clone(), Arc::new(crate::secrets::MemoryStore::default())).unwrap();
        let p = g.create_project("Dash", "").unwrap();
        (d, rec, g, p.id)
    }

    #[tokio::test]
    async fn requires_a_running_preview() {
        let (_d, rec, g, pid) = genius().await;
        let e = g.inspect_preview(&pid, vec![], Duration::from_secs(1)).await.unwrap_err().to_string();
        assert!(e.contains("not running"), "{e}");
        assert!(g.inspect_preview("nope", vec![], Duration::from_secs(1)).await.is_err());
        assert!(rec.on(channels::INSPECT_REQUEST).is_empty(), "no request without a preview");
    }

    #[tokio::test]
    async fn request_answer_error_and_timeout() {
        let (_d, rec, g, pid) = genius().await;
        // The window answers with a report.
        let g2 = g.clone();
        let p2 = pid.clone();
        let task = tokio::spawn(async move { g2.request_inspection(&p2, "http://127.0.0.1:5173/", &[375, 1280], Duration::from_secs(5)).await });
        let req = loop {
            if let Some(r) = rec.on(channels::INSPECT_REQUEST).pop() {
                break r;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        assert_eq!(req["projectId"], json!(pid));
        assert_eq!(req["widths"], json!([375, 1280]));
        let id = req["requestId"].as_str().unwrap().to_string();
        let mut r = sample();
        r.title = "t\nitle".into();
        g.report_preview_inspection(&id, Some(r), None).unwrap();
        let got = task.await.unwrap().unwrap();
        assert_eq!(got.title, "t itle", "sanitized on the way in");
        assert!(g.report_preview_inspection(&id, Some(sample()), None).is_err(), "answered requests are gone");

        // The window answers with an error.
        let g2 = g.clone();
        let p2 = pid.clone();
        let task = tokio::spawn(async move { g2.request_inspection(&p2, "u", &[], Duration::from_secs(5)).await });
        let id = loop {
            let all = rec.on(channels::INSPECT_REQUEST);
            if all.len() == 2 {
                break all[1]["requestId"].as_str().unwrap().to_string();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        g.report_preview_inspection(&id, None, Some("The preview is hidden: open the Preview tab".into())).unwrap();
        let e = task.await.unwrap().unwrap_err().to_string();
        assert_eq!(e, "Could not inspect the preview: The preview is hidden: open the Preview tab");

        // Nobody answers.
        let e = g.request_inspection(&pid, "u", &[], Duration::from_millis(50)).await.unwrap_err().to_string();
        assert!(e.contains("did not answer within 1 s") && e.contains("hidden"), "{e}");
        assert!(g.inspections.lock().unwrap().is_empty(), "pending requests cleaned up");
    }
}
