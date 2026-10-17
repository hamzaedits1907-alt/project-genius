//! In-app updates based on GitHub Releases (no signing keys required).
//!
//! [`check`] asks the GitHub API for the latest release, compares versions and picks the
//! installer asset for the running platform. [`download_and_install`] streams that asset
//! into a temp directory, verifies its SHA-256 when the release publishes one, and launches
//! the platform installer. Downloads are restricted to GitHub-owned hosts.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::error::{GeniusError, Result};

pub const REPO: &str = "hamzaedits1907-alt/project-genius";
const API_BASE: &str = "https://api.github.com";
const USER_AGENT: &str = concat!("Genius-Updater/", env!("CARGO_PKG_VERSION"));

/// Hosts an update download (and every redirect it follows) may come from.
const ALLOWED_DOWNLOAD_HOSTS: &[&str] =
    &["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"];
/// Hosts allowed for the *initial* asset URL handed to [`download_and_install`].
const ALLOWED_ASSET_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAsset {
    pub name: String,
    pub size: u64,
    pub url: String,
    pub sha256_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub available: bool,
    pub notes: String,
    pub published_at: Option<String>,
    pub release_url: Option<String>,
    pub asset: Option<UpdateAsset>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Downloading,
    Verifying,
    Installing,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub stage: Stage,
}

/// What the shell should do after the installer has been launched.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PostInstall {
    /// An external installer is running; quit so it can replace the app's files.
    Exit,
    /// The running binary was replaced in place (AppImage); restart into the new version.
    Restart,
    /// The package was handed to the OS (e.g. a .deb opened in the software center);
    /// the app keeps running.
    Nothing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    pub action: PostInstall,
    pub path: String,
    pub message: String,
}

// ---------------------------------------------------------------- GitHub API shapes

#[derive(Debug, Clone, Deserialize)]
pub struct GhAsset {
    pub name: String,
    #[serde(default)]
    pub size: u64,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

// ---------------------------------------------------------------- version comparison

/// Strip a leading `v`/`V` and surrounding whitespace from a tag.
pub fn normalize_version(tag: &str) -> &str {
    let t = tag.trim();
    t.strip_prefix('v').or_else(|| t.strip_prefix('V')).unwrap_or(t)
}

fn split_version(v: &str) -> (Vec<u64>, Option<&str>) {
    let v = normalize_version(v);
    // Build metadata never affects precedence.
    let v = v.split('+').next().unwrap_or(v);
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, Some(p)),
        Some((c, _)) => (c, None),
        None => (v, None),
    };
    let nums = core.split('.').map(|p| p.trim().parse::<u64>().unwrap_or(0)).collect();
    (nums, pre)
}

fn compare_prerelease(a: &str, b: &str) -> Ordering {
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(n), Ok(m)) => n.cmp(&m),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// Semver-ish comparison: numeric `major.minor.patch` (missing parts are 0, a leading `v`
/// is ignored); a prerelease (`1.0.0-beta`) sorts before the release it precedes.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let (an, ap) = split_version(a);
    let (bn, bp) = split_version(b);
    let len = an.len().max(bn.len()).max(3);
    for i in 0..len {
        let x = an.get(i).copied().unwrap_or(0);
        let y = bn.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    match (ap, bp) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => compare_prerelease(x, y),
    }
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    compare_versions(latest, current) == Ordering::Greater
}

// ---------------------------------------------------------------- asset selection

const ARM_TOKENS: &[&str] = &["aarch64", "arm64"];
const X64_TOKENS: &[&str] = &["x86_64", "x64", "amd64"];

/// 3 = names this arch, 2 = names no arch, 1 = other arch but runnable (x64 on Apple
/// silicon via Rosetta), 0 = unusable.
fn arch_score(name: &str, os: &str, arch: &str) -> u8 {
    let is_arm = ARM_TOKENS.iter().any(|t| name.contains(t));
    let is_x64 = X64_TOKENS.iter().any(|t| name.contains(t));
    let (mine, other) = match arch {
        "aarch64" | "arm64" => (is_arm, is_x64),
        "x86_64" | "x64" | "amd64" => (is_x64, is_arm),
        _ => (false, is_arm || is_x64),
    };
    if mine {
        3
    } else if !other {
        2
    } else if os == "macos" && matches!(arch, "aarch64" | "arm64") && is_x64 {
        1
    } else {
        0
    }
}

