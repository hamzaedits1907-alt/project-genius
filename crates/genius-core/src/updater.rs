//! In-app updates based on GitHub Releases (no signing keys required).
//!
//! [`check`] asks the GitHub API for the latest release, compares versions and picks the
//! installer asset for the running platform. [`download_and_install`] streams that asset
//! into a temp directory (`<name>.part`, renamed only after every check passed), verifies its
//! SHA-256 and launches the platform installer. Downloads are restricted to GitHub-owned hosts.
//!
//! What the checksum does and does not prove: the `.sha256` file is published in the same
//! release as the installer, so it detects corrupted or truncated downloads and a substituted
//! file on the CDN, but it is **integrity, not authenticity**. Anyone able to publish release
//! assets could publish a matching pair. Authenticity needs a signature checked against a public
//! key shipped inside the app (Tauri's minisign updater signatures, which require a private key
//! secret in CI that this repository does not have yet; see `docs/RELEASING.md`). Nothing here
//! pretends otherwise. The Windows installer is not Authenticode-signed either.
//!
//! A failed update never touches the current installation: downloads land in the temp
//! directory, an AppImage is replaced by an atomic rename with a `.bak` copy of the previous
//! file kept until the new version has started ([`finish_update`]), and [`install`] only runs
//! files that [`download_into`] left in the update directory.
//!
//! Private repositories: GitHub answers 404 to anonymous requests, so when a GitHub token is
//! available (`GENIUS_UPDATE_TOKEN`, `GH_TOKEN`/`GITHUB_TOKEN`, or the GitHub CLI login Genius
//! already uses) checks and downloads go through the authenticated API. The token is only ever
//! sent to `api.github.com`; reqwest drops it on the redirect to the asset storage host. A
//! rejected token (401) falls back to anonymous access so a stale token never blocks updates.

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
const ALLOWED_ASSET_HOSTS: &[&str] = &["github.com", "objects.githubusercontent.com", "api.github.com"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAsset {
    pub name: String,
    pub size: u64,
    pub url: String,
    pub sha256_url: Option<String>,
    /// Authenticated API download URLs (needed for private repositories).
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub sha256_api_url: Option<String>,
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
    /// API URL of the asset (`/repos/{repo}/releases/assets/{id}`).
    #[serde(default)]
    pub url: String,
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
            let sha = assets.iter().find(|s| s.name.to_lowercase() == sha_name);
            let api = |u: &str| (!u.is_empty()).then(|| u.to_string());
            return Some(UpdateAsset {
                name: a.name.clone(),
                size: a.size,
                url: a.browser_download_url.clone(),
                sha256_url: sha.map(|s| s.browser_download_url.clone()),
                api_url: api(&a.url),
                sha256_api_url: sha.and_then(|s| api(&s.url)),
            });
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

/// A GitHub token for update requests, if one is available. Order: `GENIUS_UPDATE_TOKEN`,
/// `GH_TOKEN`, `GITHUB_TOKEN`, then `gh auth token` (the login used by Settings → GitHub).
pub async fn auth_token() -> Option<String> {
    for var in ["GENIUS_UPDATE_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(var) {
            if !v.trim().is_empty() {
                return Some(v.trim().to_string());
            }
        }
    }
    let gh = crate::process::find_executable("gh")?;
    let mut cmd = tokio::process::Command::new(gh);
    cmd.args(["auth", "token", "--hostname", "github.com"]).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(Duration::from_secs(8), cmd.output()).await.ok()?.ok()?;
    let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !token.is_empty() && !token.chars().any(char::is_whitespace)).then(|| {
        crate::redact::register_secret(&token);
        token
    })
}

/// Check GitHub Releases for a version newer than `current_version`.
pub async fn check(current_version: &str) -> Result<UpdateInfo> {
    let token = auth_token().await;
    check_with(API_BASE, current_version, token.as_deref()).await
}

/// [`check`] against an arbitrary API base URL without credentials (used by tests).
pub async fn check_at(api_base: &str, current_version: &str) -> Result<UpdateInfo> {
    check_with(api_base, current_version, None).await
}

/// [`check`] against `api_base` with an optional token.
pub async fn check_with(api_base: &str, current_version: &str, token: Option<&str>) -> Result<UpdateInfo> {
    fetch_release(api_base, "latest", current_version, token).await
}

