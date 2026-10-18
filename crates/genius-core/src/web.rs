//! Internet access for the agent: a keyless web search (DuckDuckGo's HTML endpoint) and a
//! guarded page fetcher.
//!
//! * **SSRF protection.** Only http(s) URLs are fetched. The host is resolved by Genius and
//!   every resolved address must be public (no loopback, private, link-local, CGNAT,
//!   unspecified, multicast or documentation ranges); the connection is then pinned to the
//!   checked address so DNS cannot change between the check and the request. Redirects are
//!   followed manually (at most 5), re-checking every hop.
//! * **Bounds.** 20 s timeout, 2 MB body, text clipped for the model.
//! * **More SSRF cases covered** (see the tests): IPv4-compatible / 6to4 / Teredo / NAT64 IPv6
//!   forms, cloud metadata addresses, numeric host spellings (decimal, hex, octal), a block
//!   list of well-known service ports, and `localhost.`-style trailing dots. A resolution that
//!   contains *any* forbidden address is refused, so a mixed DNS answer cannot smuggle one in.
//!   Note: when the user's environment forces an HTTP(S) proxy (`HTTPS_PROXY`), the proxy
//!   resolves and connects, so address pinning cannot apply to that hop (the pre-check still does).
//! * **Untrusted.** Everything returned is data from the internet; callers label it so.
//!
//! The parsers (`parse_ddg_results`, `html_to_text`) are pure functions with tests.

use crate::error::{GeniusError, Result};
use regex::Regex;
use reqwest::Url;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
pub const MAX_RESULTS: usize = 8;
/// Largest body kept from one fetch (also the bound tests assert).
pub const MAX_BODY_BYTES: usize = MAX_BODY;

/// Ports of well-known non-web services; fetching them from a public host is never useful
/// for reading documentation and is a classic SSRF probe.
const BLOCKED_PORTS: &[u16] = &[22, 23, 25, 110, 135, 139, 143, 445, 465, 587, 993, 995, 1433, 1521, 2375, 2376, 3306, 3389, 5432, 5900, 6379, 9200, 11211, 27017];

/// What an outbound fetch may reach. The default (production) policy allows public internet
/// addresses only; the loopback exception exists so tests can serve pages from a local mock
/// server. It is only reachable from Rust code (never from the UI or the model), and
/// everything else — private, link-local, metadata, `localhost` by name — stays blocked.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WebPolicy {
    allow_loopback_ips: bool,
}

