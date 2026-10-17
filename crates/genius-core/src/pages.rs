//! One-click website deploy to GitHub Pages.
//!
//! Build (the project's own `build` script, gated like any build, with `--base=/<repo>/`
//! passed on the command line so vite.config stays untouched) → copy `index.html` to
//! `404.html` for client-side routing → push `dist/` as a single commit to the Genius-owned
//! `gh-pages` branch from a throwaway repository (approval: `git_push` policy) → point
//! GitHub Pages at that branch through `gh api`. The project must already be published to
//! GitHub; its `origin` remote names the repository. Progress and the live URL are kept in
//! the project status under `deploy`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use crate::app::{Genius, TaskInfo};
use crate::db::now;
use crate::error::{GeniusError, Result};
use crate::events::{EventKind, EventStatus};
use crate::git::{Git, PAGES_BRANCH};
use crate::github::{validate_full_name, GitHub};
use crate::process::Operation;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeployStatus {
    /// "building" | "uploading" | "enabling" | "deployed" | "failed"
    pub step: String,
    pub repo: Option<String>,
    /// Live site of the last successful deploy (kept when a redeploy fails).
    pub url: Option<String>,
    pub head: Option<String>,
    pub at: String,
    pub deployed_at: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub running: bool,
}

/// `owner/name` from a GitHub remote URL (https, ssh or scp-style).
pub fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let u = url.trim();
    let rest = ["https://github.com/", "ssh://git@github.com/", "git@github.com:"].iter().find_map(|p| u.strip_prefix(p))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (o, n) = rest.split_once('/')?;
    let full = format!("{o}/{n}");
    validate_full_name(&full).ok()?;
    Some((o.to_string(), n.to_string()))
}

/// Vite `base` and public URL of the Pages site. `<owner>.github.io` repositories are user
/// sites served at the domain root; every other repository is a project site under `/<repo>/`.
pub fn pages_target(owner: &str, repo: &str) -> (String, String) {
    let host = format!("{}.github.io", owner.to_lowercase());
    if repo.eq_ignore_ascii_case(&host) {
        ("/".into(), format!("https://{host}/"))
    } else {
        (format!("/{repo}/"), format!("https://{host}/{repo}/"))
    }
}

/// Make a Vite build servable by GitHub Pages: `404.html` (a copy of `index.html`, so deep
/// links into a client-side router still load the app) and `.nojekyll` (serve `_`-prefixed
/// files as-is). Returns the number of files that will be published.
pub fn prepare_dist(dist: &Path) -> Result<usize> {
    let index = dist.join("index.html");
    if !index.is_file() {
        return Err(GeniusError::Validation("The build did not produce dist/index.html. Genius deploys the standard Vite output folder (dist/).".into()));
    }
    std::fs::copy(&index, dist.join("404.html"))?;
    std::fs::write(dist.join(".nojekyll"), "")?;
    Ok(walkdir::WalkDir::new(dist).into_iter().filter_map(|e| e.ok()).filter(|e| e.file_type().is_file()).count())
}

impl Genius {
    pub fn start_deploy_pages(self: &Arc<Self>, project_id: &str) -> Result<TaskInfo> {
        self.db.get_project(project_id)?;
        let me = self.clone();
        let pid = project_id.to_string();
        self.spawn_task(project_id, "deploy", "Deploy website to GitHub Pages", move |tid, cancel| async move { me.deploy_pages(&pid, Some(&tid), &cancel).await })
    }

    pub fn deploy_status(&self, project_id: &str) -> Result<Option<DeployStatus>> {
        let p = self.db.get_project(project_id)?;
        let Some(mut st) = p.status.get("deploy").and_then(|v| serde_json::from_value::<DeployStatus>(v.clone()).ok()) else { return Ok(None) };
        st.running = self.active_tasks(Some(project_id)).iter().any(|t| t.kind == "deploy");
        if !st.running && !matches!(st.step.as_str(), "deployed" | "failed") {
            st.step = "failed".into();
            st.error = Some("Interrupted before it finished. Deploy again.".into());
        }
        Ok(Some(st))
    }

    fn deploy_step(&self, project_id: &str, step: &str, patch: Value) -> Result<()> {
        let mut cur = self.db.get_project(project_id)?.status.get("deploy").cloned().unwrap_or_else(|| json!({}));
        cur["step"] = json!(step);
        cur["at"] = json!(now());
        cur["error"] = Value::Null;
        if let (Some(o), Some(p)) = (cur.as_object_mut(), patch.as_object()) {
            o.extend(p.clone());
        }
        self.db.patch_project_status(project_id, &json!({ "deploy": cur }))?;
        Ok(())
    }

