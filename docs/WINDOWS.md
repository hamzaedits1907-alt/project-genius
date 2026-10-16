# Windows 11 setup and verification

Genius is developed and tested on Linux in CI; the Windows target **type-checks** (`cargo check --target x86_64-pc-windows-gnu`) but the desktop app has **not yet been run on Windows by the automated suites**. Use this checklist to verify it on your machine.

## Prerequisites (install once; Genius never changes Windows security settings)

| Tool | Get it | Check |
|---|---|---|
| Microsoft C++ Build Tools ("Desktop development with C++") | https://visualstudio.microsoft.com/visual-cpp-build-tools/ | — |
| WebView2 runtime | preinstalled on Windows 10/11 | — |
| Rust (MSVC toolchain) | https://rustup.rs | `rustc --version` (1.80+) |
| Node.js 20+ (includes npm) | https://nodejs.org | `node --version` |
| Git for Windows | https://git-scm.com | `git --version` |
| GitHub CLI (optional, for GitHub) | https://cli.github.com | `gh --version` |

## Run and build (PowerShell, in `C:\Users\<you>\Downloads\project-genius`)

```powershell
npm install
npm run tauri dev        # development (React dev build, StrictMode)
npm run tauri build      # installers in target\release\bundle\(msi|nsis)
```

## Checks before reporting a problem

```powershell
npm run typecheck; npm run lint; npm test
cargo test --workspace
```

## Manual verification checklist

1. App starts, Home shows; open Settings, every tab renders.
2. Settings → AI provider: save key → **stored in Windows Credential Manager** (shown as the backend; also visible in *Credential Manager → Windows Credentials* as `dev.genius.studio`).
3. Refresh models → pick → Verify → Test connection (live NVIDIA).
4. Create a project, generate, run preview; send a follow-up in the Assistant ("add a sidebar"); review and apply the diff; check the build result in the message.
5. Close and reopen Genius: project, conversation and history are still there.
6. GitHub: Settings → GitHub → Connect (device code) → publish a project; open the repository URL Genius reports.
7. If a view ever shows *Something went wrong in this view*, press **Copy details** and send them; the same report is saved (redacted) to `%APPDATA%\dev.genius.studio\logs\ui-errors.log`.

## Windows-specific behaviour already handled in code

- Paths: all project paths are validated as relative and normalised; drive letters, `\\?\` prefixes, reserved names (`CON`, `NUL`, …) and `..` are rejected.
- Processes: npm runs via `npm.cmd` without a shell; trees are killed with `taskkill /T /F` on stop and on exit.
- Credentials: `keyring` with the `windows-native` backend (Credential Manager).