impl WebPolicy {
    /// Tests only: also allow `127.0.0.0/8` / `::1` IP addresses (never the name `localhost`).
    pub fn allow_loopback_for_tests() -> Self {
        Self { allow_loopback_ips: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    /// Final URL after redirects.
    pub url: String,
    pub status: u16,
    pub content_type: String,
    pub title: Option<String>,
    pub text: String,
    /// The body exceeded the size limit and was cut.
    pub truncated: bool,
}

// ----------------------------------------------------------------------------- search

static RESULT_A: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<a\s([^>]*class="[^"]*\bresult__a\b[^"]*"[^>]*)>(.*?)</a>"#).unwrap());
static SNIPPET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?is)<(a|div|td|span)\s[^>]*class="[^"]*\bresult__snippet\b[^"]*"[^>]*>(.*?)</(?:a|div|td|span)>"#).unwrap());
static HREF: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"(?i)\bhref\s*=\s*"([^"]*)""#).unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());

/// DuckDuckGo result links are redirects (`//duckduckgo.com/l/?uddg=<encoded>&rut=…`);
/// return the real target. Other links are returned absolute.
pub fn decode_ddg_href(href: &str) -> String {
    let href = decode_entities(href.trim());
    let abs = if href.starts_with("//") { format!("https:{href}") } else if href.starts_with('/') { format!("https://duckduckgo.com{href}") } else { href.clone() };
    match Url::parse(&abs) {
        Ok(u) if u.host_str().is_some_and(|h| h.ends_with("duckduckgo.com")) && u.path().starts_with("/l/") => {
            u.query_pairs().find(|(k, _)| k == "uddg").map(|(_, v)| v.into_owned()).unwrap_or(abs)
        }
        _ => abs,
    }
}

fn inline_text(html: &str) -> String {
    let t = decode_entities(&TAG.replace_all(html, ""));
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse DuckDuckGo's HTML results page. Ads (`y.js` links) are skipped.
pub fn parse_ddg_results(html: &str) -> Vec<SearchResult> {
    let titles: Vec<_> = RESULT_A.captures_iter(html).collect();
    let snippets: Vec<(usize, String)> = SNIPPET.captures_iter(html).map(|c| (c.get(0).unwrap().start(), inline_text(&c[2]))).collect();
    let mut out: Vec<SearchResult> = Vec::new();
    for (i, c) in titles.iter().enumerate() {
        let start = c.get(0).unwrap().start();
        let next = titles.get(i + 1).map(|n| n.get(0).unwrap().start()).unwrap_or(usize::MAX);
        let Some(href) = HREF.captures(&c[1]).map(|h| h[1].to_string()) else { continue };
        let url = decode_ddg_href(&href);
        if url.contains("duckduckgo.com/y.js") || !(url.starts_with("http://") || url.starts_with("https://")) || out.iter().any(|r| r.url == url) {
            continue;
        }
        let snippet = snippets.iter().find(|(p, _)| *p > start && *p < next).map(|(_, s)| s.clone()).unwrap_or_default();
        out.push(SearchResult { title: inline_text(&c[2]), url, snippet });
        if out.len() >= MAX_RESULTS {
            break;
        }
    }
    out
}

pub fn render_results(results: &[SearchResult]) -> String {
    results.iter().enumerate().map(|(i, r)| format!("{}. {} — {}\n{}\n", i + 1, r.title, r.url, r.snippet)).collect::<Vec<_>>().join("\n")
}

/// Search the web (no API key). Returns up to [`MAX_RESULTS`] results.
pub async fn search(query: &str) -> Result<Vec<SearchResult>> {
    let q = query.trim();
    if q.len() < 2 || q.len() > 400 {
        return Err(GeniusError::Validation("search query must be 2-400 characters".into()));
    }
    let url = Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", q)]).map_err(|e| GeniusError::Internal(e.to_string()))?;
    let page = fetch_raw(url, WebPolicy::default()).await?;
    if !(200..300).contains(&page.0) {
        return Err(GeniusError::Validation(format!("the search service answered HTTP {}; try again shortly", page.0)));
    }
    Ok(parse_ddg_results(&String::from_utf8_lossy(&page.3)))
}

// ----------------------------------------------------------------------------- fetch

/// True for addresses an outbound fetch must never reach.
pub fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => forbidden_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return forbidden_v4(v4); // ::ffff:a.b.c.d
            }
            let s = v6.segments();
            if s[..6].iter().all(|x| *x == 0) {
                return true; // ::, ::1 and the deprecated IPv4-compatible ::a.b.c.d
            }
            if s[0] == 0x2002 {
                // 6to4 embeds an IPv4 address in the next 32 bits
                return forbidden_v4(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
            }
            v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7 (includes AWS fd00:ec2::254)
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x2001 && s[1] == 0) // Teredo (obfuscated IPv4)
                || (s[0] == 0x0064 && s[1] == 0xff9b) // NAT64 64:ff9b::/32 (embeds arbitrary IPv4)
                || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0) // discard-only 100::/64
        }
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() || ip.is_documentation()
        || o[0] == 0 // "this network"
        || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // benchmarking
        || o[0] >= 240 // reserved
}

/// What the URL itself says (no network): scheme, credentials, host name and port rules.
/// Returns the bare host (no brackets, no trailing dot) and the port.
fn check_url_shape(url: &Url) -> Result<(String, u16)> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(GeniusError::Validation(format!("only http and https URLs can be fetched (got {}:)", url.scheme())));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(GeniusError::Validation("URLs with embedded credentials are not fetched".into()));
    }
    let host = url.host_str().ok_or_else(|| GeniusError::Validation("the URL has no host".into()))?;
    let bare = host.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.');
    let lower = bare.to_ascii_lowercase();
    if lower.is_empty() {
        return Err(GeniusError::Validation("the URL has no host".into()));
    }
    if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") || lower.ends_with(".internal") || lower.ends_with(".localdomain") || lower.ends_with(".home.arpa") || lower == "metadata" {
        return Err(GeniusError::Validation(format!("{host} is a local address; only public internet addresses can be fetched")));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    if BLOCKED_PORTS.contains(&port) {
        return Err(GeniusError::Validation(format!("port {port} is reserved for non-web services and is not fetched")));
    }
    Ok((lower, port))
}

