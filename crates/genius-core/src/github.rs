//! GitHub integration through the official GitHub CLI (`gh`).
//!
//! Why gh: it implements GitHub's OAuth device flow with a registered OAuth app, stores
//! the token in the OS credential store, and exposes documented commands for repository
//! management. Genius never sees or stores the GitHub token; git network operations use
//! `gh auth git-credential` as a per-command credential helper (see `git.rs`).

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::error::{GeniusError, Result};
use crate::process::{capture, find_executable};
use crate::redact::redact;

pub const HOST: &str = "github.com";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub gh_installed: bool,
    pub gh_version: Option<String>,
    pub connected: bool,
    pub login: Option<String>,
    pub scopes: Vec<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub name_with_owner: String,
    pub name: String,
    pub owner: String,
    pub visibility: String,
    pub url: String,
    pub description: Option<String>,
    pub updated_at: Option<String>,
    pub default_branch: Option<String>,
    pub is_empty: Option<bool>,
}

#[derive(Clone)]
pub struct GitHub {
    gh: PathBuf,
}

static CODE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"one-time code:\s*([A-Z0-9]{4}-[A-Z0-9]{4})").unwrap());
static REPO_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9._-]{1,100}$").unwrap());

impl GitHub {
    pub fn detect() -> Option<Self> {
        find_executable("gh").map(|gh| Self { gh })
    }

    pub fn path(&self) -> &Path {
        &self.gh
    }

    async fn gh(&self, args: &[&str], timeout_secs: u64) -> Result<(i32, String, String)> {
        let (c, o, e) = capture(&self.gh, args, None, Duration::from_secs(timeout_secs)).await?;
        Ok((c, o, redact(&e)))
    }

    async fn gh_ok(&self, args: &[&str], timeout_secs: u64) -> Result<String> {
        let (c, o, e) = self.gh(args, timeout_secs).await?;
        if c == 0 {
            Ok(o)
        } else {
            Err(GeniusError::GitHub(classify_gh_error(&e, &o)))
        }
    }

    pub async fn status(&self) -> AuthStatus {
        let mut st = AuthStatus { gh_installed: true, ..Default::default() };
        st.gh_version = self.gh(&["--version"], 10).await.ok().map(|(_, o, _)| o.lines().next().unwrap_or("").to_string());
        match self.gh(&["auth", "status", "--hostname", HOST], 20).await {
            Ok((0, out, err)) => {
                let text = format!("{out}\n{err}");
                st.connected = true;
                st.scopes = parse_scopes(&text);
                st.detail = "Connected through GitHub CLI.".into();
                if let Ok(login) = self.gh_ok(&["api", "user", "--jq", ".login"], 20).await {
                    st.login = Some(login.trim().to_string());
                }
            }
            Ok((_, out, err)) => {
                st.detail = redact(format!("{out}\n{err}").trim());
            }
            Err(e) => st.detail = e.to_string(),
        }
        st
    }