/// Like [`check_with`] but for one specific release tag instead of the latest release
/// (used by the live verification workflow to test an update to a chosen version).
pub async fn check_tag_with(api_base: &str, tag: &str, current_version: &str, token: Option<&str>) -> Result<UpdateInfo> {
    if tag.is_empty() || tag.len() > 64 || !tag.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+')) {
        return Err(GeniusError::Validation(format!("invalid release tag: {tag}")));
    }
    fetch_release(api_base, &format!("tags/{tag}"), current_version, token).await
}

async fn fetch_release(api_base: &str, endpoint: &str, current_version: &str, token: Option<&str>) -> Result<UpdateInfo> {
    let client = http_client()?;
    let url = format!("{}/repos/{}/releases/{}", api_base.trim_end_matches('/'), REPO, endpoint);
    let get = |token: Option<&str>| {
        let mut req = client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(30));
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        req.send()
    };
    let mut resp = get(token).await.map_err(net_err)?;
    let mut used_token = token.is_some();
    if used_token && resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // An expired or revoked token must not block updates of a public repository.
        resp = get(None).await.map_err(net_err)?;
        used_token = false;
    }

    let current = normalize_version(current_version).to_string();
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        let detail = if used_token {
            "No releases have been published yet — you're on the latest version."
        } else {
            "No releases found. If the Genius repository is private, connect GitHub in Settings → GitHub (or make the repository public) so Genius can see its releases."
        };
        return Ok(UpdateInfo {
            current_version: current,
            latest_version: None,
            available: false,
            notes: String::new(),
            published_at: None,
            release_url: None,
            asset: None,
            detail: Some(detail.into()),
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
    // Unit tests serve releases from a local mock server; release builds never allow this.
    if cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1") {
        return true;
    }
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
    let token = auth_token().await;
    let path = download_with(asset, token.as_deref(), &progress).await?;
    progress(Progress { downloaded: asset.size, total: Some(asset.size), stage: Stage::Installing });
    let outcome = install(&path)?;
    progress(Progress { downloaded: asset.size, total: Some(asset.size), stage: Stage::Done });
    Ok(outcome)
}

/// GET an update file: through the authenticated API URL when a token is available (the
/// only way to read private-repository assets), otherwise — or if the token is rejected —
/// from the public download URL.
async fn fetch(client: &reqwest::Client, public: &str, api: Option<&str>, token: Option<&str>, timeout: Option<Duration>) -> Result<reqwest::Response> {
    let send = |req: reqwest::RequestBuilder| async move {
        let req = match timeout {
            Some(t) => req.timeout(t),
            None => req,
        };
        req.send().await.map_err(|e| GeniusError::Integration(format!("Update download failed: {e}")))
    };
    if let (Some(api), Some(token)) = (api, token) {
        let url = validate_download_url(api)?;
        let resp = send(client.get(url).header(reqwest::header::ACCEPT, "application/octet-stream").bearer_auth(token)).await?;
        if resp.status() != reqwest::StatusCode::UNAUTHORIZED {
            return check_status(resp);
        }
    }
    check_status(send(client.get(validate_download_url(public)?)).await?)
}

fn check_status(resp: reqwest::Response) -> Result<reqwest::Response> {
    if resp.status().is_success() {
        Ok(resp)
    } else {
        Err(GeniusError::Integration(format!("Update download failed: HTTP {}", resp.status().as_u16())))
    }
}

/// Download (and verify) an asset into [`download_dir`] without credentials.
pub async fn download<F>(asset: &UpdateAsset, progress: &F) -> Result<PathBuf>
where
    F: Fn(Progress) + Send + Sync,
{
    download_with(asset, None, progress).await
}

/// Download (and verify) an asset into [`download_dir`]. Returns the local file path.
pub async fn download_with<F>(asset: &UpdateAsset, token: Option<&str>, progress: &F) -> Result<PathBuf>
where
    F: Fn(Progress) + Send + Sync,
{
    download_into(&download_dir(), asset, token, progress).await
}