/// Pick the address to connect to from a resolution. **Every** address must be allowed: a DNS
/// answer that mixes a public and a private address is refused, so the connection can never be
/// steered to the private one. The chosen address is then pinned for the request.
pub fn choose_address(host: &str, addrs: &[SocketAddr], policy: WebPolicy) -> Result<SocketAddr> {
    if addrs.is_empty() {
        return Err(GeniusError::Validation(format!("{host} did not resolve to any address")));
    }
    let allowed = |ip: IpAddr| !is_forbidden_ip(ip) || (policy.allow_loopback_ips && ip.is_loopback());
    if let Some(bad) = addrs.iter().find(|a| !allowed(a.ip())) {
        return Err(GeniusError::Validation(format!("{host} resolves to a private or local address ({}); only public internet addresses can be fetched", bad.ip())));
    }
    Ok(addrs[0])
}

/// Validate a URL's scheme and host and resolve it to a public address.
async fn check_url(url: &Url, policy: WebPolicy) -> Result<SocketAddr> {
    let (bare, port) = check_url_shape(url)?;
    let addrs: Vec<SocketAddr> = match bare.parse::<IpAddr>() {
        Ok(ip) => vec![SocketAddr::new(ip, port)],
        Err(_) => tokio::time::timeout(Duration::from_secs(10), tokio::net::lookup_host((bare.as_str(), port)))
            .await
            .map_err(|_| GeniusError::Validation(format!("DNS lookup for {bare} timed out")))?
            .map_err(|e| GeniusError::Validation(format!("could not resolve {bare}: {e}")))?
            .collect(),
    };
    choose_address(&bare, &addrs, policy)
}

/// GET with manual, re-checked redirects. Returns (status, final url, content type, body, truncated).
async fn fetch_raw(start: Url, policy: WebPolicy) -> Result<(u16, Url, String, Vec<u8>, bool)> {
    let mut url = start;
    for _ in 0..=MAX_REDIRECTS {
        let addr = check_url(&url, policy).await?;
        let mut b = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).timeout(TIMEOUT).user_agent(USER_AGENT);
        if let Some(host) = url.host_str().filter(|h| h.parse::<IpAddr>().is_err() && !h.starts_with('[')) {
            b = b.resolve(host, addr); // pin the checked address (no DNS rebinding)
        }
        let client = b.build().map_err(|e| GeniusError::Internal(e.to_string()))?;
        let mut resp = client
            .get(url.clone())
            .header("Accept", "text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.5")
            .header("Accept-Language", "en-US,en;q=0.8")
            .send()
            .await
            .map_err(|e| GeniusError::Validation(format!("request to {} failed: {}", url.host_str().unwrap_or(""), if e.is_timeout() { "timed out".to_string() } else { e.to_string() })))?;
        let status = resp.status();
        if status.is_redirection() {
            let loc = resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).ok_or_else(|| GeniusError::Validation(format!("HTTP {status} redirect without a location")))?;
            url = url.join(loc).map_err(|e| GeniusError::Validation(format!("invalid redirect location: {e}")))?;
            continue;
        }
        let ct = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        let mut body = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = resp.chunk().await.map_err(|e| GeniusError::Validation(format!("reading the response failed: {e}")))? {
            body.extend_from_slice(&chunk);
            if body.len() > MAX_BODY {
                body.truncate(MAX_BODY);
                truncated = true;
                break;
            }
        }
        return Ok((status.as_u16(), url, ct, body, truncated));
    }
    Err(GeniusError::Validation(format!("too many redirects (more than {MAX_REDIRECTS})")))
}

/// Fetch a public web page and convert it to readable text.
pub async fn fetch(raw_url: &str) -> Result<Page> {
    fetch_with(raw_url, WebPolicy::default()).await
}

