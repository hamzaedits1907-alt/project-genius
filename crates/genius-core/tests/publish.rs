//! Publish-to-GitHub workflow test (Unix).
//!
//! github.com is not reachable from CI, so:
//!  * a fake `gh` executable (first on PATH) implements the handful of gh commands Genius
//!    uses, creating bare repositories in a temp "GitHub" directory;
//!  * git's `url.<dir>/.insteadOf https://github.com/` redirects pushes and ls-remote to
//!    those bare repositories, so the real git push + verification code runs unchanged.
//!
//! Verifies: secret findings block publishing, a denied push approval leaves a resumable
//! state, a retry reuses the already-created repository (no duplicate), the push is
//! verified against the remote, and the project status records the result.
#![cfg(unix)]

use genius_core::app::Genius;
use genius_core::events::NullEmitter;
use genius_core::secrets::MemoryStore;
use genius_core::settings::Policy;
use genius_core::vcs::PublishRequest;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const FAKE_GH: &str = r#"#!/bin/sh
STATE="$FAKE_GH_STATE"
case "$1 $2" in
  "--version "*) echo "gh version 0.0.0-fake"; exit 0;;
  "auth status") echo "github.com"; echo "  ✓ Logged in to github.com account tester"; echo "  - Token scopes: 'repo', 'read:org'"; exit 0;;
  "api user") echo "tester"; exit 0;;
  "api user/orgs") exit 0;;
  "repo view")
    if [ -d "$STATE/$3.git" ]; then
      name=$(basename "$3")
      printf '{"nameWithOwner":"%s","name":"%s","owner":{"login":"tester"},"visibility":"PRIVATE","url":"https://github.com/%s","description":"","updatedAt":null,"defaultBranchRef":null,"isEmpty":true}\n' "$3" "$name" "$3"
      exit 0
    fi
    echo "GraphQL: Could not resolve to a Repository with the name '$3'." >&2; exit 1;;
  "repo create")
    mkdir -p "$STATE/$(dirname "$3")"
    git init -q --bare -b main "$STATE/$3.git" || exit 1
    echo x >> "$STATE/create-count"
    echo "https://github.com/$3"; exit 0;;
esac
echo "fake gh: unsupported: $*" >&2
exit 2
"#;

#[tokio::test(flavor = "multi_thread")]
async fn publish_is_guarded_resumable_and_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("bin");
    let state = tmp.path().join("github");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    let gh = bin.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    // This test binary contains only this test, so adjusting the process environment is safe.
    std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()));
    std::env::set_var("FAKE_GH_STATE", &state);
    std::env::set_var("GIT_CONFIG_COUNT", "1");
    std::env::set_var("GIT_CONFIG_KEY_0", format!("url.{}/.insteadOf", state.display()));
    std::env::set_var("GIT_CONFIG_VALUE_0", "https://github.com/");

    let g = Genius::new(&tmp.path().join("data"), Arc::new(NullEmitter), Arc::new(MemoryStore::default())).unwrap();
    let p = g.create_project("Publish Me", "").unwrap();
    let ws = g.workspace(&p.id).unwrap();
    ws.write_text("package.json", "{\"name\":\"publish-me\"}\n").unwrap();
    ws.write_text("src/main.ts", "console.log('hi');\n").unwrap();
    ws.write_text("src/config.ts", "export const key = 'nvapi-abcdefghijklmnopqrstuvwxyz0123';\n").unwrap();
    std::fs::write(ws.root().join(".env"), "SECRET=1\n").unwrap();

    let status = g.github_status().await;
    assert!(status.connected, "{}", status.detail);
    assert_eq!(status.login.as_deref(), Some("tester"));

    // Plan shows files and the secret finding; .env is never part of the file list.
    let plan = g.publish_plan(&p.id).await.unwrap();
    assert!(plan.files.contains(&"src/main.ts".to_string()));
    assert!(!plan.files.contains(&".env".to_string()));
    assert!(plan.findings.iter().any(|f| f.path == "src/config.ts"));

    let mut req = PublishRequest {
        owner: "tester".into(),
        name: "publish-me".into(),
        visibility: "private".into(),
        description: "test".into(),
        commit_message: "Initial commit".into(),
        acknowledge_findings: false,
    };
    let err = g.publish(&p.id, &req, &CancellationToken::new()).await.unwrap_err();
    assert_eq!(err.kind(), "conflict", "secrets block publishing: {err}");
    assert!(!state.join("tester/publish-me.git").exists(), "nothing created on GitHub");

    // Remove the secret; deny the push approval after the repo has been created.
    ws.write_text("src/config.ts", "export const key = import.meta.env.VITE_KEY;\n").unwrap();
    req.acknowledge_findings = false;
    let g2 = g.clone();
    let pid = p.id.clone();
    let req2 = req.clone();
    let h = tokio::spawn(async move { g2.publish(&pid, &req2, &CancellationToken::new()).await });
    let approval = loop {
        if let Some(a) = g.approvals.pending(Some(&p.id)).into_iter().next() {
            break a;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert_eq!(approval.category, "git_push");
    g.approvals.respond(&approval.id, false).unwrap();
    let err = h.await.unwrap().unwrap_err();
    assert_eq!(err.kind(), "approval_denied");
    assert!(state.join("tester/publish-me.git").exists(), "repository was created before the push step");
    assert_eq!(g.project(&p.id).unwrap().status["publish"]["step"], "repo_ready", "progress recorded for retry");

    // Retry with push auto-approved: the existing repository is reused, push is verified.
    let mut s = g.settings().unwrap();
    s.approvals.git_push = Policy::Auto;
    g.save_settings(&s).unwrap();
    let out = g.publish(&p.id, &req, &CancellationToken::new()).await.unwrap();
    assert_eq!(out["verified"], true);
    assert_eq!(out["url"], "https://github.com/tester/publish-me");
    let creates = std::fs::read_to_string(state.join("create-count")).unwrap();
    assert_eq!(creates.lines().count(), 1, "retry must not create a duplicate repository");

    // The bare remote really has the commit, and local HEAD matches it.
    let remote_head = std::process::Command::new("git").args(["--git-dir", state.join("tester/publish-me.git").to_str().unwrap(), "rev-parse", "refs/heads/main"]).output().unwrap();
    let local_head = std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(ws.root()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&remote_head.stdout), String::from_utf8_lossy(&local_head.stdout));
    let files = std::process::Command::new("git").args(["--git-dir", state.join("tester/publish-me.git").to_str().unwrap(), "ls-tree", "-r", "--name-only", "main"]).output().unwrap();
    let files = String::from_utf8_lossy(&files.stdout);
    assert!(files.contains("src/main.ts") && files.contains(".gitignore"));
    assert!(!files.contains(".env\n"), "secrets file not published");

    let st = g.project(&p.id).unwrap().status;
    assert_eq!(st["publish"]["step"], "pushed");
    assert_eq!(st["publish"]["verified"], true);
    let gv = g.git_view(&p.id).await.unwrap();
    assert!(gv.status.files.is_empty(), "everything committed");
}
