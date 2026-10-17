//! Git integration through the installed `git` executable.
//!
//! Every command uses an explicit argument vector (no shell), runs non-interactively
//! (`GIT_TERMINAL_PROMPT=0`), and is scoped to the project directory. Genius refuses to
//! operate on a repository whose top level is *not* the project directory, so a project
//! that happens to live inside some other repository can never modify that repository.
//! No destructive command (force push, reset --hard, clean, history rewrite) is exposed on a
//! user repository. The one force push is [`Git::push_pages_snapshot`]: it publishes a build
//! from a throwaway repository to the Genius-owned `gh-pages` branch only.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::{GeniusError, Result};
use crate::process::{capture, find_executable};
use crate::redact::redact;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileStatus {
    pub path: String,
    pub orig_path: Option<String>,
    /// Porcelain v2 XY codes: index (staged) and worktree (unstaged) state.
    pub index: String,
    pub worktree: String,
    pub staged: bool,
    pub unstaged: bool,
    pub untracked: bool,
    pub conflicted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: i64,
    pub behind: i64,
    pub files: Vec<FileStatus>,
    pub merging: bool,
    pub remotes: Vec<Remote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub sha: String,
    pub short: String,
    pub author: String,
    pub email: String,
    pub date: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Branch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOutput {
    pub command: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Branch GitHub Pages serves the website from. Owned by Genius: replaced on every deploy.
pub const PAGES_BRANCH: &str = "gh-pages";

#[derive(Clone)]
pub struct Git {
    program: PathBuf,
    /// Optional credential helper (e.g. GitHub CLI) used for network operations only.
    pub credential_helper: Option<String>,
}

impl Git {
    pub fn detect() -> Result<Self> {
        let program = find_executable("git").ok_or_else(|| {
            GeniusError::MissingTool("Git is not installed or not on PATH. Install it from https://git-scm.com and restart Genius.".into())
        })?;
        Ok(Self { program, credential_helper: None })
    }

    pub fn with_gh_helper(mut self, gh: Option<&Path>) -> Self {
        self.credential_helper = gh.map(|p| format!("!\"{}\" auth git-credential", p.to_string_lossy().replace('\\', "/")));
        self
    }

    async fn raw(&self, cwd: &Path, args: &[&str], network: bool) -> Result<GitOutput> {
        let mut full: Vec<String> = vec!["-c".into(), "core.quotepath=false".into(), "-c".into(), "color.ui=false".into()];
        if network {
            if let Some(h) = &self.credential_helper {
                full.extend(["-c".into(), "credential.helper=".into(), "-c".into(), format!("credential.helper={h}")]);
            }
        }
        full.extend(args.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = full.iter().map(String::as_str).collect();
        let timeout = if network { Duration::from_secs(180) } else { Duration::from_secs(60) };
        let (code, stdout, stderr) = capture(&self.program, &refs, Some(cwd), timeout).await?;
        Ok(GitOutput { command: format!("git {}", args.join(" ")), exit_code: code, stdout, stderr: redact(&stderr) })
    }

    async fn run(&self, cwd: &Path, args: &[&str]) -> Result<GitOutput> {
        let out = self.raw(cwd, args, false).await?;
        check(out)
    }

    async fn run_net(&self, cwd: &Path, args: &[&str]) -> Result<GitOutput> {
        let out = self.raw(cwd, args, true).await?;
        check(out)
    }

    /// True only if `dir` is itself the top level of a Git repository.
    pub async fn is_repo_root(&self, dir: &Path) -> Result<bool> {
        let out = self.raw(dir, &["rev-parse", "--show-toplevel"], false).await?;
        if out.exit_code != 0 {
            return Ok(false);
        }
        let top = PathBuf::from(out.stdout.trim());
        let a = top.canonicalize().unwrap_or(top);
        let b = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        Ok(a == b)
    }

    async fn ensure_repo(&self, dir: &Path) -> Result<()> {
        if self.is_repo_root(dir).await? {
            Ok(())
        } else {
            Err(GeniusError::Git("This project is not a Git repository yet. Initialize one from the Source Control panel.".into()))
        }
    }

    pub async fn init(&self, dir: &Path, branch: &str) -> Result<GitOutput> {
        if self.is_repo_root(dir).await? {
            return Err(GeniusError::Git("A Git repository already exists here.".into()));
        }
        validate_ref_name(branch)?;
        self.run(dir, &["init", "-b", branch]).await
    }

    pub async fn status(&self, dir: &Path) -> Result<RepoStatus> {
        if !self.is_repo_root(dir).await? {
            return Ok(RepoStatus::default());
        }
        let out = self.run(dir, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"]).await?;
        let mut st = parse_status(&out.stdout);
        st.is_repo = true;
        st.merging = dir.join(".git").join("MERGE_HEAD").exists();
        st.remotes = self.remotes(dir).await.unwrap_or_default();
        Ok(st)
    }

    pub async fn remotes(&self, dir: &Path) -> Result<Vec<Remote>> {
        let out = self.run(dir, &["remote", "-v"]).await?;
        let mut v: Vec<Remote> = Vec::new();
        for line in out.stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && !v.iter().any(|r| r.name == parts[0]) {
                v.push(Remote { name: parts[0].into(), url: redact(parts[1]) });
            }
        }
        Ok(v)
    }

    pub async fn diff(&self, dir: &Path, path: &str, staged: bool) -> Result<String> {
        self.ensure_repo(dir).await?;
        let mut args = vec!["diff", "--no-ext-diff", "--no-color"];
        if staged {
            args.push("--cached");
        }
        args.extend(["--", path]);
        Ok(self.run(dir, &args).await?.stdout)
    }

    pub async fn staged_diff(&self, dir: &Path) -> Result<String> {
        self.ensure_repo(dir).await?;
        Ok(self.run(dir, &["diff", "--cached", "--no-ext-diff", "--no-color"]).await?.stdout)
    }

    pub async fn stage(&self, dir: &Path, paths: &[String]) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        if paths.is_empty() {
            return Err(GeniusError::Validation("no files selected".into()));
        }
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(dir, &args).await
    }

    pub async fn unstage(&self, dir: &Path, paths: &[String]) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        if paths.is_empty() {
            return Err(GeniusError::Validation("no files selected".into()));
        }
        let has_head = self.raw(dir, &["rev-parse", "--verify", "HEAD"], false).await?.exit_code == 0;
        let mut args: Vec<&str> = if has_head { vec!["restore", "--staged", "--"] } else { vec!["rm", "--cached", "--quiet", "-r", "--"] };
        args.extend(paths.iter().map(String::as_str));
        self.run(dir, &args).await
    }

    pub async fn staged_paths(&self, dir: &Path) -> Result<Vec<String>> {
        Ok(self.status(dir).await?.files.into_iter().filter(|f| f.staged).map(|f| f.path).collect())
    }

    pub async fn identity(&self, dir: &Path) -> Result<Option<(String, String)>> {
        let n = self.raw(dir, &["config", "user.name"], false).await?;
        let e = self.raw(dir, &["config", "user.email"], false).await?;
        if n.exit_code == 0 && e.exit_code == 0 && !n.stdout.trim().is_empty() && !e.stdout.trim().is_empty() {
            Ok(Some((n.stdout.trim().into(), e.stdout.trim().into())))
        } else {
            Ok(None)
        }
    }

    /// Set the commit identity for this repository only (never global config).
    pub async fn set_local_identity(&self, dir: &Path, name: &str, email: &str) -> Result<()> {
        self.ensure_repo(dir).await?;
        if name.trim().is_empty() || !email.contains('@') {
            return Err(GeniusError::Validation("Provide a name and a valid email for commits.".into()));
        }
        self.run(dir, &["config", "user.name", name.trim()]).await?;
        self.run(dir, &["config", "user.email", email.trim()]).await?;
        Ok(())
    }

    pub async fn commit(&self, dir: &Path, message: &str) -> Result<Commit> {
        self.ensure_repo(dir).await?;
        if message.trim().is_empty() {
            return Err(GeniusError::Validation("Commit message is empty.".into()));
        }
        if self.identity(dir).await?.is_none() {
            return Err(GeniusError::Git(
                "Git does not know who you are. Set your name and email for this repository in the Source Control panel.".into(),
            ));
        }
        if self.staged_paths(dir).await?.is_empty() {
            return Err(GeniusError::Validation("Nothing is staged. Stage the files you want to commit first.".into()));
        }
        self.run(dir, &["commit", "-m", message]).await?;
        self.log(dir, 1).await?.into_iter().next().ok_or_else(|| GeniusError::Git("commit created but could not be read back".into()))
    }

    pub async fn log(&self, dir: &Path, limit: usize) -> Result<Vec<Commit>> {
        self.ensure_repo(dir).await?;
        let has_head = self.raw(dir, &["rev-parse", "--verify", "HEAD"], false).await?.exit_code == 0;
        if !has_head {
            return Ok(vec![]);
        }
        let n = format!("-n{}", limit.clamp(1, 500));
        let out = self.run(dir, &["log", &n, "--pretty=format:%H%x1f%h%x1f%an%x1f%ae%x1f%aI%x1f%s%x1e"]).await?;
        Ok(out
            .stdout
            .split('\x1e')
            .filter_map(|rec| {
                let f: Vec<&str> = rec.trim_matches(['\n', '\r']).split('\x1f').collect();
                (f.len() == 6).then(|| Commit {
                    sha: f[0].into(),
                    short: f[1].into(),
                    author: f[2].into(),
                    email: f[3].into(),
                    date: f[4].into(),
                    subject: f[5].into(),
                })
            })
            .collect())
    }

    pub async fn commit_files(&self, dir: &Path, sha: &str) -> Result<Vec<(String, String)>> {
        validate_sha(sha)?;
        let out = self.run(dir, &["show", "--name-status", "--pretty=format:", "--no-color", sha]).await?;
        Ok(out
            .stdout
            .lines()
            .filter_map(|l| {
                let mut p = l.split('\t');
                let st = p.next()?.trim();
                let path = p.next_back()?.trim();
                (!st.is_empty()).then(|| (st.to_string(), path.to_string()))
            })
            .collect())
    }

    pub async fn commit_diff(&self, dir: &Path, sha: &str, path: Option<&str>) -> Result<String> {
        validate_sha(sha)?;
        let mut args = vec!["show", "--no-color", "--no-ext-diff", "--pretty=format:%H %s%n", sha];
        if let Some(p) = path {
            args.extend(["--", p]);
        }
        Ok(self.run(dir, &args).await?.stdout)
    }

    pub async fn branches(&self, dir: &Path) -> Result<Vec<Branch>> {
        self.ensure_repo(dir).await?;
        let out = self.run(dir, &["branch", "--format=%(refname:short)%1f%(HEAD)%1f%(upstream:short)"]).await?;
        Ok(out
            .stdout
            .lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split('\x1f').collect();
                (f.len() == 3).then(|| Branch {
                    name: f[0].into(),
                    current: f[1] == "*",
                    upstream: (!f[2].is_empty()).then(|| f[2].to_string()),
                })
            })
            .collect())
    }

    pub async fn create_branch(&self, dir: &Path, name: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        validate_ref_name(name)?;
        self.run(dir, &["switch", "-c", name]).await
    }

    /// Switch branches. Git itself refuses if uncommitted changes would be overwritten,
    /// which preserves user work; that refusal is reported verbatim.
    pub async fn switch_branch(&self, dir: &Path, name: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        validate_ref_name(name)?;
        self.run(dir, &["switch", name]).await
    }

    pub async fn set_remote(&self, dir: &Path, name: &str, url: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        validate_ref_name(name)?;
        validate_remote_url(url)?;
        let exists = self.remotes(dir).await?.iter().any(|r| r.name == name);
        if exists {
            self.run(dir, &["remote", "set-url", name, url]).await
        } else {
            self.run(dir, &["remote", "add", name, url]).await
        }
    }

    pub async fn fetch(&self, dir: &Path, remote: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        validate_ref_name(remote)?;
        self.run_net(dir, &["fetch", "--prune", remote]).await
    }

    /// Fast-forward-only pull. If histories diverged, nothing is changed and the caller can
    /// offer an explicit merge.
    pub async fn pull_ff(&self, dir: &Path) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        self.run_net(dir, &["pull", "--ff-only"]).await
    }

    /// Explicitly requested merge pull (never rebase). Conflicts are left for resolution.
    pub async fn pull_merge(&self, dir: &Path) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        let out = self.raw(dir, &["pull", "--no-rebase", "--no-edit"], true).await?;
        if out.exit_code != 0 && self.status(dir).await?.files.iter().any(|f| f.conflicted) {
            return Err(GeniusError::Conflict(
                "The pull produced merge conflicts. Resolve the conflicted files, then commit the merge.".into(),
            ));
        }
        check(out)
    }

    pub async fn merge_abort(&self, dir: &Path) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        self.run(dir, &["merge", "--abort"]).await
    }

    /// Push the current branch. Never forces. Rejections are classified for the UI.
    pub async fn push(&self, dir: &Path, remote: &str, branch: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        validate_ref_name(remote)?;
        validate_ref_name(branch)?;
        let out = self.raw(dir, &["push", "-u", remote, branch], true).await?;
        if out.exit_code != 0 {
            let e = out.stderr.to_lowercase();
            let hint = if e.contains("non-fast-forward") || e.contains("fetch first") || e.contains("rejected") {
                "The remote has commits you don't have. Pull (and resolve any conflicts) before pushing again. Genius never force-pushes."
            } else if e.contains("authentication") || e.contains("could not read username") || e.contains("permission") || e.contains("403") {
                "GitHub rejected the credentials. Connect your GitHub account in Settings → GitHub, then retry."
            } else if e.contains("repository not found") {
                "The remote repository was not found or you don't have access to it."
            } else {
                "Push failed."
            };
            return Err(GeniusError::Git(format!("{hint}\n\n{}", out.stderr.trim())));
        }
        Ok(out)
    }

    /// Verify the remote branch points at local HEAD (proof that a push actually landed).
    pub async fn verify_remote_head(&self, dir: &Path, remote: &str, branch: &str) -> Result<(String, Option<String>)> {
        let local = self.run(dir, &["rev-parse", "HEAD"]).await?.stdout.trim().to_string();
        let r = format!("refs/heads/{branch}");
        let out = self.run_net(dir, &["ls-remote", remote, &r]).await?;
        let remote_sha = out.stdout.split_whitespace().next().map(String::from);
        Ok((local, remote_sha))
    }

    pub async fn current_branch(&self, dir: &Path) -> Result<Option<String>> {
        Ok(self.status(dir).await?.branch)
    }

    pub async fn ls_files_for_publish(&self, dir: &Path) -> Result<Vec<String>> {
        self.ensure_repo(dir).await?;
        let out = self.run(dir, &["ls-files", "--cached", "--others", "--exclude-standard", "-z"]).await?;
        Ok(out.stdout.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
    }

    /// Resolve a conflicted file by taking one side, then stage it.
    pub async fn resolve_side(&self, dir: &Path, path: &str, side: &str) -> Result<GitOutput> {
        self.ensure_repo(dir).await?;
        let flag = match side {
            "ours" => "--ours",
            "theirs" => "--theirs",
            _ => return Err(GeniusError::Validation("side must be 'ours' or 'theirs'".into())),
        };
        self.run(dir, &["checkout", flag, "--", path]).await?;
        self.run(dir, &["add", "--", path]).await
    }

    /// Publish the contents of `content` (e.g. a build's `dist/`) as a single commit on the
    /// remote's [`PAGES_BRANCH`], replacing what was there. Runs in a fresh repository at
    /// `scratch` (which must not exist; the caller removes it), so the user's repository,
    /// branches and working tree are never touched. Returns the pushed commit, verified
    /// against the remote.
    pub async fn push_pages_snapshot(&self, scratch: &Path, content: &Path, remote_url: &str, message: &str, identity: (&str, &str)) -> Result<String> {
        let local_remote = Path::new(remote_url).is_absolute() && Path::new(remote_url).is_dir(); // tests: a local bare repo
        if !local_remote {
            validate_remote_url(remote_url)?;
        }
        if scratch.exists() {
            return Err(GeniusError::Internal("pages scratch directory already exists".into()));
        }
        if !content.join("index.html").is_file() {
            return Err(GeniusError::Validation(format!("{} has no index.html to publish", content.display())));
        }
        std::fs::create_dir_all(scratch)?;
        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).collect::<String>();
        // Explicit --git-dir/--work-tree: no repository discovery from `content`'s parents.
        let gd = format!("--git-dir={}", scratch.join(".git").to_string_lossy());
        let wt = format!("--work-tree={}", content.to_string_lossy());
        let (name, email) = (format!("user.name={}", clean(identity.0)), format!("user.email={}", clean(identity.1)));
        self.run(scratch, &["init", "-q", "-b", PAGES_BRANCH]).await?;
        self.run(content, &["-c", "core.autocrlf=false", &gd, &wt, "add", "-A"]).await?;
        self.run(content, &["-c", &name, "-c", &email, &gd, &wt, "commit", "-q", "--no-verify", "-m", message]).await?;
        let refspec = format!("HEAD:refs/heads/{PAGES_BRANCH}");
        let out = self.raw(scratch, &["push", "--force", remote_url, &refspec], true).await?;
        if out.exit_code != 0 {
            let e = out.stderr.to_lowercase();
            let hint = if e.contains("authentication") || e.contains("could not read username") || e.contains("permission") || e.contains("403") {
                "GitHub rejected the credentials. Connect your GitHub account in Settings → GitHub, then retry."
            } else if e.contains("repository not found") {
                "The GitHub repository was not found or you don't have access to it."
            } else {
                "Uploading the website failed."
            };
            return Err(GeniusError::Git(format!("{hint}\n\n{}", out.stderr.trim())));
        }
        let local = self.run(scratch, &["rev-parse", "HEAD"]).await?.stdout.trim().to_string();
        let r = format!("refs/heads/{PAGES_BRANCH}");
        let remote = self.run_net(scratch, &["ls-remote", remote_url, &r]).await?.stdout.split_whitespace().next().map(String::from);
        if remote.as_deref() != Some(local.as_str()) {
            return Err(GeniusError::Git(format!("The upload finished but {PAGES_BRANCH} on GitHub is at {} instead of {local}.", remote.as_deref().unwrap_or("(missing)"))));
        }
        Ok(local)
    }

    pub async fn clone_repo(&self, parent: &Path, url: &str, dir_name: &str) -> Result<GitOutput> {
        validate_remote_url(url)?;
        crate::workspace::normalize_rel(dir_name)?;
        self.run_net(parent, &["clone", "--", url, dir_name]).await
    }
}

