# Status

## 0.6.0 — Code Mode repair, desktop accounts, advanced admin

Evidence words: **real pipeline** = real npm / tsc / Vite / Vitest / headless Chrome against a real project folder (only the model is scripted); **real engine** = real PostgreSQL (PGlite) with the real migrations; **tested (mock)**; **not verified**.

| Area | State | Evidence / limits |
|---|---|---|
| Code Mode: generated app shown as built while the preview showed the starter page | **fixed** | Root causes found by reading the pipeline: (1) cut-off or forgotten files (usually `src/App.tsx`, written last) were dropped silently; (2) the generate task counted as *completed* when it only proposed files or its build failed, and the UI then jumped to a preview of the old disk state; (3) verification built the starter template and reported success; (4) in review mode a build ran on the OLD files and counted as verification of the staged ones. Fix: output integrity checks (`integrity.rs`) + bounded targeted completion rounds, starter-only projects apply directly, an explicit state machine (`awaiting_review`, `incomplete`, `build_failed`, `built`, `runtime_errors`, `verified`), placeholder guard in build/verify/agent checks, banner + status badges in the UI. |
| Personal Finance Tracker regression | passes | **real pipeline**: `tests/finance.rs` — first model answer cut off and without App.tsx → Genius asks again naming the files → apply → npm install → tsc + vite build → Vitest → dev server serves the generated App → headless Chrome: dashboard totals, add/edit/delete, search/filter, navigation, localStorage persistence after reload, 375/768 px layouts, no console errors → regeneration waits for review, apply → follow-up change visible in the running preview. A second test asserts a model that never writes App.tsx FAILS (no build is run, nothing reported as built). 46 s. Model output is scripted: this proves the pipeline, not any live model's code quality. |
| Draft/apply distinction | implemented | Files marked "not applied" / "cut off — not saved", banner with Apply / Review / Discard, `pendingChangeset` in project status; 5 UI tests (`generation-banner`). |
| Desktop sign-up / sign-in (Supabase) | implemented | Same project as Genius Web; optional; settings → Account; public-key check refuses service-role/secret keys. **Not verified against a live Supabase project** (needs the project URL/anon key and the dashboard setting below). |
| E-mail confirmation | **owner action** | Must be switched off in the Supabase dashboard (cannot be done by code). The app handles both settings. Admin never depends on e-mail. |
| Provider key persistence | implemented | Desktop: OS credential store (already). Web: "Remember this key on this device" — AES-GCM + non-exportable key in IndexedDB; 6 tests with real Web Crypto and a stand-in IndexedDB. Not a defence against XSS (documented). Real IndexedDB in a real browser: not run. |
| Owner admin panel | rebuilt | users (filters, sort, paging, bulk, drawer, notes, CSV), usage charts, plan limits, maintenance, registrations switch, announcements, audit filters, read-only admin list, system checks. **real engine**: 9 new tests (27 in total in `rls.test.ts`). The React panel itself is typechecked and linted, **not** exercised in a browser against a database. |
| Public site | updated | NVIDIA key guide (steps from NVIDIA's documented flow; limits are NVIDIA's and may change), Discord invite, no GitHub repository links. |

## 0.6.0 (continued) — Genius Web and core repairs

Evidence levels as below: **tested (mock)**, **contract**, **live**, **user-reported**, **not verified**. New here: **real engine** = run against a real local component (PostgreSQL via PGlite, a real local HTTP server, a real browser).