    /// Build and publish the project's website. See the module docs for the steps.
    pub async fn deploy_pages(&self, project_id: &str, task_id: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
        let res = self.deploy_inner(project_id, task_id, cancel).await;
        if let Err(e) = &res {
            let _ = self.deploy_step(project_id, "failed", json!({ "error": e.to_string() }));
        }
        res
    }

    async fn deploy_inner(&self, project_id: &str, task_id: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
        let ws = self.workspace(project_id)?;
        let root = ws.root();
        let gh = GitHub::detect().ok_or_else(|| GeniusError::MissingTool("GitHub CLI (gh) is not installed. Install it from https://cli.github.com, then restart Genius.".into()))?;
        let git = Git::detect()?.with_gh_helper(Some(gh.path()));
        let origin = if git.is_repo_root(root).await? { git.remotes(root).await?.into_iter().find(|r| r.name == "origin").map(|r| r.url) } else { None };
        let (owner, name) = origin.as_deref().and_then(parse_github_remote).ok_or_else(|| {
            GeniusError::Validation("Publish this project to GitHub first (Publish → Publish to GitHub). The website is deployed from that repository.".into())
        })?;
        let full = format!("{owner}/{name}");
        let auth = gh.status().await;
        if !auth.connected {
            return Err(GeniusError::GitHub("Connect your GitHub account first (Settings → GitHub).".into()));
        }
        let (base, site) = pages_target(&owner, &name);
        let mut ev = self.bus.start(Some(project_id), task_id, EventKind::Github, EventStatus::Running, format!("Deploying website to {site}"), json!({ "repo": full, "branch": PAGES_BRANCH, "base": base }));
        let result: Result<Value> = async {
            // 1. Build for the Pages sub-path (same approvals as any build)
            self.deploy_step(project_id, "building", json!({ "repo": full }))?;
            if self.needs_install(project_id)? {
                let out = self.run_operation(project_id, task_id, Operation::Install, "Dependencies are needed to build the website", false, cancel).await?;
                if !out.ok() {
                    return Err(GeniusError::Process("Installing dependencies failed, so the website could not be built. See the terminal output.".into()));
                }
            }
            self.gate_operation(project_id, task_id, Operation::Build, true, cancel).await?;
            let out = self.procs.run_build_with_base(project_id, task_id, root, &base, &format!("Build the website for {site}"), cancel).await?;
            match out.status.as_str() {
                "succeeded" => {}
                "cancelled" => return Err(GeniusError::Cancelled),
                _ => {
                    return Err(GeniusError::Process(format!(
                        "The production build failed (exit code {}), so nothing was deployed. Fix the build first (Fix it for me / Build Doctor), then deploy again.",
                        out.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "none".into())
                    )))
                }
            }
            let dist = root.join("dist");
            let files = prepare_dist(&dist)?;

            // 2. Upload dist/ to gh-pages (approval per git_push policy)
            self.deploy_step(project_id, "uploading", json!({}))?;
            let remote = format!("https://github.com/{full}.git");
            let policy = self.settings()?.approvals.git_push;
            self.approvals
                .request(&self.bus, Some(project_id), task_id, "git_push", policy, &format!("Deploy website: replace branch {PAGES_BRANCH} of {full} with the new build"), json!({
                    "remote": remote, "branch": PAGES_BRANCH, "files": files, "url": site,
                    "command": format!("git push --force {remote} HEAD:{PAGES_BRANCH}"),
                    "note": format!("Pushed from a temporary repository containing only dist/. Your branches and files are not touched; {PAGES_BRANCH} is replaced on every deploy."),
                }), cancel)
                .await?;
            let identity = match git.identity(root).await? {
                Some((n, e)) => (n, e),
                None => auth.login.as_deref().map(|l| (l.to_string(), format!("{l}@users.noreply.github.com"))).unwrap_or_else(|| ("Genius".into(), "genius@users.noreply.github.com".into())),
            };
            let scratch = self.data_dir().join("tmp").join(format!("pages-{}", uuid::Uuid::new_v4()));
            let pushed = git.push_pages_snapshot(&scratch, &dist, &remote, &format!("Deploy website ({})", now()), (&identity.0, &identity.1)).await;
            let _ = std::fs::remove_dir_all(&scratch);
            let head = pushed?;

            // 3. Serve the branch with GitHub Pages
            self.deploy_step(project_id, "enabling", json!({ "head": head }))?;
            let url = gh.enable_pages(&full, PAGES_BRANCH).await?.map(|u| if u.ends_with('/') { u } else { format!("{u}/") }).unwrap_or_else(|| site.clone());
            let at = now();
            self.deploy_step(project_id, "deployed", json!({ "url": url, "head": head, "deployedAt": at }))?;
            Ok(json!({ "url": url, "repo": full, "branch": PAGES_BRANCH, "head": head, "files": files }))
        }
        .await;
        match &result {
            Ok(v) => {
                ev.set_title(format!("Website deployed to {}", v["url"].as_str().unwrap_or(&site)));
                ev.update(EventStatus::Completed, v.clone());
            }
            Err(e) => ev.update(if matches!(e, GeniusError::Cancelled | GeniusError::ApprovalDenied(_)) { EventStatus::Cancelled } else { EventStatus::Failed }, json!({ "error": { "kind": e.kind(), "message": e.to_string() } })),
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_remotes() {
        let me = Some(("me".to_string(), "my-app".to_string()));
        assert_eq!(parse_github_remote("https://github.com/me/my-app.git"), me);
        assert_eq!(parse_github_remote("https://github.com/me/my-app"), me);
        assert_eq!(parse_github_remote("git@github.com:me/my-app.git"), me);
        assert_eq!(parse_github_remote("ssh://git@github.com/me/my-app.git"), me);
        assert_eq!(parse_github_remote("https://gitlab.com/me/my-app.git"), None);
        assert_eq!(parse_github_remote("https://github.com/me/../x"), None);
        assert_eq!(parse_github_remote("https://github.com/me"), None);
    }

    #[test]
    fn derives_base_and_url() {
        assert_eq!(pages_target("Me", "my-app"), ("/my-app/".into(), "https://me.github.io/my-app/".into()));
        assert_eq!(pages_target("Me", "me.github.io"), ("/".into(), "https://me.github.io/".into()));
        for (base, _) in [pages_target("me", "my_app.v2"), pages_target("me", "me.github.io")] {
            assert!(crate::process::build_args_with_base(&base).is_ok(), "{base}");
        }
    }

    #[test]
    fn prepares_dist_for_spa_routing() {
        let d = tempfile::tempdir().unwrap();
        assert!(prepare_dist(d.path()).is_err(), "no index.html");
        std::fs::create_dir_all(d.path().join("assets")).unwrap();
        std::fs::write(d.path().join("index.html"), "<div id=root></div>").unwrap();
        std::fs::write(d.path().join("assets/a.js"), "x").unwrap();
        assert_eq!(prepare_dist(d.path()).unwrap(), 4);
        assert_eq!(std::fs::read_to_string(d.path().join("404.html")).unwrap(), "<div id=root></div>");
        assert!(d.path().join(".nojekyll").is_file());
        assert_eq!(prepare_dist(d.path()).unwrap(), 4, "idempotent");
    }

    #[tokio::test]
    async fn status_requires_published_project_and_reports_interruptions() {
        let d = tempfile::tempdir().unwrap();
        let g = Genius::new(d.path(), Arc::new(crate::events::NullEmitter), Arc::new(crate::secrets::MemoryStore::default())).unwrap();
        let p = g.create_project("Site", "").unwrap();
        assert_eq!(g.deploy_status(&p.id).unwrap(), None);
        if GitHub::detect().is_some() && Git::detect().is_ok() {
            let e = g.deploy_pages(&p.id, None, &CancellationToken::new()).await.unwrap_err();
            assert!(e.to_string().contains("Publish this project to GitHub first"), "{e}");
            let st = g.deploy_status(&p.id).unwrap().unwrap();
            assert_eq!(st.step, "failed");
        }
        g.deploy_step(&p.id, "uploading", json!({ "repo": "me/site", "url": "https://me.github.io/site/" })).unwrap();
        let st = g.deploy_status(&p.id).unwrap().unwrap();
        assert_eq!((st.step.as_str(), st.running), ("failed", false), "no task running: shown as interrupted");
        assert_eq!(st.url.as_deref(), Some("https://me.github.io/site/"), "last live URL kept");
    }
}