fn check(out: GitOutput) -> Result<GitOutput> {
    if out.exit_code == 0 {
        Ok(out)
    } else {
        let msg = if out.stderr.trim().is_empty() { out.stdout.trim().to_string() } else { out.stderr.trim().to_string() };
        Err(GeniusError::Git(format!("`{}` failed (exit {}): {}", out.command, out.exit_code, msg)))
    }
}

pub fn validate_ref_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() > 200
        || name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
        || name.ends_with('.')
        || name.contains("..")
        || name.contains("@{")
        || name.chars().any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c));
    if bad {
        Err(GeniusError::Validation(format!("'{name}' is not a valid Git name")))
    } else {
        Ok(())
    }
}

fn validate_sha(sha: &str) -> Result<()> {
    if (4..=64).contains(&sha.len()) && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(GeniusError::Validation("invalid commit id".into()))
    }
}

pub fn validate_remote_url(url: &str) -> Result<()> {
    let u = url.trim();
    let ok = (u.starts_with("https://") || u.starts_with("git@") || u.starts_with("ssh://"))
        && !u.starts_with('-')
        && !u.chars().any(|c| c.is_whitespace() || c.is_control())
        && !u.contains("::");
    if !ok {
        return Err(GeniusError::Validation("Remote URL must be an https://, ssh:// or git@ URL.".into()));
    }
    if u.starts_with("https://") && u["https://".len()..].split('/').next().is_some_and(|host| host.contains('@')) {
        return Err(GeniusError::Validation("Remote URLs must not contain embedded credentials.".into()));
    }
    Ok(())
}

