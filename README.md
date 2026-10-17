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

## New in this release: effortless build → preview → fix

- **Describe it, see it.** Type what you want on Home and press *Create*: Genius plans, writes the code, installs dependencies, runs the checks and opens the running site — one step, no npm commands.
- **Preview that tells the truth.** *Preview* checks the project, installs dependencies when the files say so, starts the dev server and only reports *ready* after a real HTTP answer carrying that run's identity and a compiled module graph. Refresh, Restart, Stop and Open in browser are always at hand.
- **Fix automatically.** When installing, compiling, building or the running page fails, one click collects the real error, has the AI propose the smallest fix, validates it, applies it (undoable), reinstalls if needed and re-runs the checks — at most 3 attempts, and it says exactly what is still wrong if it can't.
- **Safe AI edits.** Proposals are validated before they touch your files: strict `package.json` (duplicate keys, missing scripts, lifecycle hooks), undeclared imports, missing or deleted files, abbreviated "rest unchanged" content. The model is told why and gets to correct itself.
- **A simple workspace.** Ask Genius + Preview + Changes + Activity by default; files, editor, terminal, Git, history, memory and integrations under *Show code & tools* and *More*.
- **Windows fix.** Project folders are no longer handed to `npm.cmd` as `\\?\C:\…` verbatim paths (which `cmd.exe` rejects); npm is launched as `node.exe npm-cli.js` when available.

## Earlier release

- **Talk to Genius about your project.** Every project has persistent conversations. Follow-ups ("add a sidebar", "the sidebar is broken on mobile — fix it") run a bounded agent that inspects the real code with tools, proposes a targeted diff, and reports exactly what it read, ran and has *not* verified.
- **Your files as context.** Attach or import TXT/MD/CSV/JSON/code/PDF/DOCX/images/ZIP; import a ZIP as a new project with safe extraction.
- **Knows what it can do.** A live capability report (model, verified features, tools, integrations, permissions) — also what the agent consults when you ask about it.
- **Subagents.** A reviewer checks multi-file proposals; architect/reviewer/UI specialists can be consulted read-only. Every step shows which agent ran it.
- **Integrations.** Registry with GitHub and Supabase (connect with a token, link a project, configure the client safely, migrations and types as reviewable changes, remote SQL only after explicit approval).

Windows setup: [docs/WINDOWS.md](docs/WINDOWS.md).

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
