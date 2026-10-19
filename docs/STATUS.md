# Status

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
