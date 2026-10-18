//! The updater against the real GitHub release feed, plus process-level checks of its failure
//! behaviour.
//!
//! * `live_update_flow` talks to the real GitHub Releases of this repository. It only runs when
//!   `GENIUS_LIVE_UPDATER=1` (otherwise it prints why it was skipped), because it needs network
//!   access and downloads a real installer. It is the Rust half of the Windows updater
//!   verification (`.github/workflows/updater-windows.yml`) and also runs on Linux (AppImage).
//! * The remaining tests need no network and always run.
//!
//! Environment for the live test:
//! * `GENIUS_LIVE_UPDATER=1`         enable
//! * `GENIUS_LIVE_OLD_VERSION`       the version the "running app" claims to be (default `0.0.1`)
//! * `GENIUS_LIVE_NEW_TAG`           check this release tag instead of `latest`
//! * `GENIUS_LIVE_INSTALL=1`         Windows: also run the downloaded installer silently
//! * `GENIUS_LIVE_INSTALL_DIR`       Windows: install directory handed to the installer (`/D=`)
//! * `GENIUS_LIVE_RESULT`            file to write a JSON summary to (for the workflow)
//!
//! What this proves and what it does not is written down in `docs/RELEASING.md`.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::Duration;

use genius_core::updater::{self, Progress, UpdateAsset};
use sha2::{Digest, Sha256};

const API: &str = "https://api.github.com";

fn say(msg: impl AsRef<str>) {
    println!("[live-updater] {}", msg.as_ref());
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn is_clean(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut d| d.next().is_none()).unwrap_or(true)
}

