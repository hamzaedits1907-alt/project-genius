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

first-run quick setup on Home (key saved, model chosen automatically) → **simple mode**: one prompt → plan → code → **approval dialog for npm install** → real install/build → the dev server starts by itself and the preview shows the generated app and **responds to input** → Genius asks a question and a **one-click answer** is sent → element picker → change request → **Apply from the chat** → verification build succeeds → preview hot-reloads → Code view → switch to **advanced mode** → terminal output → history → Git init, identity, stage, commit → readiness → update check against GitHub Releases → **restart the app** → project, conversation and mode persisted. Screenshots are written for every step.

```bash
sudo apt install xvfb webkit2gtk-driver && cargo install tauri-driver && pip install selenium
npx tauri build --debug --no-bundle
python3 tests/ui/e2e_ui.py /tmp/genius-shots
```

Note: WebKitWebDriver cannot focus inputs inside a cross-origin iframe, so the test interacts with the previewed app by dispatching DOM events inside the frame (the app's real React handlers run).

### Updater, end to end with real releases — `tests/ui/update_e2e.py`

Downloads the **previous release's real AppImage** from GitHub, runs it under Xvfb through WebDriver, lets it discover the latest release by itself, clicks **Update now**, then checks that the AppImage on disk was replaced by a file whose SHA-256 matches the checksum published with the release, that the app relaunched itself, and that the relaunched app reports the new version and shows its "Welcome to Genius x.y.z" screen.

```bash
python3 tests/ui/update_e2e.py /tmp/update-shots          # previous release → latest
python3 tests/ui/update_e2e.py /tmp/update-shots v0.2.0   # a specific old release → latest
```

## 4. Frontend — `npm run typecheck && npm run lint && npm test && npm run build`

Vitest covers the pure state logic (activity upserts and ordering, filters, file tree, live-draft reducer, diff classification, formatting). `src-tauri` has a unit test ensuring every registered command is in the ACL manifest and granted to the main window.

## 5. Agent, conversations, imports — `cargo test -p genius-core --test agent`

A scripted **mock** provider plays the model's side of the tool protocol (labelled in the file). Real: conversation storage, tool execution on a real project folder, sandbox refusals (`.env`, `../`), unknown tools, invalid arguments, duplicate suppression, 14-step bound, plan mode, reviewer subagent, changesets, evidence-based reports, restart persistence, and that `.env` contents never reach the provider.

## 6. Regression guards for the view crash

- `src/lib/effects.test.tsx` — reproduces React 19's "destroy is not a function", tests the ESLint rule, and the StrictMode-safe `useSubscription`.
- `src/test/views.test.tsx` — mounts the real `App` in StrictMode with a mocked IPC and walks every view, tab and panel through mount/unmount.
- `tests/ui/walk_views.py` — the same walk in the **real webview**; `tests/ui/e2e_ui.py` fails on any error boundary or effect-cleanup error at every step.

## Not covered automatically (manual verification needed)

- Live NVIDIA authentication, model listing and inference against `integrate.api.nvidia.com` (blocked from the build environment by its network policy).
- Live GitHub device login, repository creation and push to github.com.
- Windows Credential Manager / macOS Keychain storage (the Linux CI box has no Secret Service, so the memory-only fallback path was exercised instead).
- Windows/macOS builds of the desktop app (built and tested on Linux only).

## Updater: failure behaviour and live verification

- `cargo test -p genius-core --lib updater` covers partial downloads (previous file untouched, no `.part`), a cut connection, checksum mismatch, missing checksum, unsafe install paths, AppImage backup and rollback.
- `cargo test -p genius-core --test updater_live` always runs the offline checks, including a child-process test that `HTTPS_PROXY` is honoured (a fake local proxy sees `CONNECT api.github.com:443`). With `GENIUS_LIVE_UPDATER=1` it also talks to the real GitHub release (needs network; optional `GENIUS_LIVE_OLD_VERSION`, `GENIUS_LIVE_NEW_TAG`).
- The Windows installer path is verified by `.github/workflows/updater-windows.yml` (manual; see `docs/RELEASING.md`). Not yet run.
- CI runs clippy with `-D warnings`; rustfmt is reported only.