/// Download (and verify) an asset into `dir`. The bytes are written to `<name>.part` and renamed
/// to `<name>` only after the size and SHA-256 checks passed, so a failure of any kind (network,
/// truncation, checksum mismatch) removes the partial file and leaves everything else in `dir`,
/// including an earlier good download of the same name, untouched.
///
/// A release that publishes no `.sha256` for the installer is refused: without it there is
/// nothing to detect a corrupted or substituted download with.
pub async fn download_into<F>(dir: &Path, asset: &UpdateAsset, token: Option<&str>, progress: &F) -> Result<PathBuf>
where
    F: Fn(Progress) + Send + Sync,
{
    validate_download_url(&asset.url)?;
    let name = safe_file_name(&asset.name)?;
    let sha_url = asset.sha256_url.as_deref().ok_or_else(|| {
        GeniusError::Integration(format!(
            "The release publishes no SHA-256 checksum for {name}, so Genius will not install it automatically. Download it from the release page instead."
        ))
    })?;
    let client = http_client()?;

    tokio::fs::create_dir_all(dir).await?;
    let dest = dir.join(name);
    let partial = dir.join(format!("{name}.part"));
    let _ = tokio::fs::remove_file(&partial).await; // leftover of an interrupted earlier attempt

    // The expected hash first: no point in downloading 100 MB we could not verify.
    let text = fetch(&client, sha_url, asset.sha256_api_url.as_deref(), token, Some(Duration::from_secs(30)))
        .await
        .map_err(|e| GeniusError::Integration(format!("Could not fetch the update checksum: {e}")))?
        .text()
        .await
        .map_err(|e| GeniusError::Integration(format!("Could not read the update checksum: {e}")))?;
    let expected = parse_sha256_file(&text).ok_or_else(|| GeniusError::Integration("The update checksum file is malformed.".into()))?;

    if let Err(e) = download_to_partial(&client, asset, token, progress, &partial, &expected).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(e);
    }
    let _ = tokio::fs::remove_file(&dest).await;
    if let Err(e) = tokio::fs::rename(&partial, &dest).await {
        let _ = tokio::fs::remove_file(&partial).await;
        return Err(e.into());
    }
    Ok(dest)
}

async fn download_to_partial<F>(client: &reqwest::Client, asset: &UpdateAsset, token: Option<&str>, progress: &F, partial: &Path, expected: &str) -> Result<()>
where
    F: Fn(Progress) + Send + Sync,
{
    let resp = fetch(client, &asset.url, asset.api_url.as_deref(), token, None).await?;
    // Fail before streaming (possibly 100 MB) when the server already announces another size.
    if let Some(announced) = resp.content_length().filter(|n| asset.size > 0 && *n != asset.size) {
        return Err(GeniusError::Integration(format!(
            "Update download was incomplete or does not match the release: the server announced {announced} bytes, the release lists {}.",
            asset.size
        )));
    }
    let total = resp.content_length().or((asset.size > 0).then_some(asset.size));
    progress(Progress { downloaded: 0, total, stage: Stage::Downloading });

    let mut file = tokio::fs::File::create(partial).await?;
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

    // The release lists the asset size; a mismatch means a truncated or substituted file.
    if let Some(t) = total.into_iter().chain((asset.size > 0).then_some(asset.size)).find(|t| *t != downloaded) {
        return Err(GeniusError::Integration(format!("Update download was incomplete ({downloaded} of {t} bytes).")));
    }

    progress(Progress { downloaded, total, stage: Stage::Verifying });
    if to_hex(&hasher.finalize()) != expected {
        return Err(GeniusError::Integration("The downloaded update failed its checksum verification and was discarded.".into()));
    }
    Ok(())
}

/// Hash `path` and compare with `expected_hex` (case-insensitive). Used to re-check a file that
/// sat on disk between download and install.
pub fn verify_file_sha256(path: &Path, expected_hex: &str) -> Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    if to_hex(&hasher.finalize()) == expected_hex.trim().to_ascii_lowercase() {
        Ok(())
    } else {
        Err(GeniusError::Integration("The update file does not match its SHA-256 checksum.".into()))
    }
}

fn spawn(cmd: &mut std::process::Command, what: &str) -> Result<()> {
    cmd.spawn().map(|_| ()).map_err(|e| GeniusError::Process(format!("could not launch {what}: {e}")))
}

/// Only files that [`download_into`] left in the update directory may be executed: an absolute
/// path with `..`, a symlink pointing elsewhere or an arbitrary file on disk is refused.
fn ensure_installable(path: &Path, dir: &Path) -> Result<()> {
    let refuse = || GeniusError::Validation("Refusing to run an update file that is not in Genius's update download folder.".into());
    let file = std::fs::canonicalize(path).map_err(|_| refuse())?;
    let root = std::fs::canonicalize(dir).map_err(|_| refuse())?;
    if file.parent() != Some(root.as_path()) || !file.is_file() {
        return Err(refuse());
    }
    safe_file_name(file.file_name().and_then(|n| n.to_str()).unwrap_or_default())?;
    Ok(())
}