/// GET a published file independently of the updater code (so the checksum comparison below is
/// not circular).
async fn fetch_text(url: &str, api_url: Option<&str>, token: Option<&str>) -> String {
    let client = reqwest::Client::builder().user_agent("genius-updater-live-test").build().unwrap();
    let req = match (api_url, token) {
        (Some(api), Some(t)) => client.get(api).header("Accept", "application/octet-stream").bearer_auth(t),
        _ => client.get(url),
    };
    let resp = req.send().await.expect("fetching the published checksum");
    assert!(resp.status().is_success(), "checksum fetch returned HTTP {}", resp.status());
    resp.text().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_update_flow() {
    if std::env::var("GENIUS_LIVE_UPDATER").as_deref() != Ok("1") {
        eprintln!("skipped: set GENIUS_LIVE_UPDATER=1 to run the updater against the real GitHub releases");
        return;
    }
    let old = std::env::var("GENIUS_LIVE_OLD_VERSION").unwrap_or_else(|_| "0.0.1".into());
    let new_tag = std::env::var("GENIUS_LIVE_NEW_TAG").ok().filter(|t| !t.trim().is_empty());
    let token = updater::auth_token().await;
    let work = tempfile::tempdir().unwrap();

    // Linux: pretend to run from an AppImage so the AppImage asset is chosen and installed.
    let fake_appimage = work.path().join("Genius.AppImage");
    if cfg!(target_os = "linux") {
        std::fs::write(&fake_appimage, b"#!/bin/sh\necho the old version\n").unwrap();
        std::env::set_var("APPIMAGE", &fake_appimage);
    }

    // ---- check: the old version sees the newer release and picks the right installer
    let info = match &new_tag {
        Some(tag) => updater::check_tag_with(API, tag, &old, token.as_deref()).await,
        None => updater::check_with(API, &old, token.as_deref()).await,
    }
    .expect("the release check must succeed");
    say(format!("check from {old}: latest = {:?}, available = {}", info.latest_version, info.available));
    assert!(info.available, "version {old} must see the release as an update: {info:?}");
    let latest = info.latest_version.clone().expect("latest version");
    if let Some(tag) = &new_tag {
        assert_eq!(latest, updater::normalize_version(tag), "the release the workflow asked for was checked");
    }
    assert!(!info.notes.trim().is_empty() || info.release_url.is_some(), "release notes or page present");
    let asset = info.asset.clone().expect("the release has an installer for this platform");
    let (suffix, label) = match std::env::consts::OS {
        "windows" => ("-setup.exe", "NSIS setup"),
        "linux" => (".appimage", "AppImage"),
        "macos" => (".dmg", "disk image"),
        os => panic!("unsupported OS {os}"),
    };
    assert!(asset.name.to_lowercase().ends_with(suffix), "expected the {label} ({suffix}), got {}", asset.name);
    assert!(asset.sha256_url.is_some(), "the release must publish {}.sha256", asset.name);
    assert!(asset.size > 100_000, "an installer is not {} bytes", asset.size);
    say(format!("selected {} ({} bytes)", asset.name, asset.size));

    // ---- download + verify against the published checksum
    let stages = std::sync::Mutex::new(Vec::new());
    let path = updater::download_with(&asset, token.as_deref(), &|p: Progress| stages.lock().unwrap().push(p.stage)).await.expect("download and verification");
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len() as u64, asset.size, "downloaded size equals the size in the release");
    let published = fetch_text(asset.sha256_url.as_deref().unwrap(), asset.sha256_api_url.as_deref(), token.as_deref()).await;
    let published = published.split_whitespace().next().unwrap().to_ascii_lowercase();
    assert_eq!(hex(&Sha256::digest(&bytes)), published, "independent SHA-256 of the file equals the published one");
    assert!(stages.lock().unwrap().contains(&updater::Stage::Verifying), "a verification stage was reported");
    assert!(!path.with_file_name(format!("{}.part", asset.name)).exists(), "no .part file after success");
    say(format!("downloaded {} and verified SHA-256 {published}", path.display()));

    // ---- a tampered copy fails verification
    let tampered = work.path().join("tampered.bin");
    let mut flipped = bytes.clone();
    flipped[bytes.len() / 2] ^= 0x01;
    std::fs::write(&tampered, &flipped).unwrap();
    assert!(updater::verify_file_sha256(&tampered, &published).is_err(), "a single flipped bit must fail");
    updater::verify_file_sha256(&path, &published).expect("the genuine file passes");
    say("tampered copy rejected");

    // ---- substituted content fails the real download path and leaves nothing behind
    // (the checksum file itself is served as if it were the installer: a different file than
    // the one the checksum describes)
    let scratch = tempfile::tempdir().unwrap();
    let substituted = UpdateAsset { name: asset.name.clone(), size: 0, url: asset.sha256_url.clone().unwrap(), api_url: None, ..asset.clone() };
    let e = updater::download_into(scratch.path(), &substituted, None, &|_| {}).await.expect_err("substituted content must not verify").to_string();
    assert!(e.contains("checksum"), "{e}");
    assert!(is_clean(scratch.path()), "nothing remains after a failed verification: {:?}", std::fs::read_dir(scratch.path()).unwrap().flatten().collect::<Vec<_>>());
    say(format!("checksum mismatch discarded: {e}"));

    // ---- truncated / over-long downloads (the release says another size) fail
    for wrong in [asset.size + 1, asset.size - 1] {
        let wrong_size = UpdateAsset { size: wrong, ..asset.clone() };
        let e = updater::download_into(scratch.path(), &wrong_size, token.as_deref(), &|_| {}).await.expect_err("a size mismatch must be rejected").to_string();
        assert!(e.contains("incomplete"), "{e}");
        assert!(is_clean(scratch.path()), "the partial file is removed");
    }
    say("size mismatch (truncated download) rejected");

    // ---- wrong hosts are refused before any request
    for url in ["https://evil.example.com/Genius_setup.exe", "https://github.com.evil.example.com/x.exe", "http://github.com/x.exe", "https://github.com:8443/x.exe"] {
        let foreign = UpdateAsset { url: url.into(), api_url: None, ..asset.clone() };
        assert!(updater::download_into(scratch.path(), &foreign, None, &|_| {}).await.is_err(), "{url} must be refused");
    }
    assert!(is_clean(scratch.path()));
    say("foreign download hosts refused");

    // ---- install
    let mut installer_exit: Option<i32> = None;
    if cfg!(target_os = "linux") {
        let outcome = updater::install(&path).expect("AppImage install");
        assert_eq!(outcome.action, updater::PostInstall::Restart);
        assert_eq!(hex(&Sha256::digest(std::fs::read(&fake_appimage).unwrap())), published, "the AppImage on disk is now the published file");
        assert_eq!(std::fs::read(updater::backup_path(&fake_appimage)).unwrap(), b"#!/bin/sh\necho the old version\n", "the previous AppImage is kept as .bak");
        assert!(!path.exists(), "the downloaded copy is cleaned up after the install");
        assert!(updater::rollback_appimage(&fake_appimage).unwrap());
        assert_eq!(std::fs::read(&fake_appimage).unwrap(), b"#!/bin/sh\necho the old version\n", "rollback restores the old file");
        say("AppImage replaced atomically, backup kept, rollback works");
    } else if cfg!(windows) && std::env::var("GENIUS_LIVE_INSTALL").as_deref() == Ok("1") {
        let dir = std::env::var("GENIUS_LIVE_INSTALL_DIR").ok().filter(|d| !d.is_empty());
        say(format!("running the installer silently (dir = {dir:?})"));
        let code = updater::install_silent(&path, dir.as_deref().map(Path::new), Duration::from_secs(600)).expect("silent install");
        say(format!("installer exit code {code}"));
        assert_eq!(code, 0, "the NSIS installer must exit with 0");
        installer_exit = Some(code);
    } else {
        say("install step not run on this platform/configuration (the installer is only launched interactively by the app)");
    }

    if let Ok(result) = std::env::var("GENIUS_LIVE_RESULT") {
        let summary = serde_json::json!({
            "oldVersion": old,
            "newVersion": latest,
            "asset": asset.name,
            "size": asset.size,
            "sha256": published,
            "downloadedPath": path.display().to_string(),
            "installerExitCode": installer_exit,
        });
        std::fs::write(result, serde_json::to_string_pretty(&summary).unwrap()).unwrap();
    }
}

