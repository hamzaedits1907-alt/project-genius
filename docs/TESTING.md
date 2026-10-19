# Testing

## 0.6.0 run (this workstation, Windows 11, Node 22, Chrome installed)

Run one suite at a time (the machine runs out of resources if many `npm install`s start at once: the 8 dashboard tests need `--test-threads=2`).

| Suite | Result | Kind |
|---|---|---|
| `cargo test -p genius-core --lib` | 311 passed | unit |
| `--test finance` (Personal Finance Tracker: plan → generate with a cut-off answer and no App.tsx → completion round → apply → npm install → tsc/vite → vitest → dev server → headless Chrome interactions → review/apply → follow-up; and the never-writes-App.tsx failure case) | 2 passed (46 s) | **real pipeline**, scripted model |
| `--test e2e` | 2 passed | real pipeline, mock model |
| `--test dashboard -- --test-threads=2` | 8 passed | real pipeline, mock model |
| `--test agent`, `missions` | 9 + 5 passed | real agent/mission engine, isolated copies, mock model |
| `--test chat_research`, `chat_identity`, `providers`, `brain`, `web_ssrf`, `browser` | 15, 10, 16, 7, 5, 6 passed | mock/contract; `browser` is a real headless browser |
| `cargo clippy` (CI flags), `cargo test -p genius` | clean, passed | |
| `npm run typecheck`, `npm run lint`, `npm test` | clean, 215 passed (24 files) | jsdom UI tests |
| `npm run build`, `npm run build:web` | pass | |
| `npm run test:db` (PGlite + Netlify config/proxy) | 44 passed | **real engine** |

Not run: live NVIDIA/OpenAI/Anthropic/Gemini calls in this release (opt-in `live_*` tests), a live Supabase project, the deployed Netlify site, the packaged Tauri window, the React admin panel against a database, real IndexedDB in a browser.


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


## 7. Genius Chat: identity, request metadata, images and tools — `cargo test -p genius-core --test chat_identity`

A scripted NVIDIA stand-in (wiremock) records every request body, so these tests prove what Genius really *sends* and *stores*, **not** what a live model does:

- the system prompt never names a model; a plain introduction is answered directly;
- the requested model, the provider-reported label and the per-request tokens stay separate in the message metadata, including when they differ;
- changing the model in Settings changes the next request; Auto mode never passes a stale preference off as the served model;
- an explicit "which model are you" is answered from `runtime_info`;
- a screenshot reaches a vision model as one real `image_url` payload (the exact attached bytes, sent once, to a vision model), follow-ups keep the picture, unrelated messages do not; with no vision model nothing is faked; non-images with an image extension are rejected;
- `calculate` and `data_summary` return computed values to the model.

Reproduction of the original identity bug: run this file against the pre-change sources (`git stash push -- crates/genius-core/src`): 7 of its 8 original tests fail.

### Live chat test (real NVIDIA API, opt-in) — `NVIDIA_API_KEY=… cargo test -p genius-core --test live_chat -- --nocapture`

Sends two tiny requests with your key: a PNG whose pixels are red but whose file name says "blue" (a model that reads the name answers blue), and a greeting. Prints the model, the provider's label, first-token and total times. Skipped without the key.

## 8. Frontend UI states without the desktop shell

`src/test/chat-ui.test.tsx` covers the sidebar, history, welcome and first-run screens, attachments and picture previews, failure and retry, math, and the stop button against the mocked backend.

To look at the real rendering, a development-only harness (`harness.html`, `src/harness/main.tsx`, not part of the production build) mounts the real app on fixture data: run `npx vite --port 5199` and open `http://localhost:5199/harness.html#s=<welcome|firstrun|thread|image|web|research|running|error>&theme=<dark|light>`. The data there is fixture data for layout review only.

## 7. Genius Core (0.5.0)

Run heavy suites one after another, not in parallel (a full parallel run can exhaust 16 GB): `CARGO_BUILD_JOBS=2 cargo test -p genius-core --test <name> -- --test-threads=2`.

| Suite | What it proves | Kind |
|---|---|---|
| `--lib` (299) | budgets, provider adapters (bodies, replies, streams, errors), Brain index/impact/search/staleness, diagnostics hints, everything older | unit |
| `--test providers` (16) | Anthropic/Gemini/OpenAI/custom over HTTP: URL, auth header, key never in URL/body, usage, stream, errors, cancellation, rejected `temperature`; key slots; measured vs estimated usage; model-call budget to `blocked`; concurrency cap; diagnostics stages and secret non-leak | **contract** (stand-in server, no live provider) |
| `--test brain` (6) | persistence across restart, incremental refresh, git history, memory staleness, project isolation, scope, `project_brain` tool via a scripted model | real files + real git, scripted model |
| `--test agent`, `chat_identity`, `chat_research`, `missions`, `web_ssrf` | agent loop, identity/image metadata, research citations, missions, SSRF | mock provider |
| `--test dashboard`, `browser`, `e2e` | Nexus Dashboard regression, real headless browser, real npm/tsc/vite | real tools, scripted model |
| `--test live_nvidia`, `live_chat` | Auto routing + real tool read, image pixels vs file name, greeting, provider diagnostics | **live**, opt-in: skipped unless `NVIDIA_API_KEY` is set |
| `npm test` (161) | UI incl. provider card, diagnostics, limits, Brain panel, blocked report | mocked IPC (jsdom), not the packaged app |

Not covered by any automated test: the packaged Tauri window, live OpenAI/Anthropic/Gemini, live web search. The Windows updater was tested **manually by the user** (reported successful); CI covers it separately (`updater-windows.yml`).

## 7. Genius Web and the database (this branch)

| Command | What it proves | What it does not |
|---|---|---|
| `npx vitest run` (frontend) | Interface, Agent Mode screen, citations, web engine (`web-engine`), conversation loop with a scripted model (`web-chat`), runtime commands, key vault, relay consent, missions (`web-runtime`) | Any live model or provider. On a small machine the whole suite can run out of memory: `maxWorkers` is capped at 2; run files one at a time if needed. |
| `npm run test:db` | **Real PostgreSQL (PGlite)** with the real migrations: row level security, admin functions, audit; plus `tests/netlify`: SSRF rules, redirects, size limits against a real local HTTP server, session and kill-switch checks with a stand-in Supabase, Netlify configuration | A live Supabase project, emails, the functions running on Netlify |
| `npm run build:web` | Type check, production build of landing + app, Netlify function type check | The deployed site |
| `NVIDIA_API_KEY=… cargo test -p genius-core --test live_mission -- --nocapture` | **Live** Agent Mode mission (opt-in, uses your quota) | Other providers |

Rust: run suites one at a time with `CARGO_BUILD_JOBS=2` (memory).