/// Launch the installer for a downloaded file.
pub fn install(path: &Path) -> Result<InstallOutcome> {
    install_from(path, &download_dir())
}

fn install_from(path: &Path, dir: &Path) -> Result<InstallOutcome> {
    ensure_installable(path, dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
    let display = path.display().to_string();
    let outcome = |action, message: &str| Ok(InstallOutcome { action, path: display.clone(), message: message.into() });

    match std::env::consts::OS {
        "windows" if name.ends_with(".exe") => {
            // Interactive NSIS installer: the user sees it and can cancel it.
            spawn(&mut std::process::Command::new(path), "the installer")?;
            outcome(PostInstall::Exit, "The installer has started. Genius will close so it can be updated.")
        }
        "windows" if name.ends_with(".msi") => {
            spawn(std::process::Command::new("msiexec").arg("/i").arg(path), "msiexec")?;
            outcome(PostInstall::Exit, "The installer has started. Genius will close so it can be updated.")
        }
        "macos" if name.ends_with(".dmg") => {
            spawn(std::process::Command::new("open").arg(path), "the disk image")?;
            outcome(PostInstall::Exit, "The disk image is open. Drag Genius into Applications to replace the old version, then start it again.")
        }
        "linux" if name.ends_with(".appimage") => {
            let target = std::env::var_os("APPIMAGE")
                .map(PathBuf::from)
                .ok_or_else(|| GeniusError::Validation("Genius is not running from an AppImage, so it cannot be replaced in place.".into()))?;
            replace_file(path, &target)?;
            // The downloaded copy is no longer needed; the previous AppImage stays as `.bak`
            // until the new version has started (see [`finish_update`] / [`rollback_appimage`]).
            let _ = std::fs::remove_file(path);
            Ok(InstallOutcome { action: PostInstall::Restart, path: target.display().to_string(), message: "Genius was updated and will restart.".into() })
        }
        "linux" if name.ends_with(".deb") => {
            spawn(std::process::Command::new("xdg-open").arg(path), "the package installer")?;
            outcome(PostInstall::Nothing, "The package was opened in your software installer. Restart Genius after installing it.")
        }
        os => Err(GeniusError::Validation(format!("don't know how to install {name} on {os}"))),
    }
}

/// Run the downloaded Windows NSIS installer **silently** (`/S`, optionally into `install_dir`
/// via `/D=`) and wait for it to finish. This exists for the automated verification workflow
/// (`.github/workflows/updater-windows.yml`); the app itself only ever calls [`install`], which
/// starts the interactive installer. Returns the installer's exit code.
pub fn install_silent(path: &Path, install_dir: Option<&Path>, timeout: Duration) -> Result<i32> {
    #[cfg(not(windows))]
    {
        let _ = (path, install_dir, timeout);
        Err(GeniusError::Validation("silent installation is only implemented for the Windows NSIS installer".into()))
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        ensure_installable(path, &download_dir())?;
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_lowercase();
        if !name.ends_with("-setup.exe") {
            return Err(GeniusError::Validation(format!("{name} is not an NSIS setup executable")));
        }
        let mut cmd = std::process::Command::new(path);
        cmd.arg("/S");
        if let Some(dir) = install_dir {
            // NSIS requires /D= to be the last argument and unquoted, even with spaces.
            cmd.raw_arg(format!("/D={}", dir.display()));
        }
        let mut child = cmd.spawn().map_err(|e| GeniusError::Process(format!("could not launch the installer: {e}")))?;
        let started = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
                Ok(None) if started.elapsed() > timeout => {
                    let _ = child.kill();
                    return Err(GeniusError::Process(format!("the installer did not finish within {} s", timeout.as_secs())));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(500)),
                Err(e) => return Err(GeniusError::Process(format!("waiting for the installer failed: {e}"))),
            }
        }
    }
}

fn sibling(target: &Path, prefix: &str, suffix: &str) -> Result<PathBuf> {
    let parent = target.parent().ok_or_else(|| GeniusError::Validation("invalid AppImage path".into()))?;
    let file_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("Genius.AppImage");
    Ok(parent.join(format!("{prefix}{file_name}{suffix}")))
}

