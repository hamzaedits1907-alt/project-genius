# Project Genius

**An AI software studio for the desktop.** Describe an app; Genius plans it with an NVIDIA-hosted model, writes real source files you can read and edit, installs and builds it, runs it in a live preview, diagnoses real build failures, keeps a reversible history of every change, and publishes to GitHub — while showing you each operation as it happens.

> Status: working MVP. See [docs/STATUS.md](docs/STATUS.md) for exactly what is verified, what is mocked in tests, and what is not done yet.

## What it does

- **Describe → Plan → Generate → Run.** A structured, editable plan (features, pages, data models, architecture, dependencies, tests, risks, assumptions); then real files for a Vite + React + TypeScript app, written live into the editor.
- **Watch the AI work.** Live activity feed of real operations (AI requests, file reads/writes, commands, builds, Git), streaming drafts that are clearly *not yet saved*, a terminal with real output and exit codes.
- **Review before anything changes.** AI edits arrive as diffs you apply or reject; nothing runs that you didn't allow (npm install, modified scripts, commits, pushes ask first by default).
- **Build Doctor.** When a build or test fails, Genius sends only the real diagnostics and relevant files to the model, proposes a targeted fix, re-runs the build, and stops after a bounded number of attempts.
- **History & recovery.** Every change — AI or yours — is a changeset you can inspect, revert, or restore the project to.
- **Live preview + element picker.** The project's real dev server, isolated on `127.0.0.1`; click an element in the running app to ask for a change to it.
- **Git & GitHub.** Status, diffs, staging, commits (secret-scanned), branches, fetch/pull/push (never forced), conflict resolution, and one-click *Publish to GitHub* with verified push.
- **Secure by design.** Your NVIDIA key lives in the OS credential store and never reaches the UI, logs, projects, or the preview.

## Requirements

| | |
|---|---|
| Node.js 20+ and npm | to install/build/run generated projects (and to build Genius) |
| Rust 1.80+ | to build Genius |
| Git | source control features |
| GitHub CLI (`gh`) | GitHub sign-in, repository creation, publishing (optional) |
| Windows: WebView2 (preinstalled on Windows 10/11) · Linux: `libwebkit2gtk-4.1-dev`, `libsecret`/Secret Service · macOS: Xcode CLT | Tauri prerequisites |

## Run it

```bash
npm install
npm run tauri dev          # development
npm run tauri build        # installers in target/release/bundle
```

Then open **Settings → AI provider**, paste your NVIDIA API key (from [build.nvidia.com](https://build.nvidia.com/)), press **Refresh models**, pick a model (optionally **Verify** it), and **Test connection**. Create a project from the **+** button.

## Develop & test

```bash
cargo test --workspace                         # Rust unit + end-to-end (mock NVIDIA, real npm/tsc/vite/git)
npm run typecheck && npm run lint && npm test  # frontend
npm run build                                  # frontend production build
npx tauri build --debug --no-bundle && python3 tests/ui/e2e_ui.py /tmp/shots   # drive the real app (Linux, needs Xvfb + tauri-driver)
```

Details in [docs/TESTING.md](docs/TESTING.md).

## Layout

```
crates/genius-core   platform-independent engine (all logic, tests)
src-tauri            Tauri shell: IPC commands, ACL, window, app lifecycle
src                  React UI (views, workspace panels, stores, design system)
tests/ui             WebDriver UI test + mock NVIDIA server (test-only)
docs                 architecture, status, ideas, testing
```

## Documentation

- [docs/architecture.md](docs/architecture.md) — design, trade-offs, security model, NVIDIA integration
- [docs/STATUS.md](docs/STATUS.md) — phase status, MVP acceptance checklist, known limitations
- [docs/IDEAS.md](docs/IDEAS.md) — implemented and proposed improvements
- [docs/TESTING.md](docs/TESTING.md) — test strategy and how to run each suite
