//! SSRF matrix for web.rs against a local server (loopback allowed only through the test
//! policy; every other private range stays blocked). Redirect hops are re-checked.

use genius_core::web::{fetch, fetch_with, WebPolicy, MAX_BODY_BYTES};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn server() -> MockServer {
    let s = MockServer::start().await;
    let html = "<html><head><title>Docs</title></head><body><p>Hello docs</p></body></html>";
    Mock::given(method("GET")).and(path("/ok")).respond_with(ResponseTemplate::new(200).set_body_raw(html, "text/html")).mount(&s).await;
    Mock::given(method("GET")).and(path("/big")).respond_with(ResponseTemplate::new(200).set_body_raw("a".repeat(3 * 1024 * 1024), "text/plain")).mount(&s).await;
    Mock::given(method("GET")).and(path("/image")).respond_with(ResponseTemplate::new(200).set_body_raw(vec![0u8; 16], "image/png")).mount(&s).await;
    Mock::given(method("GET")).and(path("/loop")).respond_with(ResponseTemplate::new(302).insert_header("Location", "/loop")).mount(&s).await;
    Mock::given(method("GET")).and(path("/hop1")).respond_with(ResponseTemplate::new(302).insert_header("Location", "/hop2")).mount(&s).await;
    Mock::given(method("GET")).and(path("/hop2")).respond_with(ResponseTemplate::new(301).insert_header("Location", "/ok#frag")).mount(&s).await;
    Mock::given(method("GET")).and(path("/nolocation")).respond_with(ResponseTemplate::new(302)).mount(&s).await;
    for (name, target) in [
        ("meta", "http://169.254.169.254/latest/meta-data/"),
        ("private10", "http://10.0.0.5/admin"),
        ("private192", "http://192.168.1.1/"),
        ("private172", "http://172.16.0.9/"),
        ("cgnat", "http://100.64.0.1/"),
        ("v6meta", "http://[fd00:ec2::254]/latest"),
        ("v4mapped", "http://[::ffff:169.254.169.254]/"),
        ("v4compat", "http://[::7f00:1]/"),
        ("v6link", "http://[fe80::1]/"),
        ("sixtofour", "http://[2002:a9fe:a9fe::1]/"),
        ("localhost", "http://localhost:1/"),
        ("localhostdot", "http://localhost.:1/"),
        ("zero", "http://0.0.0.0/"),
        ("file", "file:///etc/passwd"),
        ("ftp", "ftp://example.com/x"),
        ("gopher", "gopher://example.com/"),
        ("creds", "http://user:pw@example.com/"),
        ("redisport", "http://example.com:6379/"),
        ("internal", "http://metadata.google.internal/computeMetadata/v1/"),
        ("decimal", "http://2852039166/"),
        ("hex", "http://0xA9FEA9FE/"),
        ("octal", "http://0251.0376.0251.0376/"),
    ] {
        Mock::given(method("GET")).and(path(format!("/redir/{name}"))).respond_with(ResponseTemplate::new(302).insert_header("Location", target)).mount(&s).await;
    }
    s
}

fn u(s: &MockServer, p: &str) -> String {
    format!("http://127.0.0.1:{}{p}", s.address().port())
}

#[tokio::test]
async fn production_policy_refuses_the_loopback_server_itself() {
    let s = server().await;
    let e = fetch(&u(&s, "/ok")).await.unwrap_err().to_string();
    assert!(e.contains("private or local"), "{e}");
}

#[tokio::test]
async fn local_server_works_under_the_test_policy_and_follows_safe_redirects() {
    let s = server().await;
    let t = WebPolicy::allow_loopback_for_tests();
    let p = fetch_with(&u(&s, "/ok"), t).await.unwrap();
    assert_eq!((p.status, p.title.as_deref()), (200, Some("Docs")));
    assert!(p.text.contains("Hello docs"));
    let p = fetch_with(&u(&s, "/hop1"), t).await.unwrap();
    assert!(p.url.ends_with("/ok#frag") || p.url.ends_with("/ok"), "{}", p.url);
    assert_eq!(p.status, 200);
}

#[tokio::test]
async fn every_redirect_hop_is_rechecked() {
    let s = server().await;
    let t = WebPolicy::allow_loopback_for_tests();
    for name in [
        "meta", "private10", "private192", "private172", "cgnat", "v6meta", "v4mapped", "v4compat", "v6link", "sixtofour", "localhost", "localhostdot", "zero", "file", "ftp", "gopher", "creds", "redisport", "internal", "decimal", "hex", "octal",
    ] {
        let r = fetch_with(&u(&s, &format!("/redir/{name}")), t).await;
        assert!(r.is_err(), "redirect to {name} must be refused, got {:?}", r.map(|p| p.url));
        let e = r.unwrap_err().to_string();
        assert!(!e.contains("Hello docs"), "{e}");
    }
}

#[tokio::test]
async fn redirect_loops_missing_locations_binary_and_huge_bodies_are_bounded() {
    let s = server().await;
    let t = WebPolicy::allow_loopback_for_tests();
    assert!(fetch_with(&u(&s, "/loop"), t).await.unwrap_err().to_string().contains("too many redirects"));
    assert!(fetch_with(&u(&s, "/nolocation"), t).await.unwrap_err().to_string().contains("without a location"));
    assert!(fetch_with(&u(&s, "/image"), t).await.unwrap_err().to_string().contains("not a text page"));
    let big = fetch_with(&u(&s, "/big"), t).await.unwrap();
    assert!(big.truncated);
    assert!(big.text.len() <= MAX_BODY_BYTES, "{}", big.text.len());
}

#[tokio::test]
async fn direct_unsafe_urls_are_refused_before_any_connection() {
    let t = WebPolicy::allow_loopback_for_tests();
    for url in [
        "http://169.254.169.254/latest/meta-data", "http://[fd00:ec2::254]/", "http://10.1.2.3/", "http://localhost/", "http://127.0.0.1:6379/", "file:///etc/passwd", "data:text/plain,hi", "javascript:alert(1)", "http://2130706433:8080/x", "http://0.0.0.0:80/",
    ] {
        assert!(fetch_with(url, t).await.is_err(), "{url}");
        assert!(fetch(url).await.is_err(), "{url}");
    }
}