    /// Start GitHub's browser/device login through gh. `on_code` receives the one-time code
    /// as soon as gh prints it; the future resolves when gh finishes (user approved or it failed).
    pub async fn login_web(&self, on_code: impl Fn(&str) + Send + 'static) -> Result<()> {
        let mut cmd = tokio::process::Command::new(&self.gh);
        cmd.args(["auth", "login", "--hostname", HOST, "--git-protocol", "https", "--web", "--skip-ssh-key"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| GeniusError::GitHub(format!("could not start gh: {e}")))?;
        let mut stdin = child.stdin.take();
        let stderr = child.stderr.take().ok_or_else(|| GeniusError::Internal("no stderr".into()))?;
        let mut collected = String::new();
        let mut lines = BufReader::new(stderr).lines();
        let wait = async {
            while let Ok(Some(line)) = lines.next_line().await {
                collected.push_str(&redact(&line));
                collected.push('\n');
                if let Some(c) = CODE_RE.captures(&line) {
                    on_code(&c[1]);
                }
                if line.contains("Press Enter") {
                    if let Some(s) = stdin.as_mut() {
                        let _ = s.write_all(b"\n").await;
                    }
                }
            }
            child.wait().await
        };
        let status = tokio::time::timeout(Duration::from_secs(900), wait)
            .await
            .map_err(|_| GeniusError::GitHub("GitHub login timed out after 15 minutes.".into()))?
            .map_err(|e| GeniusError::GitHub(e.to_string()))?;
        if status.success() {
            Ok(())
        } else {
            Err(GeniusError::GitHub(format!("GitHub login did not complete.\n{}", collected.trim())))
        }
    }

    /// Alternative: hand a token entered in a masked field directly to gh via stdin.
    /// gh validates it and stores it in the OS credential store; Genius keeps no copy.
    pub async fn login_with_token(&self, token: &str) -> Result<()> {
        let token = token.trim();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(GeniusError::Validation("The token is empty or malformed.".into()));
        }
        crate::redact::register_secret(token);
        let mut cmd = tokio::process::Command::new(&self.gh);
        cmd.args(["auth", "login", "--hostname", HOST, "--git-protocol", "https", "--with-token"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = cmd.spawn().map_err(|e| GeniusError::GitHub(format!("could not start gh: {e}")))?;
        if let Some(mut s) = child.stdin.take() {
            s.write_all(token.as_bytes()).await?;
            s.write_all(b"\n").await?;
        }
        let out = tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await.map_err(|_| GeniusError::GitHub("gh did not respond".into()))??;
        if out.status.success() {
            Ok(())
        } else {
            Err(GeniusError::GitHub(redact(&String::from_utf8_lossy(&out.stderr))))
        }
    }

    pub async fn logout(&self) -> Result<()> {
        let st = self.status().await;
        let mut args = vec!["auth", "logout", "--hostname", HOST];
        let login;
        if let Some(l) = &st.login {
            login = l.clone();
            args.extend(["--user", login.as_str()]);
        }
        self.gh_ok(&args, 30).await.map(|_| ())
    }

    pub async fn list_repos(&self, owner: Option<&str>, limit: usize) -> Result<Vec<RepoInfo>> {
        let lim = limit.clamp(1, 1000).to_string();
        let mut args = vec!["repo", "list"];
        if let Some(o) = owner {
            validate_owner(o)?;
            args.push(o);
        }
        args.extend(["--limit", &lim, "--json", "nameWithOwner,name,owner,visibility,url,description,updatedAt,defaultBranchRef,isEmpty"]);
        let out = self.gh_ok(&args, 60).await?;
        parse_repo_list(&out)
    }

    pub async fn view(&self, name_with_owner: &str) -> Result<Option<RepoInfo>> {
        validate_full_name(name_with_owner)?;
        let (c, o, e) = self.gh(&["repo", "view", name_with_owner, "--json", "nameWithOwner,name,owner,visibility,url,description,updatedAt,defaultBranchRef,isEmpty"], 30).await?;
        if c != 0 {
            let l = e.to_lowercase();
            if l.contains("could not resolve") || l.contains("not found") {
                return Ok(None);
            }
            return Err(GeniusError::GitHub(classify_gh_error(&e, &o)));
        }
        let v: Value = serde_json::from_str(&o)?;
        Ok(Some(repo_from_json(&v)))
    }

    /// Create a repository. Visibility must be chosen explicitly; there is no default.
    pub async fn create_repo(&self, owner: &str, name: &str, visibility: &str, description: &str) -> Result<RepoInfo> {
        validate_owner(owner)?;
        validate_repo_name(name)?;
        let vis = match visibility {
            "private" => "--private",
            "public" => "--public",
            "internal" => "--internal",
            _ => return Err(GeniusError::Validation("Choose private, public or internal visibility.".into())),
        };
        let full = format!("{owner}/{name}");
        if self.view(&full).await?.is_some() {
            return Err(GeniusError::Conflict(format!("{full} already exists on GitHub.")));
        }
        let desc: String = description.chars().filter(|c| !c.is_control()).take(300).collect();
        let mut args = vec!["repo", "create", full.as_str(), vis];
        if !desc.is_empty() {
            args.extend(["--description", desc.as_str()]);
        }
        self.gh_ok(&args, 60).await?;
        self.view(&full).await?.ok_or_else(|| GeniusError::GitHub("GitHub reported success but the repository could not be found".into()))
    }

    pub async fn orgs(&self) -> Result<Vec<String>> {
        let out = self.gh_ok(&["api", "user/orgs", "--jq", ".[].login"], 30).await?;
        Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
    }
}

pub fn validate_repo_name(name: &str) -> Result<()> {
    if REPO_NAME_RE.is_match(name) && name != "." && name != ".." && !name.starts_with('-') {
        Ok(())
    } else {
        Err(GeniusError::Validation("Repository names may contain letters, numbers, '.', '-' and '_' (max 100).".into()))
    }
}

fn validate_owner(owner: &str) -> Result<()> {
    if !owner.is_empty() && owner.len() <= 39 && !owner.starts_with('-') && owner.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        Ok(())
    } else {
        Err(GeniusError::Validation(format!("'{owner}' is not a valid GitHub account name")))
    }
}

pub fn validate_full_name(full: &str) -> Result<()> {
    let (o, n) = full.split_once('/').ok_or_else(|| GeniusError::Validation("expected owner/name".into()))?;
    validate_owner(o)?;
    validate_repo_name(n)
}

fn parse_scopes(text: &str) -> Vec<String> {
    text.lines()
        .find(|l| l.to_lowercase().contains("token scopes"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, s)| s.split(',').map(|x| x.trim().trim_matches('\'').to_string()).filter(|x| !x.is_empty()).collect())
        .unwrap_or_default()
}

fn classify_gh_error(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() { stdout } else { stderr };
    let l = text.to_lowercase();
    let hint = if l.contains("gh auth login") || l.contains("not logged") || l.contains("authentication") {
        "Not connected to GitHub. Connect your account in Settings → GitHub. "
    } else if l.contains("name already exists") {
        "A repository with that name already exists. "
    } else if l.contains("http 403") || l.contains("resource not accessible") {
        "GitHub denied the request (missing permission or scope). "
    } else {
        ""
    };
    redact(&format!("{hint}{}", text.trim()))
}

fn repo_from_json(v: &Value) -> RepoInfo {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    RepoInfo {
        name_with_owner: s("nameWithOwner").unwrap_or_default(),
        name: s("name").unwrap_or_default(),
        owner: v.pointer("/owner/login").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        visibility: s("visibility").unwrap_or_default().to_lowercase(),
        url: s("url").unwrap_or_default(),
        description: s("description").filter(|d| !d.is_empty()),
        updated_at: s("updatedAt"),
        default_branch: v.pointer("/defaultBranchRef/name").and_then(|x| x.as_str()).map(String::from),
        is_empty: v.get("isEmpty").and_then(|x| x.as_bool()),
    }
}

pub fn parse_repo_list(json: &str) -> Result<Vec<RepoInfo>> {
    let v: Value = serde_json::from_str(json)?;
    Ok(v.as_array().map(|a| a.iter().map(repo_from_json).collect()).unwrap_or_default())
}

pub fn parse_device_code(line: &str) -> Option<String> {
    CODE_RE.captures(line).map(|c| c[1].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repo_list() {
        let j = r#"[{"nameWithOwner":"me/app","name":"app","owner":{"login":"me"},"visibility":"PRIVATE","url":"https://github.com/me/app","description":"","updatedAt":"2026-01-01T00:00:00Z","defaultBranchRef":{"name":"main"},"isEmpty":false}]"#;
        let r = parse_repo_list(j).unwrap();
        assert_eq!(r[0].owner, "me");
        assert_eq!(r[0].visibility, "private");
        assert_eq!(r[0].default_branch.as_deref(), Some("main"));
        assert_eq!(r[0].description, None);
    }

    #[test]
    fn parses_device_code_and_scopes() {
        assert_eq!(parse_device_code("! First copy your one-time code: AB12-CD34").as_deref(), Some("AB12-CD34"));
        assert_eq!(parse_scopes("  - Token scopes: 'gist', 'read:org', 'repo'"), vec!["gist", "read:org", "repo"]);
    }

    #[test]
    fn validates_names() {
        assert!(validate_repo_name("my-app_1.0").is_ok());
        assert!(validate_repo_name("bad name").is_err());
        assert!(validate_repo_name("-x").is_err());
        assert!(validate_full_name("me/app").is_ok());
        assert!(validate_full_name("me/../x").is_err());
        assert!(validate_full_name("--repo/x").is_err());
    }

    #[test]
    fn error_classification_redacts() {
        let m = classify_gh_error("HTTP 401: Bad credentials ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ", "");
        assert!(!m.contains("ghp_ABC"));
    }
}