| Area | State | Evidence / limits |
|---|---|---|
| Malformed tool calls (`<web_search {…}>`, `<websearch {…}` without `>`, `<todo>{…}</todo>`, Markdown-link URLs) | fixed | Rust: `toolcall` unit tests + `chat_research` integration tests; TypeScript twin in Genius Web (`web-engine`, `web-chat`). Raw markup never reaches an answer (one correction round, then stripped); identical failing calls are blocked. Mock models only: no live model was asked to produce these dialects. |
| Citation links (`tauri.localhost/#src-1`) | fixed | Anchors replaced by links to the fetched page or plain text; local/app links never rendered as sources (`citations` UI tests, `strip_app_links` tests, ledger tests). |
| Identity / creator | fixed | `product_creator` setting (default Hamza), "never assume the user is the creator", conversation facts kept (`chat_identity` tests). |
| Research on a URL (script-built pages) | improved | Page note + metadata (unit and local-server tests). A live fetch of the Lufthansa example page was **not** run. |
| Agent Mode | UI rebuilt; engine verified | The mission engine already worked: opt-in **live** run against NVIDIA (`live_mission`): plan 2.5 s, one task in an isolated copy, approval, real `docs/inventory.md`, completed with verification *unknown* (reported as such). The failures were product flow (a project was required, manual start, buried approval). UI: automatic start, approval card, activity log, report (`agent-mode` tests). |
| XLSX files | implemented | Unit test with a constructed workbook; first sheet as table, others named in a note. Scanned PDFs are still not OCR'd (honest message: attach pages as images). |
| Auto model choice (OpenAI/Anthropic/Gemini) | implemented | `router` unit tests with model lists; **no live call**. |
| Project Brain call relations | implemented | `tests/brain.rs`. Pattern scan only. |
| UI redesign (desktop and web) | implemented | Visual system, model chip, Projects, Agent Mode, shortcuts; Vitest suites and screenshots of the real interface in headless Chrome. **Not** run inside the packaged Tauri window. |
| Genius Web runtime/engine | implemented | `web-engine` 14, `web-chat` 8, `web-runtime` 9 tests with a scripted model and stand-in tools; interface rendered in a real browser (local-only mode). **No live provider call from the browser; no configured Supabase project yet.** |
| Supabase schema, RLS, admin functions | implemented | **real engine**: 18 tests on PostgreSQL (PGlite) with the real migrations: owner isolation, no self-escalation, no client path to plans/flags/admin, secret rejection in preferences, kill switch, rollout, cohorts, grants, suspension, audit. **Not verified on a live Supabase project** (auth emails, redirect URLs, storage). |
| Web proxy and provider relay | implemented | **real engine**: 12 tests incl. SSRF forms, redirect hops, size/type limits against a local server. Not deployed. |
| Browser access of providers | measured | Preflight on 2026-10-19: OpenAI yes, Anthropic yes (browser header), Gemini yes, NVIDIA no (no CORS headers) → relay with explicit user consent. |
| Billing | not implemented | By design. |
| Windows updater | **user-reported OK** | Tested by hand by the user; not re-run. |

Known gaps: custom agents/teams in the browser, pause/resume of web missions, charts/artifacts, resumable tasks, scanned-PDF OCR, live verification of non-NVIDIA providers, Netlify long-stream limits.

## 0.5.0 — Genius Core: general-purpose assistant, Project Brain, budgets, providers

Verification levels: **tested (mock)** = automated test against a scripted stand-in (model or HTTP server); **contract** = adapter tested against a stand-in server that mimics the provider's documented request/response shapes; **live** = run against the real API with the author's NVIDIA key; **user-reported** = tested by the user by hand; **not verified** = not run.

| Area | State | Evidence / limits |
|---|---|---|
| Conversation-first UI, Markdown, tables, KaTeX, attachments, sources, Details, retry/stop | implemented | 18 + 8 UI tests (`chat-ui`, `chat-mode`, `provider-ui`). Dev-harness screenshots only; **not** run in the packaged Tauri window by the assistant. |
| Model identity, image routing | fixed | 9 mock tests; **live**: greeting names Genius, red-pixel PNG called `blue.png` answered "red" by the vision model (re-run after the provider refactor). |
| Execution budgets | implemented | `budget.rs` (4 unit tests) + stand-in tests: exactly N model calls then `blocked` with the limit named; concurrent-task cap refuses an extra task; retries configurable. Defaults: 100 model calls, 150 tool calls, 45 min, tokens unlimited, 6 tasks. |
| Usage reporting | implemented | Provider-reported tokens kept as is; requests without `usage` are labelled `estimated` (characters / 4) and never merged. Provider billing is not visible to Genius. |
| Provider diagnostics | implemented | One report (key present, address, reach + auth + models, model access, test message, image input, optional tool-format check). **live** on NVIDIA (all stages pass, key absent from the report); stand-in tests for missing key (no network call), rejected key (classified, no leak), unknown model. |
| Providers: OpenAI, Anthropic, Gemini, OpenAI-compatible | **contract only** | Native request/response/stream/error adapters, per-provider key slot, 16 HTTP-level tests (headers, body shape, usage, errors, cancellation, parameter rejection). **No live call** to these providers; Auto routing is NVIDIA-only. |
| Project Brain v1 | partial | 9 unit + 6 integration tests: incremental index, symbols, import graph (TS/JS/Python/Rust), commands, impact, retrieval, history/memory kept apart and flagged stale, project isolation, `project_brain` tool, overview in the prompt. Pattern-based; no call graph, no embeddings, no cross-project view. |
| `live_nvidia` | fixed (test design) | See "live_nvidia investigation" below. |
| Windows installer/updater | **user-reported OK** | The user tested the updater on Windows by hand. Not re-run by the assistant; CI's Windows job and `updater-windows.yml` remain separate automated evidence. |
| Coding engine, Nexus Dashboard regression, browser inspection, checkpoints | preserved | `dashboard` 8/8, `browser` 6/6, `e2e` 2/2 re-run in this release. |