/// Pick the installer for a platform from a release's assets. Pure, so it can be tested for
/// every platform from any host. `os`/`arch` use `std::env::consts` spellings; `appimage` is
/// true when the app is running from an AppImage (`$APPIMAGE` is set).
pub fn select_asset(assets: &[GhAsset], os: &str, arch: &str, appimage: bool) -> Option<UpdateAsset> {
    let lower: Vec<String> = assets.iter().map(|a| a.name.to_lowercase()).collect();
    let ignored = |n: &str| n.ends_with(".sig") || n.ends_with(".sha256") || n.ends_with(".sha256sum");

    // Suffix preference groups, best first.
    let groups: Vec<Vec<&str>> = match os {
        "windows" => vec![vec!["-setup.exe"], vec![".exe"], vec![".msi"]],
        "macos" => vec![vec![".dmg"]],
        "linux" if appimage => vec![vec![".appimage"], vec![".deb"]],
        "linux" => vec![vec![".deb"], vec![".appimage"]],
        _ => vec![],
    };

    for suffixes in groups {
        let best = lower
            .iter()
            .enumerate()
            .filter(|(_, n)| !ignored(n) && suffixes.iter().any(|s| n.ends_with(s)))
            .map(|(i, n)| (i, arch_score(n, os, arch)))
            .filter(|(_, score)| *score > 0)
            // max_by_key returns the last max; reverse so the first listed asset wins ties.
            .rev()
            .max_by_key(|(_, score)| *score);
        if let Some((i, _)) = best {
            let a = &assets[i];
            let sha_name = format!("{}.sha256", a.name).to_lowercase();
            let sha256_url = assets
                .iter()
                .find(|s| s.name.to_lowercase() == sha_name)
                .map(|s| s.browser_download_url.clone());
            return Some(UpdateAsset { name: a.name.clone(), size: a.size, url: a.browser_download_url.clone(), sha256_url });
        }
    }
    None
}

// ---------------------------------------------------------------- check

fn http_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() > 10 {
                attempt.error("too many redirects")
            } else if is_allowed_url(attempt.url(), ALLOWED_DOWNLOAD_HOSTS) || attempt.url().host_str() == Some("api.github.com") {
                attempt.follow()
            } else {
                attempt.error("redirect to a non-GitHub host was blocked")
            }
        }))
        .build()
        .map_err(|e| GeniusError::Internal(e.to_string()))
}

fn net_err(e: reqwest::Error) -> GeniusError {
    GeniusError::Integration(format!("Could not reach GitHub to check for updates: {e}"))
}

/// Check GitHub Releases for a version newer than `current_version`.
pub async fn check(current_version: &str) -> Result<UpdateInfo> {
    check_at(API_BASE, current_version).await
}

/// [`check`] against an arbitrary API base URL (used by tests).
pub async fn check_at(api_base: &str, current_version: &str) -> Result<UpdateInfo> {
    let client = http_client()?;
    let url = format!("{}/repos/{}/releases/latest", api_base.trim_end_matches('/'), REPO);
    let resp = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(net_err)?;

    let current = normalize_version(current_version).to_string();
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateInfo {
            current_version: current,
            latest_version: None,
            available: false,
            notes: String::new(),
            published_at: None,
            release_url: None,
            asset: None,
            detail: Some("No releases have been published yet — you're on the latest version.".into()),
        });
    }
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(GeniusError::Integration(
            "GitHub is rate-limiting update checks right now. Try again in a little while.".into(),
        ));
    }
    if !status.is_success() {
        return Err(GeniusError::Integration(format!("GitHub returned HTTP {} while checking for updates.", status.as_u16())));
    }
    let release: GhRelease = resp
        .json()
        .await
        .map_err(|e| GeniusError::Integration(format!("Unexpected response from GitHub Releases: {e}")))?;
    Ok(build_info(&current, release, std::env::consts::OS, std::env::consts::ARCH, std::env::var_os("APPIMAGE").is_some()))
}

