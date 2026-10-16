# Project Genius — Architecture

## Overview

```
┌──────────────────────────── Genius desktop app (Tauri 2) ────────────────────────────┐
│  React + TypeScript UI (src/)                                                         │
│   Home · Create · Workspace (files, editor, drafts, review, plan, assistant, preview,  │
│   activity, terminal, readiness, source control, history, memory) · Settings          │
│        │ invoke (82 commands, ACL-granted to the local main window only)              │
│        ▼                          ▲ events: activity, ai-delta, file-stream,           │
│  src-tauri/ (thin command layer)  │ process-output/state, fs-changed, preview          │
│        │                          │                                                    │
│        ▼                          │                                                    │
│  crates/genius-core (platform-independent Rust)                                        │
│   app.rs ── Genius facade: projects, tasks, AI flows, verify pipeline, preview         │
│   vcs.rs ── Git/GitHub workflows, publish, secret-guarded commits                      │
│   nvidia.rs  provider (models, streaming chat, error classes, retries, cancellation)    │
│   workspace.rs sandboxed FS     changes.rs changesets/diffs/revert/restore              │
│   process.rs allowlisted npm ops, process groups, bounded output                        │
│   git.rs / github.rs  git CLI (porcelain v2) / GitHub CLI                               │
│   events.rs typed activity bus (persisted)   approvals.rs approval gate                 │
│   context.rs context selection   blocks.rs streaming file-block parser                  │
│   spec.rs plan schema   prompts.rs model contracts   diagnostics.rs build-log parsing   │
│   secrets.rs OS keyring   redact.rs   secretscan.rs   export.rs   inspector.rs          │
│   db.rs SQLite + versioned migrations   settings.rs                                     │
└───────────────────────────────────────────────────────────────────────────────────────┘
        │ HTTPS (Bearer key, backend only)        │ child processes (no shell)
        ▼                                         ▼
  integrate.api.nvidia.com/v1            npm · git · gh   in the project folder
```

The **core crate has no Tauri dependency**. Every behaviour the UI relies on is a method on
`genius_core::app::Genius`, so the same code paths are exercised by Rust unit tests, the
Rust end-to-end test (`crates/genius-core/tests/e2e.rs`) and the WebDriver UI test.

## Key decisions and trade-offs

| Decision | Why | Trade-off |
|---|---|---|
| Tauri 2 + Rust backend | Small, native, strong capability/ACL model; secrets and processes stay in Rust | WebKitGTK on Linux renders slightly differently from WebView2 |
| Core as a separate crate | Testable without a GUI; Tauri layer is ~400 lines of wrappers | Command list duplicated in `build.rs` (guarded by a unit test) |
| SQLite (rusqlite, bundled) with `PRAGMA user_version` migrations | Zero-setup, transactional, forward-only migrations; refuses to open a DB newer than the app | No ORM; SQL kept small and tested |
| **File-block text protocol** instead of function calling | Works with every NVIDIA chat model, streams naturally so files appear live, parsed incrementally | Whole-file rewrites (not patches) cost more tokens; mitigated by context selection and a 512 KB per-file limit |
| Full-file content + locally computed diffs | Models produce reliable whole files far more often than valid patches | Large files are expensive to edit via AI |
| Changesets with before/after content | Recovery works without Git, every AI/manual change is reviewable/revertible, stale proposals are detected | Stores file contents in SQLite (bounded by file-size limits) |
| Deterministic Vite + React + TS template | The most common failure of generated projects is broken toolchain config; Genius owns it | Only one project type in the MVP |
| Real `npm`, `git`, `gh` executables, never a shell | Real results; arguments fixed by Genius so model output cannot become a command | Requires those tools installed (detected and explained in Settings) |
| GitHub via the official GitHub CLI | Uses GitHub's registered OAuth device flow; token lives in the OS keyring, never in Genius | Requires `gh`; a PAT fallback is offered (piped straight to gh) |
| Dev server in an iframe on `http://127.0.0.1:<port>` | Real app, isolated origin, HMR works | WebKitWebDriver can't focus cross-origin iframe inputs (test-only limitation) |

## NVIDIA integration

* Base URL default `https://integrate.api.nvidia.com/v1` (OpenAI-compatible). `http://` is accepted only for `localhost`/`127.0.0.1` (self-hosted NIM).
* **Model discovery:** `GET {base}/models`, parsed leniently (`data[].id`, `owned_by`, `created`). Cached per base URL with a timestamp; cache older than 24 h is flagged as stale; *Refresh models* always fetches live.
* **Capabilities are never assumed.** The listing has no capability metadata, so categories (chat/code/embedding/…) are labelled *inferred from the id*. A model is marked *verified* only after a real 16-token streaming probe succeeds with the user's key (stored per model).
* **Chat:** streaming SSE with incremental parsing; automatic fallback to non-streaming if a model rejects `stream`; SSE detected even if mislabelled; `<think>…</think>` and `reasoning_content` are hidden (only flagged); idle-timeout, cancellation via `CancellationToken`, retries with backoff for 429/502–504/network (honours `Retry-After`), continuation when `finish_reason == "length"`.
* **Errors** are classified: missing key, no model, auth (401/403), model unavailable (404 or "model not found"), rate limit, quota (402/credit), bad request (unsupported parameter), server, network, timeout, malformed response, cancelled. Messages are redacted.
* **Key storage:** `keyring` crate → Windows Credential Manager / macOS Keychain / Secret Service. The UI can save, replace, remove, and see a masked hint (`nvapi-…abcd`); the value is never returned to the webview. If no OS store is available (e.g. a headless Linux box), Genius falls back to memory-only storage and says so — never plaintext on disk.