### live_nvidia investigation
The test asked the live agent to "read src/App.tsx and say what the h1 says" and required a `read_file` step. For a small project Genius inlines every source file into the prompt (`CURRENT FILES`), and the prompt tells the model not to re-read what it already has, so a model that answers directly is behaving correctly, and the test failed whenever it did. It was a test-design defect, not an application bug: the failure could not be reproduced on a rerun (the model chose to read), which confirms the nondeterminism. Fix: the needle now lives in `notes/launch-code.txt`, a file type the snapshot never inlines, with a random value; a deterministic mock test (`small_project_sources_are_inlined_so_a_read_is_not_required`) pins the cause. The assertion that a real `read_file` happened was **kept**. Live result: passes (3 runs).

Still missing or partial: charts and interactive artifacts; dependency/licence audit; Project Brain call graph and semantic search; Anthropic/OpenAI/Gemini live verification; resumable tasks; Agent City control centre and agent certification. See [ROADMAP.md](ROADMAP.md).


Honest engineering checklist. Legend: done and tested / partial / missing. "Mock" means a scripted stand-in for an external service; "real" means the actual tool or network.

## Confirmed working (with evidence)

| Area | Evidence |
|---|---|
| Project creation, plan, streamed file generation, editing, changesets, revert, export | Rust e2e (`tests/e2e.rs`) with a mock NVIDIA server and real npm, tsc, vite, vitest |
| Real build, dev server, live preview, Build Doctor on a real TS error | same e2e; UI e2e on Linux (`tests/ui/e2e_ui.py`) |
| Agent tool loop, tool-call normalization, sandbox refusals, secrets not sent to the model | `tests/agent.rs` (mock provider), unit tests |
| Git (real git, local bare remotes) | unit tests |
| Persistence and restart | Rust and UI e2e |
| Updater logic: version compare, asset selection, host allow-list, `.part` then rename, checksum/size/truncation failures leave nothing behind, install only from the update folder, AppImage replace with `.bak` and rollback, proxy variable honoured | `cargo test -p genius-core --lib updater` and `--test updater_live` (offline part) |
| Updater against the real GitHub release (Linux AppImage path: check, download, SHA-256, tamper rejection, replace) | `GENIUS_LIVE_UPDATER=1 cargo test -p genius-core --test updater_live` ran once from this workstream |
| Linux AppImage self-update in the real app | `tests/ui/update_e2e.py` (earlier release) |
| Clippy clean (two stylistic lints allowed) | CI `rust` job, run locally |

## Partial

- Windows: core tests, ACL test and installer build run in CI; the app has not been confirmed to run on Windows by automated tests. The new Windows updater verification workflow exists but **has not been run yet**.
- Release gates (`ci-gate`, `verify-artifacts`, manifest) are new: their scripts were tested locally with fake data, the workflows were only YAML and `bash -n` checked, never executed on GitHub.
- GitHub integration: tested with a fake `gh`; live github.com not verified.
- Supabase: status and key handling unit-tested; no live call.

## Missing

- Live NVIDIA verification (host blocked in the build sandbox).
- Code signing (Authenticode, macOS notarization) and updater signatures (see `docs/RELEASING.md`).
- Windows interactive installer and in-app "Update now" flow under automation; macOS runtime tests.
- Enforced `cargo fmt` gate.

## Mock vs real

Mock: NVIDIA (wiremock, `tests/ui/mock_nvidia.py`), model output in agent tests, `gh`, GitHub API in updater unit tests. Real: npm, tsc, vite, vitest, git, SQLite, filesystem, process control, GitHub Releases in the live updater test.

## Known limitations

- One project type (Vite, React, TypeScript).
- Linux without Secret Service keeps the NVIDIA key in memory only.
- The element picker does not map to a source line.
- GitHub CLI is required for GitHub features.
- Checksums prove integrity only; installers are unsigned.

## Roadmap phases

| Phase | Status |
|---|---|
| 1 Desktop foundation | done |
| 2 NVIDIA provider | partial: mock-tested, live unverified |
| 3 Project generation | done |
| 4 Execution and preview | done |
| 5 Editing and Build Doctor | done |
| 6 Recovery, export, releases | done for projects; release/update pipeline hardened but the new gates are unproven on GitHub |
