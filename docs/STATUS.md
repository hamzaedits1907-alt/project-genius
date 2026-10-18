# Status

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