/// [`fetch`] with an explicit policy (tests use it to serve pages from a local mock server).
pub async fn fetch_with(raw_url: &str, policy: WebPolicy) -> Result<Page> {
    let url = Url::parse(raw_url.trim()).map_err(|e| GeniusError::Validation(format!("invalid URL: {e}")))?;
    let (status, url, ct, body, truncated) = fetch_raw(url, policy).await?;
    if ct.starts_with("image/") || ct.starts_with("video/") || ct.starts_with("audio/") || ct.contains("octet-stream") || ct.contains("application/pdf") || ct.contains("zip") {
        return Err(GeniusError::Validation(format!("{url} is not a text page (content type {ct})")));
    }
    let raw = String::from_utf8_lossy(&body).into_owned();
    let looks_html = ct.contains("html") || (ct.is_empty() && raw.trim_start().get(..15).is_some_and(|s| s.to_ascii_lowercase().starts_with("<!doctype html") || s.to_ascii_lowercase().starts_with("<html")));
    let (title, text) = if looks_html { html_to_text(&raw) } else { (None, raw) };
    Ok(Page { url: url.to_string(), status, content_type: ct, title, text, truncated })
}

// ----------------------------------------------------------------------------- html → text

static TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static DROP: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    ["script", "style", "noscript", "svg", "nav", "head", "template", "iframe", "form", "button", "select"].iter().map(|t| Regex::new(&format!(r"(?is)<{t}\b[^>]*>.*?</{t}\s*>")).unwrap()).collect()
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<h[1-6]\b[^>]*>").unwrap());
static LI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").unwrap());
static BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</?(br|p|div|h[1-6]|ul|ol|tr|table|section|article|header|footer|main|aside|blockquote|pre|hr|dd|dt|figure|figcaption)\b[^>]*>").unwrap());
static CELL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</t[dh]\s*>").unwrap());
static NUM_ENTITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&#([xX][0-9a-fA-F]{1,6}|[0-9]{1,7});").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}]+").unwrap());
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let s = NUM_ENTITY.replace_all(s, |c: &regex::Captures| {
        let n = &c[1];
        let code = if let Some(h) = n.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok() } else { n.parse().ok() };
        code.and_then(char::from_u32).map(String::from).unwrap_or_default()
    });
    let mut out = s.into_owned();
    for (e, r) in [
        ("&nbsp;", " "), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""), ("&apos;", "'"), ("&mdash;", "—"), ("&ndash;", "–"),
        ("&hellip;", "…"), ("&rsquo;", "’"), ("&lsquo;", "‘"), ("&rdquo;", "”"), ("&ldquo;", "“"), ("&copy;", "©"), ("&reg;", "®"),
        ("&trade;", "™"), ("&middot;", "·"), ("&bull;", "•"), ("&times;", "×"), ("&amp;", "&"),
    ] {
        out = out.replace(e, r);
    }
    out
}