// ---------------------------------------------------------------- offline checks (always run)

#[tokio::test]
async fn public_api_refuses_unsafe_downloads_and_installs() {
    let dir = tempfile::tempdir().unwrap();
    let asset = UpdateAsset {
        name: "Genius_9.9.9_x64-setup.exe".into(),
        size: 10,
        url: "https://evil.example.com/Genius_9.9.9_x64-setup.exe".into(),
        sha256_url: Some("https://github.com/o/r/releases/download/v9/x.sha256".into()),
        api_url: None,
        sha256_api_url: None,
    };
    assert!(updater::download_into(dir.path(), &asset, None, &|_| {}).await.is_err());
    // A release without a published checksum is refused (nothing to verify the file with).
    let no_sum = UpdateAsset { url: "https://github.com/o/r/releases/download/v9/x.exe".into(), sha256_url: None, ..asset.clone() };
    let e = updater::download_into(dir.path(), &no_sum, None, &|_| {}).await.unwrap_err().to_string();
    assert!(e.contains("checksum"), "{e}");
    assert!(is_clean(dir.path()));

    // install() only runs files from the update download folder.
    let stray = dir.path().join("Genius_9.9.9_x64-setup.exe");
    std::fs::write(&stray, b"MZ not an installer").unwrap();
    assert!(updater::install(&stray).is_err());
    assert!(updater::install(Path::new("/etc/hosts")).is_err());
    assert!(updater::install(Path::new(r"C:\Windows\System32\cmd.exe")).is_err());
    assert!(updater::install(&dir.path().join("..").join("x.exe")).is_err());
    assert!(stray.exists(), "refusal leaves files alone");
    // Silent installation is a test-only code path and refuses the same files.
    assert!(updater::install_silent(&stray, None, Duration::from_secs(1)).is_err());
}

/// The updater's HTTP client must honour the standard proxy variables (corporate networks).
/// reqwest reads them once per process, so the parent re-runs this test binary as a child with
/// `HTTPS_PROXY` pointing at a local fake proxy and checks that the child asked it to CONNECT
/// to api.github.com.
#[test]
fn https_proxy_variable_is_honoured() {
    if std::env::var("GENIUS_PROXY_CHILD").as_deref() == Ok("1") {
        return; // this is the child process: the real work is in `proxy_child_process`
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).unwrap_or(0);
            let first_line = String::from_utf8_lossy(&buf[..n]).lines().next().unwrap_or_default().to_string();
            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            let _ = tx.send(first_line);
        }
    });
    let url = format!("http://127.0.0.1:{port}");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "proxy_child_process", "--nocapture", "--test-threads=1"])
        .env("GENIUS_PROXY_CHILD", "1")
        .env("HTTPS_PROXY", &url)
        .env("https_proxy", &url)
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GENIUS_UPDATE_TOKEN")
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        assert!(started.elapsed() < Duration::from_secs(90), "the proxied update check hung");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "the child test failed");
    let line = rx.recv_timeout(Duration::from_secs(5)).expect("the update check must go through HTTPS_PROXY");
    assert!(line.starts_with("CONNECT api.github.com:443"), "unexpected first proxy line: {line}");
}

#[tokio::test]
async fn proxy_child_process() {
    if std::env::var("GENIUS_PROXY_CHILD").as_deref() != Ok("1") {
        return;
    }
    // The proxy answers 502, so the check must fail cleanly (not hang, not panic).
    let r = updater::check_with(API, "0.0.1", None).await;
    assert!(r.is_err(), "the fake proxy refuses the tunnel: {r:?}");
}
