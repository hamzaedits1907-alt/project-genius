# Testing

Three layers, all runnable locally. Live NVIDIA and live GitHub are **not** exercised by automated tests (no credentials in CI); everything else uses real tools.

## 1. Rust unit tests — `cargo test -p genius-core --lib`

~96 tests across the core modules, including:

| Area | Examples |
|---|---|
| Workspace sandbox | traversal/absolute/drive/NUL/reserved-name rejection, **symlink escape** rejection, protected dirs, secret file names, size limits, binary detection, atomic writes |
| Changesets | diff stats, propose/apply/revert, **stale proposals refused**, revert conflict detection, restore-to-point, no-op/unsafe changes |
| NVIDIA provider (wiremock) | request body shape, base-URL rules, SSE split chunks, `<think>` filtering, error classification (401/402/403/404/422/429/5xx), key redaction in errors, bearer auth on `/models`, invalid key, missing key (no network call), **rate-limit retry**, streaming order, mislabelled SSE, non-stream fallback, malformed chunks, cancellation |
| Processes | fixed argument vectors, ANSI + secret stripping, real exit codes and streamed output, **process-group kill** (child of child is gone), persisted runs |
| Git (real `git`) | porcelain v2 parsing, init/stage/unstage/commit/log/branches, unrelated files stay uncommitted, **nested project never touches the parent repo**, push to a local bare remote + `ls-remote` verification, **diverged remote → classified rejection, no force push**, malicious ref/URL rejection |
| Persistence | fresh migration, reopen keeps data, refuses newer schema, status merge, cascade delete, settings validation |
| Events / approvals | lifecycle persistence, redaction, stale-event cleanup, proposals resolved and kept across restart, approve/deny/auto/cancel |
| Parsing | file-block protocol (char-by-char streaming equals batch), plan JSON extraction/coercion/validation, diagnostics for tsc/esbuild/Vite, log focusing, secret scanning |
| Runtime | tasks spawned from a non-Tokio thread (Tauri sync command regression test) |

## 2. Rust end-to-end — `cargo test -p genius-core --test e2e`

Drives `Genius` exactly as the UI does, with a mock NVIDIA server (wiremock) and **real** npm/TypeScript/Vite/Vitest:

key save (masked) → live model list → cache → select → test connection (+probe) → create → plan → generate (template + streamed files) → **real `npm install` → real `tsc && vite build` → real dev server** answering HTTP with the generated app and the injected element inspector → **real Vitest run** of the generated test → editor save (manual changeset) → change request with context preview → reviewable diff (not applied before approval) → apply → background verification build → change that breaks the build → **real TS2304 failure captured** → Build Doctor receives the real diagnostics and file → fix applied and re-verified by a new real build → stop preview → restore to an earlier point → zip export → readiness report → close/reopen with all data intact, no stale running events. Also: missing key and invalid key errors, and the key never appearing in errors or activity.

Requires Node/npm and npm registry access; skipped with a message otherwise (`GENIUS_SKIP_E2E=1` to skip).

## 3. UI end-to-end — `tests/ui/e2e_ui.py`

Drives the **real desktop app** through WebDriver (`tauri-driver` + `WebKitWebDriver`) under Xvfb, with `tests/ui/mock_nvidia.py` standing in for NVIDIA:

settings (base URL, key, refresh models, embedding model hidden, select, test connection) → new project → plan rendered → generate → **approval dialog for npm install** → real install/build/dev server → preview shows the generated app and **responds to input** → element picker returns the clicked element → change request → diff review → apply → verification build succeeds → preview hot-reloads with the change → terminal output → history → Git init, identity, stage, commit → readiness → **restart the app** → project and build status persisted. Screenshots are written for every step.

```bash
sudo apt install xvfb webkit2gtk-driver && cargo install tauri-driver && pip install selenium
npx tauri build --debug --no-bundle
python3 tests/ui/e2e_ui.py /tmp/genius-shots
```

Note: WebKitWebDriver cannot focus inputs inside a cross-origin iframe, so the test interacts with the previewed app by dispatching DOM events inside the frame (the app's real React handlers run).

## 4. Frontend — `npm run typecheck && npm run lint && npm test && npm run build`

Vitest covers the pure state logic (activity upserts and ordering, filters, file tree, live-draft reducer, diff classification, formatting). `src-tauri` has a unit test ensuring every registered command is in the ACL manifest and granted to the main window.

## Not covered automatically (manual verification needed)

- Live NVIDIA authentication, model listing and inference against `integrate.api.nvidia.com` (blocked from the build environment by its network policy).
- Live GitHub device login, repository creation and push to github.com.
- Windows Credential Manager / macOS Keychain storage (the Linux CI box has no Secret Service, so the memory-only fallback path was exercised instead).
- Windows/macOS builds of the desktop app (built and tested on Linux only).
