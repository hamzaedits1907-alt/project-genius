//! Source control and GitHub workflows on top of `git.rs` / `github.rs`, with activity
//! events, approvals, secret scanning and change attribution.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::app::Genius;
use crate::changes::ChangeStore;
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use crate::git::{Commit, Git, GitOutput, RepoStatus};
use crate::github::{validate_repo_name, AuthStatus, GitHub, RepoInfo};
use crate::prompts;
use crate::secretscan::{scan_files, SecretFinding};

/// Minimal entries every project .gitignore should contain.
pub const REQUIRED_IGNORES: &[&str] = &["node_modules", "dist", ".env", ".env.*", "!.env.example", "*.log", ".DS_Store", "coverage"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitView {
    pub available: bool,
    pub error: Option<String>,
    pub status: RepoStatus,
    /// Paths changed by Genius (AI or editor changesets) since the last commit.
    pub genius_paths: Vec<String>,
    pub identity: Option<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishPlan {
    pub git_available: bool,
    pub gh: AuthStatus,
    pub is_repo: bool,
    pub branch: String,
    pub remote_url: Option<String>,
    pub suggested_name: String,
    pub files: Vec<String>,
    pub excluded_by_gitignore: Vec<String>,
    pub findings: Vec<SecretFinding>,
    pub gitignore_missing: Vec<String>,
    pub uncommitted: usize,
    pub previous: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishRequest {
    pub owner: String,
    pub name: String,
    pub visibility: String,
    pub description: String,
    pub commit_message: String,
    /// The user reviewed findings and explicitly chose to continue.
    pub acknowledge_findings: bool,
}

impl Genius {
    fn git(&self) -> Result<Git> {
        let gh = GitHub::detect();
        Ok(Git::detect()?.with_gh_helper(gh.as_ref().map(|g| g.path())))
    }

    fn git_event(&self, project_id: &str, title: &str, res: &Result<GitOutput>) {
        match res {
            Ok(o) => {
                self.bus.record(
                    Some(project_id),
                    None,
                    EventKind::Git,
                    EventStatus::Completed,
                    title,
                    json!({ "command": o.command, "exitCode": o.exit_code, "output": format!("{}{}", o.stdout, o.stderr).chars().take(4000).collect::<String>() }),
                );
            }
            Err(e) => {
                self.bus.record(Some(project_id), None, EventKind::Git, EventStatus::Failed, title, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
            }
        }
    }

    pub async fn git_view(&self, project_id: &str) -> Result<GitView> {
        let ws = self.workspace(project_id)?;
        let git = match self.git() {
            Ok(g) => g,
            Err(e) => return Ok(GitView { available: false, error: Some(e.to_string()), status: RepoStatus::default(), genius_paths: vec![], identity: None }),
        };
        let status = git.status(ws.root()).await?;
        let last_commit = if status.is_repo { git.log(ws.root(), 1).await?.into_iter().next().map(|c| c.date) } else { None };
        let mut genius_paths: Vec<String> = Vec::new();
        for cs in ChangeStore::new(&self.db).list(project_id, 500)? {
            let applied_after = match (&cs.applied_at, &last_commit) {
                (Some(a), Some(c)) => chrono::DateTime::parse_from_rfc3339(a).ok() > chrono::DateTime::parse_from_rfc3339(c).ok(),
                (Some(_), None) => true,
                _ => false,
            };
            if applied_after {
                genius_paths.extend(cs.files.iter().map(|f| f.path.clone()));
            }
        }
        genius_paths.sort();
        genius_paths.dedup();
        let identity = if status.is_repo { git.identity(ws.root()).await? } else { None };
        Ok(GitView { available: true, error: None, status, genius_paths, identity })
    }

    /// Initialise a repository and ensure .gitignore protects dependencies, build output and secrets.
    /// An existing .gitignore is preserved; only missing entries are appended (recorded as a changeset).
    pub async fn git_init(&self, project_id: &str) -> Result<Value> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let res = git.init(ws.root(), "main").await;
        self.git_event(project_id, "Initialized Git repository", &res);
        res?;
        let added = self.ensure_gitignore(project_id)?;
        Ok(json!({ "gitignoreAdded": added }))
    }

    pub fn gitignore_missing(&self, project_id: &str) -> Result<Vec<String>> {
        let ws = self.workspace(project_id)?;
        let current = ws.read_text_opt(".gitignore")?.unwrap_or_default();
        let lines: Vec<&str> = current.lines().map(|l| l.trim().trim_end_matches('/')).collect();
        Ok(REQUIRED_IGNORES.iter().filter(|r| !lines.contains(r) && !lines.contains(&format!("/{r}").as_str())).map(|s| s.to_string()).collect())
    }

    pub fn ensure_gitignore(&self, project_id: &str) -> Result<Vec<String>> {
        let missing = self.gitignore_missing(project_id)?;
        if missing.is_empty() {
            return Ok(missing);
        }
        let ws = self.workspace(project_id)?;
        let mut current = ws.read_text_opt(".gitignore")?.unwrap_or_default();
        if !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        current.push_str("\n# Added by Genius: keep dependencies, build output and secrets out of Git\n");
        for m in &missing {
            current.push_str(m);
            current.push('\n');
        }
        let changes = crate::changes::compute_changes(&ws, &[(".gitignore".into(), current)], &[])?;
        let store = ChangeStore::new(&self.db);
        let cs = store.create(project_id, None, "manual", "Update .gitignore", "Appended missing ignore rules; existing rules preserved", &changes)?;
        store.apply(&ws, &cs.id)?;
        self.bus.record(Some(project_id), None, EventKind::FileWrite, EventStatus::Completed, "Updated .gitignore", json!({ "added": missing, "changesetId": cs.id }));
        Ok(missing)
    }

    pub async fn git_diff(&self, project_id: &str, path: &str, staged: bool, untracked: bool) -> Result<String> {
        let ws = self.workspace(project_id)?;
        if untracked {
            let content = ws.read_text(path).unwrap_or_else(|e| format!("({e})\n"));
            return Ok(crate::changes::unified_diff(path, None, Some(&content)));
        }
        self.git()?.diff(ws.root(), path, staged).await
    }

    pub async fn git_stage(&self, project_id: &str, paths: Vec<String>, stage: bool) -> Result<RepoStatus> {
        let ws = self.workspace(project_id)?;
        for p in &paths {
            crate::workspace::normalize_rel(p)?;
        }
        let git = self.git()?;
        let res = if stage { git.stage(ws.root(), &paths).await } else { git.unstage(ws.root(), &paths).await };
        self.git_event(project_id, &format!("{} {} file(s)", if stage { "Staged" } else { "Unstaged" }, paths.len()), &res);
        res?;
        git.status(ws.root()).await
    }

    /// Scan the staged files for secrets before committing.
    pub async fn staged_findings(&self, project_id: &str) -> Result<Vec<SecretFinding>> {
        let ws = self.workspace(project_id)?;
        let staged = self.git()?.staged_paths(ws.root()).await?;
        Ok(scan_files(&ws, &staged))
    }

    pub async fn git_commit(&self, project_id: &str, message: &str, allow_findings: bool) -> Result<Commit> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let findings = self.staged_findings(project_id).await?;
        if !findings.is_empty() && !allow_findings {
            self.bus.record(Some(project_id), None, EventKind::Git, EventStatus::Failed, "Commit blocked: possible secrets in staged files", json!({ "findings": findings }));
            return Err(GeniusError::Conflict(format!(
                "Possible secrets found in staged files: {}. Unstage them or remove the secrets. If these are false positives, confirm to commit anyway.",
                findings.iter().map(|f| format!("{}{}", f.path, f.line.map(|l| format!(":{l}")).unwrap_or_default())).collect::<Vec<_>>().join(", ")
            )));
        }
        let mut ev = self.bus.start(Some(project_id), None, EventKind::Git, EventStatus::Running, "Creating commit", json!({ "message": message, "findingsAcknowledged": !findings.is_empty() }));
        match git.commit(ws.root(), message).await {
            Ok(c) => {
                ev.set_title(format!("Committed {} {}", c.short, c.subject));
                ev.update(EventStatus::Completed, serde_json::to_value(&c)?);
                Ok(c)
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    pub async fn suggest_commit_message(&self, project_id: &str) -> Result<String> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let staged = git.staged_paths(ws.root()).await?;
        if staged.is_empty() {
            return Err(GeniusError::Validation("Stage files first so Genius can describe them.".into()));
        }
        let diff = git.staged_diff(ws.root()).await?;
        let diff = crate::redact::redact(&crate::diagnostics::focus_log(&diff, 14_000));
        let (r, _) = self.ai_call_simple(project_id, "Writing commit message", &prompts::commit_message_messages(&diff, &staged)).await?;
        Ok(prompts::clean_commit_message(&r))
    }

    pub async fn git_log(&self, project_id: &str, limit: usize) -> Result<Vec<Commit>> {
        let ws = self.workspace(project_id)?;
        self.git()?.log(ws.root(), limit).await
    }

    pub async fn git_commit_detail(&self, project_id: &str, sha: &str) -> Result<Value> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let files = git.commit_files(ws.root(), sha).await?;
        let diff = git.commit_diff(ws.root(), sha, None).await?;
        let diff: String = diff.chars().take(400_000).collect();
        Ok(json!({ "files": files, "diff": diff }))
    }

    pub async fn git_branches(&self, project_id: &str) -> Result<Vec<crate::git::Branch>> {
        let ws = self.workspace(project_id)?;
        self.git()?.branches(ws.root()).await
    }

    pub async fn git_branch(&self, project_id: &str, name: &str, create: bool) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let res = if create { git.create_branch(ws.root(), name).await } else { git.switch_branch(ws.root(), name).await };
        self.git_event(project_id, &format!("{} branch {name}", if create { "Created" } else { "Switched to" }), &res);
        res.map(|_| ())
    }

    pub async fn git_set_identity(&self, project_id: &str, name: &str, email: &str) -> Result<()> {
        let ws = self.workspace(project_id)?;
        self.git()?.set_local_identity(ws.root(), name, email).await
    }

    pub async fn git_set_remote(&self, project_id: &str, name: &str, url: &str) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let res = self.git()?.set_remote(ws.root(), name, url).await;
        self.git_event(project_id, &format!("Configured remote {name}"), &res);
        res.map(|_| ())
    }

    pub async fn git_fetch(&self, project_id: &str) -> Result<RepoStatus> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let res = git.fetch(ws.root(), "origin").await;
        self.git_event(project_id, "Fetched from origin", &res);
        res?;
        git.status(ws.root()).await
    }

    pub async fn git_pull(&self, project_id: &str, allow_merge: bool) -> Result<RepoStatus> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let res = if allow_merge { git.pull_merge(ws.root()).await } else { git.pull_ff(ws.root()).await };
        let title = if allow_merge { "Pulled (merge)" } else { "Pulled (fast-forward only)" };
        self.git_event(project_id, title, &res);
        if let Err(e) = res {
            let msg = e.to_string();
            if !allow_merge && (msg.contains("Not possible to fast-forward") || msg.contains("diverg")) {
                return Err(GeniusError::Conflict(
                    "Local and remote histories have diverged. Choose 'Pull with merge' to combine them (you may need to resolve conflicts). Nothing was changed.".into(),
                ));
            }
            return Err(e);
        }
        self.bus.push(crate::events::channels::FS_CHANGED, json!({ "projectId": project_id, "paths": [] }));
        git.status(ws.root()).await
    }

    pub async fn git_merge_abort(&self, project_id: &str) -> Result<()> {
        let ws = self.workspace(project_id)?;
        let res = self.git()?.merge_abort(ws.root()).await;
        self.git_event(project_id, "Aborted merge", &res);
        self.bus.push(crate::events::channels::FS_CHANGED, json!({ "projectId": project_id, "paths": [] }));
        res.map(|_| ())
    }

    pub async fn git_resolve(&self, project_id: &str, path: &str, side: &str) -> Result<()> {
        let ws = self.workspace(project_id)?;
        crate::workspace::normalize_rel(path)?;
        let res = self.git()?.resolve_side(ws.root(), path, side).await;
        self.git_event(project_id, &format!("Resolved {path} using {side}"), &res);
        self.bus.push(crate::events::channels::FS_CHANGED, json!({ "projectId": project_id, "paths": [path] }));
        res.map(|_| ())
    }

    /// Ask the model to merge a conflicted file; result is a reviewable changeset.
    pub async fn propose_conflict_resolution(&self, project_id: &str, path: &str) -> Result<crate::changes::Changeset> {
        let ws = self.workspace(project_id)?;
        let content = ws.read_text(path)?;
        if !content.contains("<<<<<<<") {
            return Err(GeniusError::Validation(format!("{path} has no conflict markers")));
        }
        let (text, _) = self.ai_call_simple(project_id, &format!("Resolving merge conflict in {path}"), &prompts::conflict_messages(path, &content)).await?;
        let parsed = crate::blocks::collect(&crate::blocks::parse_all(&text));
        let resolved = parsed.files.into_iter().find(|(p, _)| p == path).map(|(_, c)| c).ok_or_else(|| GeniusError::Validation("The model did not return the resolved file.".into()))?;
        if resolved.contains("<<<<<<<") || resolved.contains(">>>>>>>") {
            return Err(GeniusError::Validation("The proposed resolution still contains conflict markers.".into()));
        }
        let changes = crate::changes::compute_changes(&ws, &[(path.to_string(), resolved)], &[])?;
        let cs = ChangeStore::new(&self.db).create(project_id, None, "change", &format!("Resolve conflict in {path}"), &parsed.explanation, &changes)?;
        self.bus.record(Some(project_id), None, EventKind::Changeset, EventStatus::AwaitingApproval, format!("Review proposed changes: {}", cs.title), json!({ "changesetId": cs.id }));
        Ok(cs)
    }

    /// Push with an approval gate (policy `git_push`) and verification against the remote.
    pub async fn git_push(&self, project_id: &str, cancel: &CancellationToken) -> Result<Value> {
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let st = git.status(ws.root()).await?;
        let branch = st.branch.clone().ok_or_else(|| GeniusError::Git("Not on a branch (detached HEAD).".into()))?;
        let remote = st.remotes.iter().find(|r| r.name == "origin").ok_or_else(|| GeniusError::Git("No 'origin' remote is configured.".into()))?;
        let policy = self.settings()?.approvals.git_push;
        self.approvals
            .request(
                &self.bus,
                Some(project_id),
                None,
                "git_push",
                policy,
                &format!("Push branch {branch} to {}", remote.url),
                json!({ "branch": branch, "remote": remote.url, "ahead": st.ahead, "command": format!("git push -u origin {branch}") }),
                cancel,
            )
            .await?;
        self.push_and_verify(project_id, ws.root(), &git, &branch).await
    }

    async fn push_and_verify(&self, project_id: &str, root: &Path, git: &Git, branch: &str) -> Result<Value> {
        let mut ev = self.bus.start(Some(project_id), None, EventKind::Git, EventStatus::Running, format!("Pushing {branch} to origin"), json!({ "command": format!("git push -u origin {branch}") }));
        if let Err(e) = git.push(root, "origin", branch).await {
            ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
            return Err(e);
        }
        let (local, remote) = git.verify_remote_head(root, "origin", branch).await?;
        let verified = Some(&local) == remote.as_ref();
        if verified {
            ev.set_title(format!("Pushed {branch} to origin (verified)"));
            ev.update(EventStatus::Completed, json!({ "localHead": local, "remoteHead": remote, "verified": true }));
            Ok(json!({ "verified": true, "head": local }))
        } else {
            let e = GeniusError::Git(format!("Push finished but the remote branch is at {} while local HEAD is {local}.", remote.as_deref().unwrap_or("(missing)")));
            ev.update(EventStatus::Failed, json!({ "localHead": local, "remoteHead": remote, "verified": false }));
            Err(e)
        }
    }

    async fn ai_call_simple(&self, project_id: &str, purpose: &str, messages: &[crate::nvidia::ChatMessage]) -> Result<(String, ())> {
        let s = self.settings()?;
        let key = self.secrets.get(crate::secrets::NVIDIA_KEY)?;
        let client = crate::nvidia::NvidiaClient::new(&s.base_url, key)?;
        let model = s.model.clone().ok_or(crate::nvidia::ProviderError::NoModel)?;
        let mut ev = self.bus.start(Some(project_id), None, EventKind::Ai, EventStatus::Running, format!("{purpose} ({model})"), json!({ "model": model }));
        let opts = crate::nvidia::ChatOptions { temperature: s.temperature, top_p: 0.95, max_tokens: s.max_tokens.min(8192), stream: true };
        match client.chat(&model, messages, &opts, &CancellationToken::new(), |_| {}).await {
            Ok(r) => {
                ev.update(EventStatus::Completed, json!({ "outputChars": r.content.len() }));
                Ok((r.content, ()))
            }
            Err(e) => {
                let e = GeniusError::from(e);
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    // ===================================================================== GitHub

    pub async fn github_status(&self) -> AuthStatus {
        match GitHub::detect() {
            Some(gh) => gh.status().await,
            None => AuthStatus { detail: "GitHub CLI (gh) is not installed. Install it from https://cli.github.com, then restart Genius.".into(), ..Default::default() },
        }
    }

    fn gh(&self) -> Result<GitHub> {
        GitHub::detect().ok_or_else(|| GeniusError::MissingTool("GitHub CLI (gh) is not installed. Install it from https://cli.github.com, then restart Genius.".into()))
    }

    pub fn github_login_web(self: &Arc<Self>) -> Result<()> {
        let gh = self.gh()?;
        let me = self.clone();
        self.runtime().spawn(async move {
            let mut ev = me.bus.start(None, None, EventKind::Github, EventStatus::Running, "Connecting GitHub account", json!({ "method": "device" }));
            let bus = me.bus.clone();
            let res = gh
                .login_web(move |code| {
                    bus.push("genius://github-device-code", json!({ "code": code, "url": "https://github.com/login/device" }));
                })
                .await;
            match res {
                Ok(()) => {
                    let st = me.github_status().await;
                    ev.update(if st.connected { EventStatus::Completed } else { EventStatus::Failed }, json!({ "login": st.login, "scopes": st.scopes }));
                }
                Err(e) => ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } })),
            }
            me.bus.push("genius://github-status", serde_json::to_value(me.github_status().await).unwrap_or(Value::Null));
        });
        Ok(())
    }

    pub async fn github_login_token(&self, token: &str) -> Result<AuthStatus> {
        let gh = self.gh()?;
        let res = gh.login_with_token(token).await;
        self.bus.record(
            None,
            None,
            EventKind::Github,
            if res.is_ok() { EventStatus::Completed } else { EventStatus::Failed },
            "Connect GitHub with token (stored by GitHub CLI)",
            match &res {
                Ok(_) => json!({}),
                Err(e) => json!({ "error": { "kind": e.kind(), "message": e.to_string() } }),
            },
        );
        res?;
        Ok(gh.status().await)
    }

    pub async fn github_logout(&self) -> Result<AuthStatus> {
        let gh = self.gh()?;
        let res = gh.logout().await;
        self.bus.record(None, None, EventKind::Github, if res.is_ok() { EventStatus::Completed } else { EventStatus::Failed }, "Disconnected GitHub account", json!({}));
        res?;
        Ok(gh.status().await)
    }

    pub async fn github_repos(&self, owner: Option<String>) -> Result<Vec<RepoInfo>> {
        self.gh()?.list_repos(owner.as_deref(), 200).await
    }

    pub async fn github_orgs(&self) -> Result<Vec<String>> {
        self.gh()?.orgs().await
    }

    /// Clone a GitHub repository into the workspace directory and register it as a project.
    pub async fn github_clone(&self, name_with_owner: &str) -> Result<crate::db::ProjectRow> {
        crate::github::validate_full_name(name_with_owner)?;
        let git = self.git()?;
        let base = self.workspace_dir()?;
        std::fs::create_dir_all(&base)?;
        let repo_name = name_with_owner.split('/').nth(1).unwrap_or("repo");
        let mut dir_name = crate::scaffold::slugify(repo_name);
        let mut n = 2;
        while base.join(&dir_name).exists() {
            dir_name = format!("{}-{n}", crate::scaffold::slugify(repo_name));
            n += 1;
        }
        let url = format!("https://github.com/{name_with_owner}.git");
        let mut ev = self.bus.start(None, None, EventKind::Github, EventStatus::Running, format!("Cloning {name_with_owner}"), json!({ "url": url, "into": base.join(&dir_name) }));
        match git.clone_repo(&base, &url, &dir_name).await {
            Ok(_) => {
                ev.update(EventStatus::Completed, json!({}));
                self.import_project(&base.join(&dir_name).to_string_lossy(), Some(repo_name))
            }
            Err(e) => {
                ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                Err(e)
            }
        }
    }

    pub async fn publish_plan(&self, project_id: &str) -> Result<PublishPlan> {
        let p = self.db.get_project(project_id)?;
        let ws = self.workspace(project_id)?;
        let gh = self.github_status().await;
        let git = self.git();
        let git_available = git.is_ok();
        let (is_repo, branch, remote_url, files, uncommitted) = match &git {
            Ok(g) if g.is_repo_root(ws.root()).await? => {
                let st = g.status(ws.root()).await?;
                let files = g.ls_files_for_publish(ws.root()).await?;
                (true, st.branch.clone().unwrap_or_else(|| "main".into()), st.remotes.iter().find(|r| r.name == "origin").map(|r| r.url.clone()), files, st.files.len())
            }
            _ => {
                // Not a repo yet: approximate what will be published using .gitignore rules applied by Genius.
                let files: Vec<String> = ws.list_files()?.into_iter().map(|f| f.path).filter(|f| !crate::workspace::is_secret_name(f.rsplit('/').next().unwrap_or(f))).collect();
                (false, "main".into(), None, files, 0)
            }
        };
        let all: Vec<String> = ws.list_files()?.into_iter().map(|f| f.path).collect();
        let excluded: Vec<String> = all.iter().filter(|f| !files.contains(f)).cloned().collect();
        let findings = scan_files(&ws, &files);
        Ok(PublishPlan {
            git_available,
            gh,
            is_repo,
            branch,
            remote_url,
            suggested_name: crate::scaffold::slugify(&p.name),
            files,
            excluded_by_gitignore: excluded,
            findings,
            gitignore_missing: self.gitignore_missing(project_id)?,
            uncommitted,
            previous: p.status.get("publish").cloned(),
        })
    }

    /// One-click publish. Each step is recorded in project status so a retry resumes
    /// without creating a duplicate repository.
    pub async fn publish(&self, project_id: &str, req: &PublishRequest, cancel: &CancellationToken) -> Result<Value> {
        validate_repo_name(&req.name)?;
        if !["private", "public", "internal"].contains(&req.visibility.as_str()) {
            return Err(GeniusError::Validation("Choose the repository visibility explicitly.".into()));
        }
        let gh = self.gh()?;
        let auth = gh.status().await;
        if !auth.connected {
            return Err(GeniusError::GitHub("Connect your GitHub account first (Settings → GitHub).".into()));
        }
        let ws = self.workspace(project_id)?;
        let git = self.git()?;
        let full = format!("{}/{}", req.owner, req.name);
        let step = |s: &str, extra: Value| -> Result<()> {
            let mut v = json!({ "repo": full, "step": s, "at": crate::db::now(), "visibility": req.visibility });
            if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
                o.extend(e.clone());
            }
            self.db.patch_project_status(project_id, &json!({ "publish": v }))?;
            Ok(())
        };
        let mut task_ev = self.bus.start(
            Some(project_id),
            None,
            EventKind::Github,
            EventStatus::Running,
            format!("Publishing to GitHub: {full} ({})", req.visibility),
            json!({ "repo": full, "visibility": req.visibility }),
        );

        let result: Result<Value> = async {
            // 1. Local repository
            if !git.is_repo_root(ws.root()).await? {
                let r = git.init(ws.root(), "main").await;
                self.git_event(project_id, "Initialized Git repository", &r);
                r?;
            }
            self.ensure_gitignore(project_id)?;
            if git.identity(ws.root()).await?.is_none() {
                if let Some(login) = &auth.login {
                    git.set_local_identity(ws.root(), login, &format!("{login}@users.noreply.github.com")).await?;
                    self.bus.record(Some(project_id), None, EventKind::Git, EventStatus::Completed, format!("Set commit identity for this repository to {login} (GitHub no-reply email)"), json!({}));
                }
            }
            step("local_ready", json!({}))?;

            // 2. Secret scan of everything that will be published
            let files = git.ls_files_for_publish(ws.root()).await?;
            let findings = scan_files(&ws, &files);
            if !findings.is_empty() && !req.acknowledge_findings {
                return Err(GeniusError::Conflict(format!(
                    "Possible secrets in files to publish: {}. Review them before publishing.",
                    findings.iter().map(|f| f.path.clone()).collect::<Vec<_>>().join(", ")
                )));
            }

            // 3. Commit reviewed files (only if there is something to commit)
            let st = git.status(ws.root()).await?;
            if !st.files.is_empty() {
                git.stage(ws.root(), &files).await?;
                let msg = if req.commit_message.trim().is_empty() { "Initial commit".to_string() } else { req.commit_message.clone() };
                let c = git.commit(ws.root(), &msg).await?;
                self.bus.record(Some(project_id), None, EventKind::Git, EventStatus::Completed, format!("Committed {} {}", c.short, c.subject), serde_json::to_value(&c)?);
            }
            if git.log(ws.root(), 1).await?.is_empty() {
                return Err(GeniusError::Git("There are no commits to publish.".into()));
            }
            step("committed", json!({}))?;

            // 4. Remote repository: reuse if it already exists (retry-safe), else create
            let repo = match gh.view(&full).await? {
                Some(r) => {
                    self.bus.record(Some(project_id), None, EventKind::Github, EventStatus::Completed, format!("Using existing repository {full}"), serde_json::to_value(&r)?);
                    r
                }
                None => {
                    let mut ev = self.bus.start(Some(project_id), None, EventKind::Github, EventStatus::Running, format!("Creating {} repository {full}", req.visibility), json!({}));
                    match gh.create_repo(&req.owner, &req.name, &req.visibility, &req.description).await {
                        Ok(r) => {
                            ev.update(EventStatus::Completed, serde_json::to_value(&r)?);
                            r
                        }
                        Err(e) => {
                            ev.update(EventStatus::Failed, json!({ "error": { "kind": e.kind(), "message": e.to_string() } }));
                            return Err(e);
                        }
                    }
                }
            };
            step("repo_ready", json!({ "url": repo.url }))?;
            let remote_url = format!("{}.git", repo.url.trim_end_matches('/'));
            let r = git.set_remote(ws.root(), "origin", &remote_url).await;
            self.git_event(project_id, "Configured remote origin", &r);
            r?;

            // 5. Push (approval per policy) and verify
            let branch = git.current_branch(ws.root()).await?.unwrap_or_else(|| "main".into());
            let policy = self.settings()?.approvals.git_push;
            self.approvals
                .request(
                    &self.bus,
                    Some(project_id),
                    None,
                    "git_push",
                    policy,
                    &format!("Push {branch} to {full}"),
                    json!({ "remote": remote_url, "branch": branch, "visibility": repo.visibility }),
                    cancel,
                )
                .await?;
            let pushed = self.push_and_verify(project_id, ws.root(), &git, &branch).await?;
            step("pushed", json!({ "url": repo.url, "verified": true, "head": pushed["head"] }))?;
            Ok(json!({ "url": repo.url, "repo": full, "branch": branch, "verified": true }))
        }
        .await;

        match &result {
            Ok(v) => task_ev.update(EventStatus::Completed, v.clone()),
            Err(e) => {
                let prev = self.db.get_project(project_id)?.status.get("publish").cloned();
                task_ev.update(
                    if matches!(e, GeniusError::Cancelled | GeniusError::ApprovalDenied(_)) { EventStatus::Cancelled } else { EventStatus::Failed },
                    json!({ "error": { "kind": e.kind(), "message": e.to_string() }, "progress": prev }),
                );
            }
        }
        result
    }
}