fn build_info(current: &str, release: GhRelease, os: &str, arch: &str, appimage: bool) -> UpdateInfo {
    let latest = normalize_version(&release.tag_name).to_string();
    let newer = is_newer(&latest, current);
    let asset = select_asset(&release.assets, os, arch, appimage);
    let detail = if !newer {
        Some(format!("You're on the latest version ({current})."))
    } else if asset.is_none() {
        Some(format!("Version {latest} is available, but it has no installer for this platform ({os}/{arch}). Download it from the release page."))
    } else {
        None
    };
    UpdateInfo {
        current_version: current.to_string(),
        latest_version: Some(latest),
        available: newer,
        notes: release.body.unwrap_or_default(),
        published_at: release.published_at,
        release_url: release.html_url,
        asset,
        detail,
    }
}

// ---------------------------------------------------------------- download & install

fn is_allowed_url(url: &reqwest::Url, hosts: &[&str]) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().map(|h| hosts.contains(&h)).unwrap_or(false)
}

/// Validate an update URL: https on a GitHub release host, nothing else.
pub fn validate_download_url(raw: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(raw).map_err(|_| GeniusError::Validation("invalid update download URL".into()))?;
    if !is_allowed_url(&url, ALLOWED_ASSET_HOSTS) {
        return Err(GeniusError::Validation(
            "update downloads are only allowed from https://github.com/ or https://objects.githubusercontent.com/".into(),
        ));
    }
    Ok(url)
}

/// Reject names that could escape the download directory.
fn safe_file_name(name: &str) -> Result<&str> {
    let ok = !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'));
    if ok {
        Ok(name)
    } else {
        Err(GeniusError::Validation(format!("unsafe update file name: {name}")))
    }
}

pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("genius-update")
}

fn parse_sha256_file(text: &str) -> Option<String> {
    let token = text.split_whitespace().next()?.to_ascii_lowercase();
    (token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit())).then_some(token)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Download `asset`, verify it, and launch the installer. `progress` is called repeatedly
/// with the download position and once per later stage.
pub async fn download_and_install<F>(asset: &UpdateAsset, progress: F) -> Result<InstallOutcome>
where
    F: Fn(Progress) + Send + Sync,
{
    let path = download(asset, &progress).await?;
    progress(Progress { downloaded: asset.size, total: Some(asset.size), stage: Stage::Installing });
    let outcome = install(&path)?;
    progress(Progress { downloaded: asset.size, total: Some(asset.size), stage: Stage::Done });
    Ok(outcome)
}