## AI orchestration

All AI work runs as **tasks** (`spawn_task`): cancellable, one active AI task per project, persisted as a `task` activity event whose final status comes from the real result.

1. **Plan** — JSON spec (features, pages, components, data models, architecture, deps, milestones, tests, risks, assumptions, files). Robust JSON extraction + coercion of common shape variations + validation (npm package names). One automatic re-ask on invalid output. Architecture and assumptions seed *project memory*.
2. **Generate** — template changeset (deterministic), then streamed AI file blocks (drafts shown live), continuation on length limit, path validation (rejects traversal, `node_modules/`, `.git/`, `.env`, keys), `package.json` validation (no lifecycle scripts, no `git:`/`file:`/URL dependencies). Applied automatically only into a fresh template; regenerations follow the review policy. Then the **verify pipeline**.
3. **Change** — context selection (`context.rs`): explicit pins, references, keyword overlap, entry points, under a character budget; sensitive files never included; the UI previews the exact file list before sending and lets the user exclude/pin files. Produces a *proposed* changeset.
4. **Build Doctor** — parses real logs (tsc, esbuild, Vite/Rollup, Vitest) into diagnostics, sends only the failing command, exit code, focused log and referenced files, forbids "fixes" that weaken checks, records each attempt, and stops after `maxRepairAttempts` (default 3). Previous failed attempts are given to the model to avoid repeating them.
5. **Verify pipeline** — install (if `package.json` changed; approval-gated) → `npm run build` → on failure Build Doctor → on success preview. Every step is a real process with a real exit code.

## Safety model

* **Filesystem:** all paths are relative, normalized and validated; symlink escapes are detected by canonicalizing existing ancestors; writes are atomic (temp + rename) with size limits; binary files are never opened as text; apply is all-or-nothing with rollback; a changeset never overwrites files that changed since it was proposed.
* **Processes:** only `Operation::{Install, Dev, Build, Test, Typecheck}` mapped to fixed npm argument vectors; dev server bound to `127.0.0.1` on a Genius-chosen port; scripts that differ from the template require approval showing the exact script text; secret env vars are removed from child environments; each process runs in its own process group (Unix) or is killed with `taskkill /T` (Windows); all processes are killed on app exit; runs left "running" by a crash are marked *interrupted* at startup.
* **Approvals:** `approvals.rs` gate with per-category policy (AI edits, dependency install, commit, push — all *ask* by default). Approval events show exactly what will run.
* **IPC isolation:** application commands are declared in an app manifest (`src-tauri/build.rs`) and granted only to the `main` window's local origin (`capabilities/main.json`). The preview iframe is a different origin with no granted permissions; CSP restricts frames to `http://127.0.0.1:*`. Events are emitted only to the main window.
* **Secrets:** pattern + exact-value redaction on every event, log line, error and Git output; secret scan before commits/publish (blocking unless the user confirms a false positive); `.env*`, keys and credential files are excluded from AI context, exports and automatic writes.
* **Git:** Genius refuses to operate on a repository whose top-level is not the project folder; no force push, reset, clean or history rewrite is exposed; pulls are fast-forward-only unless the user explicitly chooses merge; remote URLs with embedded credentials or exotic transports (`ext::`, `file://`) are rejected; network ops use `gh auth git-credential` only for that command (global config untouched).

## Data

`<app data dir>/genius.db` (SQLite, WAL): `settings`, `projects` (spec, memory notes, status JSON), `messages`, `activity` (bounded to 3,000 events/project), `changesets` + `changeset_files`, `runs` (command, cwd, exit code, output tail), `model_cache`. Projects live in `<app data dir>/projects/` by default (configurable) — separate from Genius's own files.

## Live activity pipeline

`EventBus::start` creates a persisted event (`queued/running/completed/failed/cancelled/awaiting_approval`), `EventHandle::update` advances it, and each change is upserted in SQLite and pushed to the UI. High-frequency streams — model tokens (batched ~60 ms), parsed file drafts, process output lines (batched per animation frame in the UI) — use separate ephemeral channels. Drafts are clearly distinguished from applied files; an applied changeset converts open draft tabs into real file tabs.
