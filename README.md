# Project Genius

**An AI software studio for the desktop.** Describe an app; Genius plans it with an NVIDIA-hosted model, writes real source files you can read and edit, installs and builds it, runs it in a live preview, diagnoses real build failures, keeps a reversible history of every change, and publishes to GitHub — while showing you each operation as it happens.

> **Genius is a general-purpose AI assistant first** — chat, research, images and documents, data and calculations — with app building as one of its tools (the **Code** workspace).
>
> Status: working MVP. See [docs/STATUS.md](docs/STATUS.md) for exactly what is verified, what is mocked in tests, and what is not done yet.

## Genius Web (browser version)

Genius also runs on **Android** (APK, see [docs/MOBILE.md](docs/MOBILE.md)) and, as a project you can build on a Mac, **iOS**; the same Genius account gives the same conversations and (optionally, encrypted) the same provider keys on every device. Genius also runs in the browser: accounts, synced conversations, Chat, Research with real citations, a Code assistant, Agent Mode (web tools), Projects and an owner admin panel. It uses **your own** provider key, kept only in the browser tab. Setup and the exact Netlify/Supabase steps: [docs/DEPLOY-NETLIFY-SUPABASE.md](docs/DEPLOY-NETLIFY-SUPABASE.md) (Turkish quick guide: [docs/KURULUM-TR.md](docs/KURULUM-TR.md)).

```bash
npm run dev:web      # http://localhost:1430 (landing) and /app/index.html (app)
npm run build:web    # dist-web/ — what Netlify publishes
npm run test:db      # row level security on a real PostgreSQL engine + proxy/relay tests
```

What a browser cannot do, Genius Web says so: no local project folders, terminal, Git or builds (use Genius Desktop for those).

## What it does

- **Describe → Plan → Generate → Run.** A structured, editable plan (features, pages, data models, architecture, dependencies, tests, risks, assumptions); then real files for a Vite + React + TypeScript app, written live into the editor.
- **Watch the AI work.** Live activity feed of real operations (AI requests, file reads/writes, commands, builds, Git), streaming drafts that are clearly *not yet saved*, a terminal with real output and exit codes.
- **Review before anything changes.** AI edits arrive as diffs you apply or reject; nothing runs that you didn't allow (npm install, modified scripts, commits, pushes ask first by default).
- **Build Doctor.** When a build or test fails, Genius sends only the real diagnostics and relevant files to the model, proposes a targeted fix, re-runs the build, and stops after a bounded number of attempts.
- **History & recovery.** Every change — AI or yours — is a changeset you can inspect, revert, or restore the project to.
- **Live preview + element picker.** The project's real dev server, isolated on `127.0.0.1`; click an element in the running app to ask for a change to it.
- **Git & GitHub.** Status, diffs, staging, commits (secret-scanned), branches, fetch/pull/push (never forced), conflict resolution, and one-click *Publish to GitHub* with verified push.
- **Secure by design.** Your NVIDIA key lives in the OS credential store and never reaches the UI, logs, projects, or the preview.

## New in 0.2

- **Meet Genius.** An animated character who shows what it is doing (listening, thinking, working, happy, stuck), with a friendlier chat: Markdown answers, live narration, a progress checklist, and one-click answers when Genius asks you a question.
- **Simple and Advanced modes.** Simple (default) is just the chat and your running app: type an idea on Home and Genius plans and builds it. Advanced keeps the full IDE (files, terminal, Git, history, memory, integrations). Switch any time from the top bar.
- **Smarter agent.** Knows who it is, chats naturally, asks clarifying questions, builds whole multi-page websites in batches (`write_files`), keeps a visible todo list, and follows a built-in design guide.
- **More agents.** New *designer* and *tester* subagents alongside architect, reviewer and UI.
- **Internet access.** `web_search` and `fetch_url` tools for docs and inspiration (SSRF-protected, content treated as data; toggle in Settings → AI provider).
- **In-app updates** from GitHub Releases (see below).

## Earlier releases

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

## Updates & releases

Genius checks [GitHub Releases](https://github.com/hamzaedits1907-alt/project-genius/releases) when it starts and every 6 hours (turn this off in **Settings → Updates & about**). When a new version exists it shows what's new and, on **Update now**, downloads the installer for your platform, verifies its published SHA-256 checksum and installs it:

| Platform | What happens |
|---|---|
| Windows | the setup `.exe` runs and Genius closes. The installer is **not code-signed yet**, so Windows may warn; install only if you trust the source and compare the published SHA-256 |
| macOS | the `.dmg` opens; drag Genius into Applications (not notarized: if needed `xattr -cr /Applications/Genius.app`) |
| Linux AppImage | the AppImage replaces itself and restarts — fully automatic |
| Linux .deb | the package opens in your software installer |

Downloads are only accepted from GitHub, a release without a published `.sha256` is refused, and the checksum proves integrity only (it comes from the same release; signing is described in [docs/RELEASING.md](docs/RELEASING.md)). If the repository is private, the updater uses your GitHub CLI login (Settings → GitHub) or `GH_TOKEN`.

**Publishing a release** (one command):

1. Add a section for the new version at the top of [CHANGELOG.md](CHANGELOG.md) — it becomes the release notes shown in the app.
2. Run `scripts/release.sh 0.2.1`. It bumps every version field, commits, tags `v0.2.1` and pushes.
   (No terminal handy? Bump the versions, push to `main`, then GitHub → **Actions → Release → Run workflow**.)
3. The [Release workflow](.github/workflows/release.yml) builds Windows, macOS (Apple silicon + Intel) and Linux installers, publishes the release and its `.sha256` checksums. Installed copies update themselves.

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