/// Readable text from HTML: drops scripts/styles/navigation/svg, keeps headings, list items
/// and paragraphs on their own lines, decodes entities and collapses whitespace.
pub fn html_to_text(html: &str) -> (Option<String>, String) {
    let title = TITLE.captures(html).map(|c| inline_text(&c[1])).filter(|t| !t.is_empty());
    let mut s = COMMENT.replace_all(html, "").into_owned();
    for re in DROP.iter() {
        s = re.replace_all(&s, "\n").into_owned();
    }
    s = HEADING.replace_all(&s, "\n\n## ").into_owned();
    s = LI.replace_all(&s, "\n- ").into_owned();
    s = CELL.replace_all(&s, " | ").into_owned();
    s = BLOCK.replace_all(&s, "\n").into_owned();
    s = TAG.replace_all(&s, "").into_owned();
    s = decode_entities(&s);
    let lines: Vec<String> = s.lines().map(|l| SPACES.replace_all(l, " ").trim().to_string()).collect();
    let text = BLANKS.replace_all(&lines.join("\n"), "\n\n").trim().to_string();
    (title, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DDG: &str = r#"<html><body>
<div class="result results_links results_links_deep result--ad">
  <a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad_provider=x&amp;u3=1">Sponsored thing</a>
  <a class="result__snippet" href="https://duckduckgo.com/y.js?x">Buy now</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Freact.dev%2Freference%2Freact%2FuseState&amp;rut=abc">useState &ndash; <b>React</b></a>
  </h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x"><b>useState</b> is a React Hook that lets you add a state variable to your component.</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title"><a class="result__a" rel="nofollow" href="https://vite.dev/guide/">Getting Started | Vite</a></h2>
  <div class="result__snippet">Next Generation &amp; Frontend Tooling</div>
</div>
<div class="result"><h2><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fno-snippet">No snippet</a></h2></div>
</body></html>"#;

    #[test]
    fn parses_duckduckgo_results() {
        let r = parse_ddg_results(DDG);
        assert_eq!(r.len(), 3, "{r:?}");
        assert_eq!(r[0], SearchResult { title: "useState – React".into(), url: "https://react.dev/reference/react/useState".into(), snippet: "useState is a React Hook that lets you add a state variable to your component.".into() });
        assert_eq!(r[1].url, "https://vite.dev/guide/");
        assert_eq!(r[1].snippet, "Next Generation & Frontend Tooling");
        assert_eq!(r[2].url, "https://example.com/no-snippet");
        assert_eq!(r[2].snippet, "", "snippets are not borrowed from other results");
        assert!(render_results(&r).starts_with("1. useState – React — https://react.dev/reference/react/useState\n"));
        assert!(parse_ddg_results("<html>no results</html>").is_empty());
    }

    #[test]
    fn decodes_redirect_links() {
        assert_eq!(decode_ddg_href("//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.com%2Fp%3Fq%3D1%26r%3D2&amp;rut=z"), "https://a.com/p?q=1&r=2");
        assert_eq!(decode_ddg_href("https://b.org/x"), "https://b.org/x");
    }

    #[test]
    fn html_becomes_readable_text() {
        let html = r#"<!doctype html><html><head><title>Docs &amp; Guides</title><style>body{color:red}</style><script>alert("x<p>")</script></head>
<body><nav><a href="/">Home</a><a href="/about">About</a></nav><!-- hidden -->
<h1>Getting   started</h1><p>Install with <code>npm&nbsp;i</code>.<br>Then run it.</p>
<ul><li>Fast</li><li>Small &lt;1kB&gt;</li></ul><svg><text>icon</text></svg>
<table><tr><td>a</td><td>b</td></tr></table><p>&#169; 2026 &#x2014; done</p></body></html>"#;
        let (title, text) = html_to_text(html);
        assert_eq!(title.as_deref(), Some("Docs & Guides"));
        assert!(text.contains("## Getting started"), "{text}");
        assert!(text.contains("Install with npm i.\nThen run it."), "{text}");
        assert!(text.contains("- Fast\n- Small <1kB>"), "{text}");
        assert!(text.contains("a | b |"), "{text}");
        assert!(text.contains("© 2026 — done"), "{text}");
        for gone in ["alert", "color:red", "Home", "About", "icon", "hidden"] {
            assert!(!text.contains(gone), "{gone} should be stripped: {text}");
        }
        assert!(!text.contains("\n\n\n"));
    }

    #[test]
    fn private_and_local_addresses_are_forbidden() {
        for ip in ["127.0.0.1", "10.1.2.3", "172.16.0.1", "192.168.1.1", "169.254.169.254", "0.0.0.0", "100.64.0.1", "224.0.0.1", "::1", "::", "fe80::1", "fd00::1", "::ffff:127.0.0.1", "::ffff:10.0.0.1"] {
            assert!(is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["93.184.216.34", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(!is_forbidden_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn ipv6_and_cloud_metadata_variants_are_forbidden() {
        for ip in [
            "::127.0.0.1", "::7f00:1", "::a00:1", "::ffff:169.254.169.254", "::ffff:a9fe:a9fe", "2002:7f00:1::1", "2002:a9fe:a9fe::", "2002:c0a8:101::", "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "64:ff9b::7f00:1", "64:ff9b::a9fe:a9fe", "fd00:ec2::254", "fe80::a9fe:a9fe", "fec0::1", "ff02::1", "2001:db8::1", "100::1", "100.100.100.200", "169.254.170.2", "192.0.0.192", "198.18.0.1", "240.0.0.1", "255.255.255.255", "172.31.255.255",
        ] {
            assert!(is_forbidden_ip(ip.parse().unwrap()), "{ip} must be forbidden");
        }
        for ip in ["8.8.8.8", "93.184.216.34", "2606:4700:4700::1111", "2a00:1450:4001:81b::200e", "2002:808:808::"] {
            assert!(!is_forbidden_ip(ip.parse().unwrap()), "{ip} must be allowed");
        }
    }

    fn shape(u: &str) -> Result<(String, u16)> {
        check_url_shape(&Url::parse(u).unwrap())
    }

    #[test]
    fn url_shape_rules_cover_schemes_hosts_ports_and_numeric_spellings() {
        for u in [
            "file:///etc/passwd", "ftp://example.com/", "gopher://example.com/", "data:text/html,hi", "javascript:alert(1)", "ws://example.com/", "http://user@example.com/", "http://user:pw@example.com/",
            "http://localhost/", "http://LOCALHOST:8080/", "http://localhost./", "http://app.localhost/", "http://router.local/", "http://metadata.google.internal/computeMetadata/v1/", "http://metadata.google.internal./",
            "http://example.com:22/", "http://example.com:6379/", "https://example.com:3306/",
        ] {
            assert!(shape(u).is_err(), "{u} must be rejected by shape rules");
        }
        for u in ["https://react.dev/reference/react", "http://example.com:8080/x", "https://docs.rs:443/"] {
            assert!(shape(u).is_ok(), "{u}");
        }
        // Numeric spellings are normalised by the URL parser to dotted quads, which are then forbidden.
        for (u, canon) in [("http://2130706433/", "127.0.0.1"), ("http://0x7f.1/", "127.0.0.1"), ("http://0177.0.0.1/", "127.0.0.1"), ("http://127.1/", "127.0.0.1"), ("http://0xA9FEA9FE/", "169.254.169.254"), ("http://2852039166/", "169.254.169.254"), ("http://[::ffff:7f00:1]/", "[::ffff:7f00:1]")] {
            let url = Url::parse(u).unwrap();
            let host = url.host_str().unwrap();
            assert_eq!(host, canon, "{u}");
            let (bare, _) = check_url_shape(&url).unwrap();
            assert!(is_forbidden_ip(bare.parse().unwrap()), "{u} -> {bare}");
        }
    }

    #[test]
    fn a_mixed_dns_answer_is_refused_and_the_checked_address_is_the_one_returned() {
        let sa = |s: &str| -> SocketAddr { s.parse().unwrap() };
        let policy = WebPolicy::default();
        // DNS rebinding style answer: a public address next to a private one is refused outright.
        assert!(choose_address("evil.example", &[sa("93.184.216.34:443"), sa("127.0.0.1:443")], policy).is_err());
        assert!(choose_address("evil.example", &[sa("127.0.0.1:443"), sa("93.184.216.34:443")], policy).is_err());
        assert!(choose_address("evil.example", &[sa("[::1]:443")], policy).is_err());
        assert!(choose_address("evil.example", &[], policy).is_err());
        // The returned address is the one that gets pinned for the connection.
        assert_eq!(choose_address("ok.example", &[sa("93.184.216.34:443"), sa("1.1.1.1:443")], policy).unwrap(), sa("93.184.216.34:443"));
        // The test-only loopback exception never opens private, link-local or metadata ranges.
        let t = WebPolicy::allow_loopback_for_tests();
        assert!(choose_address("x", &[sa("127.0.0.1:80")], t).is_ok());
        for bad in ["169.254.169.254:80", "10.0.0.1:80", "192.168.1.1:80", "172.16.0.1:80", "[fd00:ec2::254]:80", "[::ffff:169.254.169.254]:80"] {
            assert!(choose_address("x", &[sa(bad)], t).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn fetch_rejects_unsafe_urls_without_network() {
        for u in ["file:///etc/passwd", "ftp://example.com/x", "http://localhost:3000/", "http://127.0.0.1/", "http://[::1]/", "http://169.254.169.254/latest/meta-data", "http://user:pw@example.com/", "http://printer.local/"] {
            assert!(fetch(u).await.is_err(), "{u} must be rejected");
        }
    }
}