/// Download (and verify) an asset into [`download_dir`]. Returns the local file path.
pub async fn download<F>(asset: &UpdateAsset, progress: &F) -> Result<PathBuf>
where
    F: Fn(Progress) + Send + Sync,
{
    let url = validate_download_url(&asset.url)?;
    let sha_url = asset.sha256_url.as_deref().map(validate_download_url).transpose()?;
    let name = safe_file_name(&asset.name)?;
    let client = http_client()?;

    let dir = download_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let dest = dir.join(name);
    let partial = dir.join(format!("{name}.part"));

    let resp = client.get(url).send().await.map_err(|e| GeniusError::Integration(format!("Update download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(GeniusError::Integration(format!("Update download failed: HTTP {}", resp.status().as_u16())));
    }
    let total = resp.content_length().or((asset.size > 0).then_some(asset.size));
    progress(Progress { downloaded: 0, total, stage: Stage::Downloading });

    let mut file = tokio::fs::File::create(&partial).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;
    let mut last_report: u64 = 0;
    let step = total.map(|t| (t / 100).max(256 * 1024)).unwrap_or(512 * 1024);
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| GeniusError::Integration(format!("Update download interrupted: {e}")))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if downloaded - last_report >= step {
            last_report = downloaded;
            progress(Progress { downloaded, total, stage: Stage::Downloading });
        }
    }
    file.flush().await?;
    drop(file);
    progress(Progress { downloaded, total, stage: Stage::Downloading });

    if let Some(t) = total {
        if downloaded != t {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(GeniusError::Integration(format!("Update download was incomplete ({downloaded} of {t} bytes).")));
        }
    }

    progress(Progress { downloaded, total, stage: Stage::Verifying });
    if let Some(sha_url) = sha_url {
        let text = client
            .get(sha_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| GeniusError::Integration(format!("Could not fetch the update checksum: {e}")))?
            .text()
            .await
            .map_err(|e| GeniusError::Integration(format!("Could not read the update checksum: {e}")))?;
        let expected = parse_sha256_file(&text)
            .ok_or_else(|| GeniusError::Integration("The update checksum file is malformed.".into()))?;
        let actual = to_hex(&hasher.finalize());
        if actual != expected {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(GeniusError::Integration(
                "The downloaded update failed its checksum verification and was discarded.".into(),
            ));
        }
    }

    let _ = tokio::fs::remove_file(&dest).await;
    tokio::fs::rename(&partial, &dest).await?;
    Ok(dest)
}

fn spawn(cmd: &mut std::process::Command, what: &str) -> Result<()> {
    cmd.spawn().map(|_| ()).map_err(|e| GeniusError::Process(format!("could not launch {what}: {e}")))
}

/// Launch the installer for a downloaded file.
pub fn install(path: &Path) -> Result<InstallOutcome> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
    let display = path.display().to_string();
    let outcome = |action, message: &str| Ok(InstallOutcome { action, path: display.clone(), message: message.into() });

    match std::env::consts::OS {
        "windows" if name.ends_with(".exe") => {
            spawn(&mut std::process::Command::new(path), "the installer")?;
            outcome(PostInstall::Exit, "The installer has started. Genius will close so it can be updated.")
        }
        "windows" if name.ends_with(".msi") => {
            spawn(std::process::Command::new("msiexec").arg("/i").arg(path), "msiexec")?;
            outcome(PostInstall::Exit, "The installer has started. Genius will close so it can be updated.")
        }
        "macos" if name.ends_with(".dmg") => {
            spawn(std::process::Command::new("open").arg(path), "the disk image")?;
            outcome(
                PostInstall::Exit,
                "The disk image is open. Drag Genius into Applications to replace the old version, then start it again.",
            )
        }
        "linux" if name.ends_with(".appimage") => {
            let target = std::env::var_os("APPIMAGE").map(PathBuf::from).ok_or_else(|| {
                GeniusError::Validation("Genius is not running from an AppImage, so it cannot be replaced in place.".into())
            })?;
            replace_file(path, &target)?;
            Ok(InstallOutcome {
                action: PostInstall::Restart,
                path: target.display().to_string(),
                message: "Genius was updated and will restart.".into(),
            })
        }
        "linux" if name.ends_with(".deb") => {
            spawn(std::process::Command::new("xdg-open").arg(path), "the package installer")?;
            outcome(PostInstall::Nothing, "The package was opened in your software installer. Restart Genius after installing it.")
        }
        os => Err(GeniusError::Validation(format!("don't know how to install {name} on {os}"))),
    }
}

