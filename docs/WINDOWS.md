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

## What changed for Windows in the effortless release (please verify)

- **Project paths.** Earlier versions stored project folders as `\\?\C:\Users\…` (Rust `canonicalize`). That path was used as the working directory of `npm.cmd`; `cmd.exe` does not accept such a directory and silently runs in `C:\Windows` instead, so `npm install` could not find `package.json`. Paths are now plain `C:\…` paths, and existing projects are rewritten the first time they are opened (nothing is moved or deleted).
- **How npm starts.** If `node.exe` has `node_modules\npm\bin\npm-cli.js` next to it (the standard Node.js installer layout), Genius runs `node.exe npm-cli.js <fixed arguments>` — no `cmd.exe` at all. Otherwise it falls back to `npm.cmd`, which Rust launches through `cmd.exe` with escaping; every argument is first checked against a strict character set (no spaces, quotes, `& | < > ^ % !`).
- **Locked files.** Before installing, Genius stops the project's own dev server (esbuild keeps files open, causing `EPERM`/`EBUSY`) and restarts the preview afterwards.
- **Ports.** `EACCES` on listen (Hyper-V/WSL reserved port ranges) is treated like a port collision and retried with a new port.

Additional checks for the checklist below: create a project from Home and confirm the Activity shows `Install dependencies … cwd C:\Users\…` (no `\\?\`), the preview shows the site, and closing Genius leaves no `node.exe` processes in Task Manager.

## Windows-specific behaviour already handled in code

- Paths: all project paths are validated as relative and normalised; drive letters, `\\?\` prefixes, reserved names (`CON`, `NUL`, …) and `..` are rejected.
- Processes: npm is launched as `node.exe npm-cli.js` or, as a fallback, `npm.cmd` (never the POSIX `npm` script or `npm.ps1`) with a fixed, validated argument list chosen by Genius — model output never becomes a command; process trees are killed with `taskkill /T` on stop and on exit.
- Credentials: `keyring` with the `windows-native` backend (Credential Manager).