pub fn parse_status(raw: &str) -> RepoStatus {
    let mut st = RepoStatus::default();
    let mut parts = raw.split('\0').peekable();
    while let Some(entry) = parts.next() {
        if entry.is_empty() {
            continue;
        }
        if let Some(h) = entry.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" => st.head = (v != "(initial)").then(|| v.to_string()),
                "branch.head" => st.branch = (v != "(detached)").then(|| v.to_string()),
                "branch.upstream" => st.upstream = Some(v.to_string()),
                "branch.ab" => {
                    for t in v.split_whitespace() {
                        if let Some(a) = t.strip_prefix('+') {
                            st.ahead = a.parse().unwrap_or(0);
                        } else if let Some(b) = t.strip_prefix('-') {
                            st.behind = b.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = entry.chars().next().unwrap_or(' ');
        match kind {
            '1' | '2' | 'u' => {
                let n = match kind {
                    '1' => 8,
                    '2' => 9,
                    _ => 10,
                };
                let fields: Vec<&str> = entry.splitn(n + 1, ' ').collect();
                if fields.len() < n + 1 {
                    continue;
                }
                let xy = fields[1];
                let x = &xy[0..1];
                let y = &xy[1..2];
                let orig = if kind == '2' { parts.next().map(String::from) } else { None };
                st.files.push(FileStatus {
                    path: fields[n].to_string(),
                    orig_path: orig,
                    index: x.into(),
                    worktree: y.into(),
                    staged: kind != 'u' && x != ".",
                    unstaged: kind != 'u' && y != ".",
                    untracked: false,
                    conflicted: kind == 'u',
                });
            }
            '?' => st.files.push(FileStatus {
                path: entry[2..].to_string(),
                index: "?".into(),
                worktree: "?".into(),
                unstaged: true,
                untracked: true,
                ..Default::default()
            }),
            _ => {}
        }
    }
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2() {
        let raw = "# branch.oid abc123\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 h1 h2 src/App.tsx\0\
1 A. N... 000000 100644 100644 h1 h2 src/new file.ts\0\
2 R. N... 100644 100644 100644 h1 h2 R100 src/b.ts\0src/a.ts\0\
u UU N... 100644 100644 100644 100644 h1 h2 h3 conflict.ts\0\
? notes.md\0";
        let st = parse_status(raw);
        assert_eq!(st.branch.as_deref(), Some("main"));
        assert_eq!((st.ahead, st.behind), (2, 1));
        assert_eq!(st.files.len(), 5);
        assert!(st.files[0].unstaged && !st.files[0].staged);
        assert_eq!(st.files[1].path, "src/new file.ts");
        assert!(st.files[1].staged);
        assert_eq!(st.files[2].orig_path.as_deref(), Some("src/a.ts"));
        assert!(st.files[3].conflicted);
        assert!(st.files[4].untracked);
    }

    #[test]
    fn rejects_dangerous_names_and_urls() {
        assert!(validate_ref_name("--force").is_err());
        assert!(validate_ref_name("feature/x").is_ok());
        assert!(validate_ref_name("a..b").is_err());
        assert!(validate_remote_url("https://github.com/a/b.git").is_ok());
        assert!(validate_remote_url("git@github.com:a/b.git").is_ok());
        assert!(validate_remote_url("ext::sh -c touch% /tmp/pwned").is_err());
        assert!(validate_remote_url("--upload-pack=evil").is_err());
        assert!(validate_remote_url("https://user:tok@github.com/a/b").is_err());
        assert!(validate_remote_url("file:///etc").is_err());
    }

    async fn repo() -> (tempfile::TempDir, Git, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("proj");
        std::fs::create_dir_all(&dir).unwrap();
        let g = Git::detect().unwrap();
        g.init(&dir, "main").await.unwrap();
        g.set_local_identity(&dir, "Test User", "test@example.com").await.unwrap();
        (d, g, dir)
    }

    #[tokio::test]
    async fn real_repo_workflow() {
        let (_d, g, dir) = repo().await;
        assert!(g.is_repo_root(&dir).await.unwrap());
        std::fs::write(dir.join("a.txt"), "one\n").unwrap();
        std::fs::write(dir.join("user-notes.txt"), "mine\n").unwrap();
        let st = g.status(&dir).await.unwrap();
        assert_eq!(st.files.len(), 2);
        assert!(st.files.iter().all(|f| f.untracked));

        // commit only selected file; unrelated user file stays uncommitted
        g.stage(&dir, &["a.txt".into()]).await.unwrap();
        assert!(g.commit(&dir, "").await.is_err());
        let c = g.commit(&dir, "Add a").await.unwrap();
        assert_eq!(c.subject, "Add a");
        let st = g.status(&dir).await.unwrap();
        assert_eq!(st.files.len(), 1);
        assert_eq!(st.files[0].path, "user-notes.txt");

        std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
        let d = g.diff(&dir, "a.txt", false).await.unwrap();
        assert!(d.contains("+two"));
        g.stage(&dir, &["a.txt".into()]).await.unwrap();
        assert!(g.diff(&dir, "a.txt", true).await.unwrap().contains("+two"));
        g.unstage(&dir, &["a.txt".into()]).await.unwrap();
        assert!(g.status(&dir).await.unwrap().files.iter().any(|f| f.path == "a.txt" && !f.staged && f.unstaged));

        let log = g.log(&dir, 10).await.unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(g.commit_files(&dir, &log[0].sha).await.unwrap(), vec![("A".to_string(), "a.txt".to_string())]);

        g.create_branch(&dir, "feature/x").await.unwrap();
        let b = g.branches(&dir).await.unwrap();
        assert!(b.iter().any(|b| b.name == "feature/x" && b.current));
        assert!(g.create_branch(&dir, "-bad").await.is_err());
    }

    #[tokio::test]
    async fn nested_project_does_not_touch_parent_repo() {
        let (_d, g, dir) = repo().await;
        let child = dir.join("child");
        std::fs::create_dir_all(&child).unwrap();
        assert!(!g.is_repo_root(&child).await.unwrap());
        let st = g.status(&child).await.unwrap();
        assert!(!st.is_repo);
        assert!(g.stage(&child, &["x".into()]).await.is_err());
    }

    #[tokio::test]
    async fn push_to_local_bare_remote_and_verify() {
        let (d, g, dir) = repo().await;
        let bare = d.path().join("remote.git");
        let out = capture(&g.program, &["init", "--bare", "-b", "main", bare.to_str().unwrap()], None, Duration::from_secs(20)).await.unwrap();
        assert_eq!(out.0, 0);
        std::fs::write(dir.join("a.txt"), "x\n").unwrap();
        g.stage(&dir, &["a.txt".into()]).await.unwrap();
        g.commit(&dir, "init").await.unwrap();
        // local path remotes are not allowed by validate_remote_url; add directly for the test
        g.run(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]).await.unwrap();
        g.push(&dir, "origin", "main").await.unwrap();
        let (local, remote) = g.verify_remote_head(&dir, "origin", "main").await.unwrap();
        assert_eq!(Some(local), remote);

        // a diverged remote causes a classified rejection, never a force push
        let other = d.path().join("other");
        g.clone_repo_unchecked(d.path(), bare.to_str().unwrap(), "other").await;
        g.set_local_identity(&other, "O", "o@example.com").await.unwrap();
        std::fs::write(other.join("b.txt"), "b\n").unwrap();
        g.stage(&other, &["b.txt".into()]).await.unwrap();
        g.commit(&other, "remote change").await.unwrap();
        g.push(&other, "origin", "main").await.unwrap();
        std::fs::write(dir.join("c.txt"), "c\n").unwrap();
        g.stage(&dir, &["c.txt".into()]).await.unwrap();
        g.commit(&dir, "local change").await.unwrap();
        let err = g.push(&dir, "origin", "main").await.unwrap_err().to_string();
        assert!(err.contains("Pull"), "{err}");
    }

    #[tokio::test]
    async fn pages_snapshot_replaces_branch_without_touching_project() {
        let (d, g, dir) = repo().await;
        std::fs::write(dir.join("a.txt"), "x\n").unwrap();
        g.stage(&dir, &["a.txt".into()]).await.unwrap();
        let head = g.commit(&dir, "init").await.unwrap().sha;
        let bare = d.path().join("remote.git");
        assert_eq!(capture(&g.program, &["init", "--bare", "-b", "main", bare.to_str().unwrap()], None, Duration::from_secs(20)).await.unwrap().0, 0);
        g.run(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]).await.unwrap();
        g.push(&dir, "origin", "main").await.unwrap();

        let dist = dir.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), "<div id=root></div>").unwrap();
        std::fs::write(dist.join("assets/app.js"), "1").unwrap();
        let remote = bare.to_str().unwrap();
        assert!(g.push_pages_snapshot(&d.path().join("s0"), &dir, remote, "x", ("G", "g@x")).await.is_err(), "needs an index.html");
        let first = g.push_pages_snapshot(&d.path().join("s1"), &dist, remote, "Deploy 1", ("Genius", "g@example.com")).await.unwrap();
        std::fs::remove_file(dist.join("assets/app.js")).unwrap();
        std::fs::write(dist.join("assets/app2.js"), "2").unwrap();
        let second = g.push_pages_snapshot(&d.path().join("s2"), &dist, remote, "Deploy 2", ("Genius", "g@example.com")).await.unwrap();
        assert_ne!(first, second);
        assert!(g.push_pages_snapshot(&d.path().join("s2"), &dist, remote, "again", ("G", "g@x")).await.is_err(), "scratch must be fresh");

        let tree = g.run(&dir, &["ls-tree", "-r", "--name-only", &second]).await;
        assert!(tree.is_err(), "pages commit is not in the project repository");
        let files = capture(&g.program, &["--git-dir", remote, "ls-tree", "-r", "--name-only", PAGES_BRANCH], None, Duration::from_secs(20)).await.unwrap().1;
        assert_eq!(files.lines().collect::<Vec<_>>(), vec!["assets/app2.js", "index.html"], "branch replaced by the new build");
        let log = capture(&g.program, &["--git-dir", remote, "rev-list", "--count", PAGES_BRANCH], None, Duration::from_secs(20)).await.unwrap().1;
        assert_eq!(log.trim(), "1", "one snapshot commit, no history");
        let main = capture(&g.program, &["--git-dir", remote, "rev-parse", "main"], None, Duration::from_secs(20)).await.unwrap().1;
        assert_eq!(main.trim(), head, "main untouched on the remote");
        let st = g.status(&dir).await.unwrap();
        assert_eq!((st.branch.as_deref(), st.head.as_deref()), (Some("main"), Some(head.as_str())), "local branch and HEAD untouched");
        assert!(!g.branches(&dir).await.unwrap().iter().any(|b| b.name == PAGES_BRANCH));
        assert!(g.push_pages_snapshot(&d.path().join("s3"), &dist, "ext::sh -c x", "x", ("G", "g@x")).await.is_err());
    }

    impl Git {
        async fn clone_repo_unchecked(&self, parent: &Path, url: &str, name: &str) {
            self.run(parent, &["clone", url, name]).await.unwrap();
        }
    }
}