/// Atomically replace `target` with `source`: copy to a sibling temp file, mark it
/// executable, then rename over the original.
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().ok_or_else(|| GeniusError::Validation("invalid AppImage path".into()))?;
    let file_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("Genius.AppImage");
    let tmp = parent.join(format!(".{file_name}.genius-update"));
    std::fs::copy(source, &tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str) -> GhAsset {
        GhAsset {
            name: name.into(),
            size: 10,
            browser_download_url: format!("https://github.com/{REPO}/releases/download/v0.2.0/{name}"),
        }
    }

    fn tauri_assets() -> Vec<GhAsset> {
        [
            "Genius_0.2.0_x64-setup.exe",
            "Genius_0.2.0_x64-setup.exe.sig",
            "Genius_0.2.0_x64-setup.exe.sha256",
            "Genius_0.2.0_x64_en-US.msi",
            "Genius_0.2.0_aarch64.dmg",
            "Genius_0.2.0_x64.dmg",
            "Genius_aarch64.app.tar.gz",
            "Genius_0.2.0_amd64.AppImage",
            "Genius_0.2.0_amd64.AppImage.sig",
            "Genius_0.2.0_amd64.deb",
            "latest.json",
        ]
        .iter()
        .map(|n| a(n))
        .collect()
    }

    #[test]
    fn version_comparison() {
        use Ordering::*;
        assert_eq!(compare_versions("0.2.0", "0.1.0"), Greater);
        assert_eq!(compare_versions("v0.2.0", "0.2.0"), Equal);
        assert_eq!(compare_versions("0.10.0", "0.9.9"), Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Equal);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Less);
        assert_eq!(compare_versions("1.0.0", "1.0.0-rc.1"), Greater);
        assert_eq!(compare_versions("1.0.0-beta.2", "1.0.0-beta.10"), Less);
        assert_eq!(compare_versions("1.0.0-alpha", "1.0.0-beta"), Less);
        assert_eq!(compare_versions("1.0.0+build.5", "1.0.0"), Equal);
        assert_eq!(compare_versions("2.0.0-rc.1", "1.9.9"), Greater);
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn selects_windows_setup_exe_first() {
        let assets = tauri_assets();
        let s = select_asset(&assets, "windows", "x86_64", false).unwrap();
        assert_eq!(s.name, "Genius_0.2.0_x64-setup.exe");
        assert!(s.sha256_url.as_deref().unwrap().ends_with("Genius_0.2.0_x64-setup.exe.sha256"));
        let msi_only: Vec<_> = assets.into_iter().filter(|x| !x.name.contains(".exe")).collect();
        let s = select_asset(&msi_only, "windows", "x86_64", false).unwrap();
        assert_eq!(s.name, "Genius_0.2.0_x64_en-US.msi");
        assert_eq!(s.sha256_url, None);
    }

    #[test]
    fn selects_mac_dmg_by_arch() {
        let assets = tauri_assets();
        assert_eq!(select_asset(&assets, "macos", "aarch64", false).unwrap().name, "Genius_0.2.0_aarch64.dmg");
        assert_eq!(select_asset(&assets, "macos", "x86_64", false).unwrap().name, "Genius_0.2.0_x64.dmg");
        // Apple silicon can fall back to an Intel build; Intel cannot run an arm build.
        let x64_only = vec![a("Genius_0.2.0_x64.dmg")];
        assert_eq!(select_asset(&x64_only, "macos", "aarch64", false).unwrap().name, "Genius_0.2.0_x64.dmg");
        let arm_only = vec![a("Genius_0.2.0_aarch64.dmg")];
        assert!(select_asset(&arm_only, "macos", "x86_64", false).is_none());
        let universal = vec![a("Genius_0.2.0_universal.dmg")];
        assert!(select_asset(&universal, "macos", "x86_64", false).is_some());
    }

    #[test]
    fn selects_linux_package_by_appimage_flag() {
        let assets = tauri_assets();
        assert_eq!(select_asset(&assets, "linux", "x86_64", true).unwrap().name, "Genius_0.2.0_amd64.AppImage");
        assert_eq!(select_asset(&assets, "linux", "x86_64", false).unwrap().name, "Genius_0.2.0_amd64.deb");
        let appimage_only = vec![a("Genius_0.2.0_amd64.AppImage"), a("Genius_0.2.0_amd64.AppImage.sig")];
        assert_eq!(select_asset(&appimage_only, "linux", "x86_64", false).unwrap().name, "Genius_0.2.0_amd64.AppImage");
        assert!(select_asset(&assets, "linux", "aarch64", false).is_none());
        assert!(select_asset(&assets, "freebsd", "x86_64", false).is_none());
        assert!(select_asset(&[a("x.sig"), a("y.sha256")], "windows", "x86_64", false).is_none());
    }

    #[test]
    fn download_urls_are_restricted_to_github() {
        assert!(validate_download_url("https://github.com/o/r/releases/download/v1/x.exe").is_ok());
        assert!(validate_download_url("https://objects.githubusercontent.com/abc").is_ok());
        assert!(validate_download_url("http://github.com/o/r/x.exe").is_err());
        assert!(validate_download_url("https://evil.com/x.exe").is_err());
        assert!(validate_download_url("https://github.com.evil.com/x.exe").is_err());
        assert!(validate_download_url("https://user@github.com/x.exe").is_err());
        assert!(validate_download_url("https://github.com:8443/x.exe").is_err());
        assert!(validate_download_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_unsafe_file_names_and_parses_checksums() {
        assert!(safe_file_name("Genius_0.2.0_x64-setup.exe").is_ok());
        assert!(safe_file_name("../evil.exe").is_err());
        assert!(safe_file_name("a/b.exe").is_err());
        assert!(safe_file_name(".hidden").is_err());
        let h = "a".repeat(64);
        assert_eq!(parse_sha256_file(&format!("{h}  Genius.exe\n")), Some(h.clone()));
        assert_eq!(parse_sha256_file(&h.to_uppercase()), Some(h));
        assert_eq!(parse_sha256_file("nope"), None);
        assert_eq!(to_hex(&Sha256::digest(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn build_info_reports_availability() {
        let rel = |tag: &str| GhRelease {
            tag_name: tag.into(),
            body: Some("notes".into()),
            html_url: Some("https://github.com/x".into()),
            published_at: Some("2026-10-01T00:00:00Z".into()),
            assets: tauri_assets(),
        };
        let i = build_info("0.1.0", rel("v0.2.0"), "linux", "x86_64", false);
        assert!(i.available);
        assert_eq!(i.latest_version.as_deref(), Some("0.2.0"));
        assert_eq!(i.asset.unwrap().name, "Genius_0.2.0_amd64.deb");
        assert_eq!(i.detail, None);
        let i = build_info("0.2.0", rel("v0.2.0"), "linux", "x86_64", false);
        assert!(!i.available);
        assert!(i.detail.is_some());
        let i = build_info("0.1.0", rel("v0.2.0"), "linux", "riscv64", false);
        assert!(i.available && i.asset.is_none() && i.detail.is_some());
    }

    #[tokio::test]
    async fn check_handles_release_and_404() {
        use wiremock::matchers::{header_exists, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let p = format!("/repos/{REPO}/releases/latest");
        Mock::given(method("GET"))
            .and(path(p.clone()))
            .and(header_exists("user-agent"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tag_name": "v9.0.0",
                "body": "Big release",
                "html_url": "https://github.com/x/y/releases/tag/v9.0.0",
                "published_at": "2026-10-01T00:00:00Z",
                "assets": [{"name": "a.deb", "size": 5, "browser_download_url": "https://github.com/a.deb"}]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let info = check_at(&server.uri(), "0.2.0").await.unwrap();
        assert!(info.available);
        assert_eq!(info.latest_version.as_deref(), Some("9.0.0"));
        assert_eq!(info.notes, "Big release");

        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(404)).mount(&server).await;
        let info = check_at(&server.uri(), "v0.2.0").await.unwrap();
        assert!(!info.available);
        assert_eq!(info.current_version, "0.2.0");
        assert!(info.detail.unwrap().contains("No releases"));
    }

    #[test]
    fn replace_file_swaps_contents_and_sets_mode() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("new.AppImage");
        let dst = dir.path().join("Genius.AppImage");
        std::fs::write(&src, b"new").unwrap();
        std::fs::write(&dst, b"old").unwrap();
        replace_file(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dst).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }
}