/// Where the previous AppImage is kept while an update has not been confirmed.
pub fn backup_path(target: &Path) -> PathBuf {
    sibling(target, "", ".bak").unwrap_or_else(|_| target.with_extension("bak"))
}

/// Atomically replace `target` with `source`: copy to a sibling temp file, check its size, mark
/// it executable, keep a copy of the current file as `<target>.bak`, then rename over the
/// original. Any failure before the final rename leaves `target` exactly as it was; a failing
/// rename removes the temp file and also leaves `target` untouched.
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let tmp = sibling(target, ".", ".genius-update")?;
    let result = (|| -> Result<()> {
        std::fs::copy(source, &tmp)?;
        if std::fs::metadata(&tmp)?.len() != std::fs::metadata(source)?.len() {
            return Err(GeniusError::Integration("The update could not be copied completely (is the disk full?). The current version was not changed.".into()));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        if target.is_file() {
            std::fs::copy(target, backup_path(target))
                .map_err(|e| GeniusError::Integration(format!("Could not keep a backup of the current version, so it was not replaced: {e}")))?;
        }
        std::fs::rename(&tmp, target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Put the previous AppImage back (after the new one failed to start). Returns whether a backup
/// existed.
pub fn rollback_appimage(target: &Path) -> Result<bool> {
    let bak = backup_path(target);
    if !bak.is_file() {
        return Ok(false);
    }
    std::fs::rename(&bak, target)?;
    Ok(true)
}

/// Housekeeping once the (new) version is running: drop the AppImage backup and stale update
/// downloads (anything in the update folder older than an hour; an installer that was just
/// downloaded may still be running, so recent files stay). Best effort, never fails.
pub fn finish_update() {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let _ = std::fs::remove_file(backup_path(Path::new(&appimage)));
    }
    remove_stale_downloads(&download_dir(), Duration::from_secs(3600));
}

fn remove_stale_downloads(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let old = entry.metadata().ok().filter(|m| m.is_file()).and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).map(|age| age >= max_age).unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(name: &str) -> GhAsset {
        GhAsset {
            name: name.into(),
            size: 10,
            browser_download_url: format!("https://github.com/{REPO}/releases/download/v0.2.0/{name}"),
            url: format!("https://api.github.com/repos/{REPO}/releases/assets/{}", name.len()),
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

    #[tokio::test]
    async fn token_is_used_and_a_rejected_token_falls_back_to_anonymous() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let p = format!("/repos/{REPO}/releases/latest");
        let release = serde_json::json!({ "tag_name": "v9.0.0", "assets": [] });
        Mock::given(method("GET")).and(path(p.clone())).and(header("authorization", "Bearer good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(release.clone())).mount(&server).await;
        Mock::given(method("GET")).and(path(p.clone())).and(header("authorization", "Bearer stale"))
            .respond_with(ResponseTemplate::new(401)).mount(&server).await;
        // Anonymous requests see nothing, like a private repository.
        Mock::given(method("GET")).and(path(p)).respond_with(ResponseTemplate::new(404)).mount(&server).await;

        let info = check_with(&server.uri(), "0.2.0", Some("good")).await.unwrap();
        assert!(info.available, "{info:?}");
        let info = check_with(&server.uri(), "0.2.0", Some("stale")).await.unwrap();
        assert!(!info.available);
        assert!(info.detail.unwrap().contains("private"), "anonymous 404 explains private repositories");
    }

    #[tokio::test]
    async fn download_verifies_checksum_and_uses_the_api_with_a_token() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let body = b"#!/bin/sh\necho genius 9.0.0\n".to_vec();
        let sum = to_hex(&Sha256::digest(&body));
        Mock::given(method("GET")).and(path("/dl/Genius_9.0.0_amd64.AppImage"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone())).mount(&server).await;
        Mock::given(method("GET")).and(path("/dl/Genius_9.0.0_amd64.AppImage.sha256"))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!("{sum}  Genius_9.0.0_amd64.AppImage\n"))).mount(&server).await;
        Mock::given(method("GET")).and(path("/dl/bad.sha256"))
            .respond_with(ResponseTemplate::new(200).set_body_string("0".repeat(64))).mount(&server).await;
        // Private-repo path: the API URL (with token) redirects to storage.
        Mock::given(method("GET")).and(path("/api/assets/1")).and(header("authorization", "Bearer t"))
            .and(header("accept", "application/octet-stream"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}/dl/Genius_9.0.0_amd64.AppImage", server.uri()))).mount(&server).await;
        Mock::given(method("GET")).and(path("/api/assets/2")).and(header("authorization", "Bearer t"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", format!("{}/dl/Genius_9.0.0_amd64.AppImage.sha256", server.uri()))).mount(&server).await;

        let asset = UpdateAsset {
            name: "Genius_9.0.0_amd64.AppImage".into(),
            size: body.len() as u64,
            url: format!("{}/dl/Genius_9.0.0_amd64.AppImage", server.uri()),
            sha256_url: Some(format!("{}/dl/Genius_9.0.0_amd64.AppImage.sha256", server.uri())),
            api_url: Some(format!("{}/api/assets/1", server.uri())),
            sha256_api_url: Some(format!("{}/api/assets/2", server.uri())),
        };
        let stages = std::sync::Mutex::new(Vec::new());
        let got = download_with(&asset, None, &|p: Progress| stages.lock().unwrap().push(p.stage)).await.unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body);
        assert!(stages.lock().unwrap().contains(&Stage::Verifying));

        let got = download_with(&asset, Some("t"), &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body, "authenticated API download works");

        let bad = UpdateAsset { sha256_url: Some(format!("{}/dl/bad.sha256", server.uri())), sha256_api_url: None, api_url: None, ..asset.clone() };
        let e = download_with(&bad, None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("checksum"), "{e}");

        let short = UpdateAsset { size: body.len() as u64 + 5, sha256_url: None, sha256_api_url: None, api_url: None, ..asset };
        assert!(download_with(&short, None, &|_| {}).await.is_err(), "incomplete downloads are rejected");
    }
}

/// Failure behaviour: what the updater leaves behind when something goes wrong.
#[cfg(test)]
mod failure_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn asset_for(server: &MockServer, size: u64, sha: Option<&str>) -> UpdateAsset {
        UpdateAsset {
            name: "Genius_9.0.0_amd64.AppImage".into(),
            size,
            url: format!("{}/dl/Genius_9.0.0_amd64.AppImage", server.uri()),
            sha256_url: sha.map(|p| format!("{}{p}", server.uri())),
            api_url: None,
            sha256_api_url: None,
        }
    }

    async fn serve(server: &MockServer, body: &[u8], sha_text: Option<String>) {
        Mock::given(method("GET")).and(path("/dl/Genius_9.0.0_amd64.AppImage")).respond_with(ResponseTemplate::new(200).set_body_bytes(body.to_vec())).mount(server).await;
        if let Some(t) = sha_text {
            Mock::given(method("GET")).and(path("/dl/sha")).respond_with(ResponseTemplate::new(200).set_body_string(t)).mount(server).await;
        }
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn good_download_is_renamed_from_part_and_replaces_an_older_copy() {
        let server = MockServer::start().await;
        let body = b"new installer bytes".to_vec();
        serve(&server, &body, Some(format!("{}  x\n", to_hex(&Sha256::digest(&body))))).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Genius_9.0.0_amd64.AppImage"), b"stale earlier download").unwrap();
        let got = download_into(dir.path(), &asset_for(&server, body.len() as u64, Some("/dl/sha")), None, &|_| {}).await.unwrap();
        assert_eq!(std::fs::read(&got).unwrap(), body);
        assert_eq!(leftovers(dir.path()), vec!["Genius_9.0.0_amd64.AppImage"], "no .part left behind");
        verify_file_sha256(&got, &to_hex(&Sha256::digest(&body))).unwrap();
    }

    #[tokio::test]
    async fn partial_download_is_rejected_without_touching_the_previous_file() {
        let server = MockServer::start().await;
        let body = b"only part of the file".to_vec();
        serve(&server, &body, Some(format!("{}\n", to_hex(&Sha256::digest(&body))))).await;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("Genius_9.0.0_amd64.AppImage");
        std::fs::write(&existing, b"previous good download").unwrap();
        // The release says the file is 1000 bytes; the server delivers fewer.
        let e = download_into(dir.path(), &asset_for(&server, 1000, Some("/dl/sha")), None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("incomplete"), "{e}");
        assert_eq!(std::fs::read(&existing).unwrap(), b"previous good download");
        assert_eq!(leftovers(dir.path()), vec!["Genius_9.0.0_amd64.AppImage"], "the .part file is removed");
    }

    #[tokio::test]
    async fn connection_cut_mid_download_leaves_no_partial_file() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        // A raw server that promises 100000 bytes, sends 10 and hangs up.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    if req.contains("/sha") {
                        let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: 65\r\n\r\n{}\n", "a".repeat(64)).as_bytes()).await;
                    } else {
                        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n0123456789").await;
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        let asset = UpdateAsset {
            name: "Genius_9.0.0_amd64.AppImage".into(),
            size: 100000,
            url: format!("http://127.0.0.1:{port}/dl/file"),
            sha256_url: Some(format!("http://127.0.0.1:{port}/sha")),
            api_url: None,
            sha256_api_url: None,
        };
        let dir = tempfile::tempdir().unwrap();
        let e = download_into(dir.path(), &asset, None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("interrupted") || e.contains("incomplete") || e.contains("failed"), "{e}");
        assert!(leftovers(dir.path()).is_empty(), "{:?}", leftovers(dir.path()));
    }

    #[tokio::test]
    async fn checksum_mismatch_discards_the_download() {
        let server = MockServer::start().await;
        let body = b"tampered installer".to_vec();
        serve(&server, &body, Some("0".repeat(64))).await;
        let dir = tempfile::tempdir().unwrap();
        let e = download_into(dir.path(), &asset_for(&server, body.len() as u64, Some("/dl/sha")), None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("checksum") && e.contains("discarded"), "{e}");
        assert!(leftovers(dir.path()).is_empty(), "neither the file nor a .part remains: {:?}", leftovers(dir.path()));
        // A malformed checksum file is also a refusal, not a pass.
        let server = MockServer::start().await;
        serve(&server, &body, Some("<html>not found</html>".into())).await;
        let e = download_into(dir.path(), &asset_for(&server, body.len() as u64, Some("/dl/sha")), None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("malformed"), "{e}");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn a_release_without_a_checksum_is_refused_before_any_download() {
        let server = MockServer::start().await; // nothing mounted: any request would 404
        let dir = tempfile::tempdir().unwrap();
        let e = download_into(dir.path(), &asset_for(&server, 5, None), None, &|_| {}).await.unwrap_err().to_string();
        assert!(e.contains("no SHA-256 checksum"), "{e}");
        assert!(server.received_requests().await.unwrap().is_empty(), "no request was made");
        assert!(leftovers(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn foreign_hosts_and_unsafe_names_are_refused_before_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        let evil = UpdateAsset { name: "Genius_setup.exe".into(), size: 1, url: "https://evil.example.com/Genius_setup.exe".into(), sha256_url: Some("https://github.com/x.sha256".into()), api_url: None, sha256_api_url: None };
        assert!(download_into(dir.path(), &evil, None, &|_| {}).await.is_err());
        let traversal = UpdateAsset { name: "../Genius_setup.exe".into(), url: "https://github.com/o/r/releases/download/v1/x.exe".into(), ..evil.clone() };
        assert!(download_into(dir.path(), &traversal, None, &|_| {}).await.is_err());
        assert!(leftovers(dir.path()).is_empty());
    }

    #[test]
    fn verify_file_sha256_detects_tampering() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.bin");
        std::fs::write(&f, b"abc").unwrap();
        let good = "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD";
        verify_file_sha256(&f, good).unwrap();
        std::fs::write(&f, b"abd").unwrap();
        assert!(verify_file_sha256(&f, good).is_err());
        assert!(verify_file_sha256(&dir.path().join("missing"), good).is_err());
    }

    #[test]
    fn install_refuses_files_outside_the_update_folder() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ok = dir.path().join("Genius_9.0.0_amd64.deb");
        std::fs::write(&ok, b"x").unwrap();
        ensure_installable(&ok, dir.path()).unwrap();

        let elsewhere = outside.path().join("Genius_9.0.0_amd64.deb");
        std::fs::write(&elsewhere, b"x").unwrap();
        assert!(ensure_installable(&elsewhere, dir.path()).is_err(), "a file in another folder");
        let dotdot = dir.path().join("..").join(outside.path().file_name().unwrap()).join("Genius_9.0.0_amd64.deb");
        assert!(ensure_installable(&dotdot, dir.path()).is_err(), "a .. path into another folder");
        assert!(ensure_installable(&dir.path().join("missing.exe"), dir.path()).is_err(), "a missing file");
        assert!(ensure_installable(dir.path(), dir.path()).is_err(), "a directory");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("x.exe"), b"x").unwrap();
        assert!(ensure_installable(&dir.path().join("sub").join("x.exe"), dir.path()).is_err(), "a file in a sub folder");
        let spaced = dir.path().join("my installer.exe");
        std::fs::write(&spaced, b"x").unwrap();
        assert!(ensure_installable(&spaced, dir.path()).is_err(), "an unsafe file name");
        #[cfg(unix)]
        {
            let link = dir.path().join("link.deb");
            std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
            assert!(ensure_installable(&link, dir.path()).is_err(), "a symlink pointing out of the folder");
        }
        // The public entry point applies the same rule against the real update folder.
        assert!(install(&elsewhere).is_err());
        assert!(install(Path::new("/etc/hosts")).is_err());
        assert!(install(Path::new(r"C:\Windows\System32\cmd.exe")).is_err());
        assert!(elsewhere.exists(), "refusing leaves the file alone");
    }

    #[test]
    fn replace_keeps_a_backup_and_rollback_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("new.AppImage");
        let dst = dir.path().join("Genius.AppImage");
        std::fs::write(&src, b"new version").unwrap();
        std::fs::write(&dst, b"old version").unwrap();
        replace_file(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), b"new version");
        assert_eq!(std::fs::read(backup_path(&dst)).unwrap(), b"old version");
        assert!(!dir.path().join(".Genius.AppImage.genius-update").exists(), "temp file is gone");

        assert!(rollback_appimage(&dst).unwrap());
        assert_eq!(std::fs::read(&dst).unwrap(), b"old version");
        assert!(!backup_path(&dst).exists());
        assert!(!rollback_appimage(&dst).unwrap(), "nothing left to roll back");
    }

    #[test]
    fn a_failed_replace_never_touches_the_current_installation() {
        let dir = tempfile::tempdir().unwrap();
        let dst = dir.path().join("Genius.AppImage");
        std::fs::write(&dst, b"running version").unwrap();
        // Source missing: nothing may change.
        assert!(replace_file(&dir.path().join("missing.AppImage"), &dst).is_err());
        assert_eq!(std::fs::read(&dst).unwrap(), b"running version");
        assert_eq!(leftovers(dir.path()), vec!["Genius.AppImage"], "no temp file, no backup");

        // Target cannot be replaced (a non-empty directory sits there): the error is reported
        // and no temp file is left.
        let blocked = dir.path().join("Blocked.AppImage");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("keep"), b"x").unwrap();
        let src = dir.path().join("new.AppImage");
        std::fs::write(&src, b"new").unwrap();
        assert!(replace_file(&src, &blocked).is_err());
        assert!(blocked.join("keep").exists());
        assert!(!dir.path().join(".Blocked.AppImage.genius-update").exists());
        assert_eq!(std::fs::read(&dst).unwrap(), b"running version");
    }

    #[test]
    fn stale_downloads_are_cleaned_but_recent_ones_stay() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old-setup.exe");
        let fresh = dir.path().join("fresh-setup.exe");
        std::fs::write(&old, b"x").unwrap();
        std::fs::write(&fresh, b"x").unwrap();
        let two_hours_ago = std::time::SystemTime::now() - Duration::from_secs(7200);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(two_hours_ago).unwrap();
        remove_stale_downloads(dir.path(), Duration::from_secs(3600));
        assert!(!old.exists());
        assert!(fresh.exists());
    }

    #[tokio::test]
    async fn check_tag_reads_a_specific_release_and_validates_the_tag() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/repos/{REPO}/releases/tags/v0.3.2")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "tag_name": "v0.3.2", "assets": [] })))
            .mount(&server)
            .await;
        let info = check_tag_with(&server.uri(), "v0.3.2", "0.3.1", None).await.unwrap();
        assert!(info.available);
        assert_eq!(info.latest_version.as_deref(), Some("0.3.2"));
        assert!(check_tag_with(&server.uri(), "../x", "0.3.1", None).await.is_err());
        assert!(check_tag_with(&server.uri(), "", "0.3.1", None).await.is_err());
    }
}
