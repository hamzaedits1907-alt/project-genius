# Status

Legend: ✅ done and verified by automated tests · 🟡 implemented, verified only partially (see note) · ⛔ blocked in the build environment · ⏳ planned

## MVP acceptance (the 15-step workflow)

| # | Step | Status | Evidence |
|---|---|---|---|
| 1 | Open Genius | ✅ | UI test launches the real Tauri app |
| 2 | Enter NVIDIA key in Settings | ✅ | UI test; key goes to the OS credential store (memory-only fallback, clearly labelled, when none exists) |
| 3 | Securely save + test connection | 🟡 | Save/mask/remove/test verified against a mock; **live NVIDIA auth unverified** (host blocked by the sandbox network policy) |
| 4 | Select a model from a working picker | 🟡 | Picker, refresh, cache/staleness, verify-probe tested against a mock `/v1/models`; **live catalog unverified** |
| 5 | Describe a small web app | ✅ | UI test |
| 6 | Structured plan | ✅ | Rust + UI e2e (mock model output, real parsing/validation/editing) |
| 7 | Real source files in a workspace | ✅ | Files written to disk, shown in tree/editor, streamed live as drafts |
| 8 | Inspect and edit files | ✅ | CodeMirror editor; saves recorded as revertible changesets |
| 9 | Genius starts the real app | ✅ | Real `npm install` → `tsc && vite build` → `vite` dev server |
| 10 | Live app in the preview panel | ✅ | UI test interacts with the running app inside the iframe; HMR verified |
| 11 | Request a change, review the diff | ✅ | Proposed changeset, side-by-side unified diff, apply/reject |
| 12 | Genius runs a build/test | ✅ | Verify build after apply; Vitest run of generated tests |
| 13 | Diagnose a real failure, propose a fix | ✅ | Rust e2e: real TS2304 → Build Doctor → fix → new real build passes |
| 14 | Recover previous changes, export | ✅ | Revert, restore-to-point, zip/folder export |
| 15 | Close and reopen without losing work | ✅ | Rust + UI e2e restart checks |

**Not verified anywhere yet:** live NVIDIA requests, live GitHub sign-in/push to github.com, Windows/macOS runtime (Windows target type-checks cleanly via cross-`cargo check`), OS keychains other than the fallback path.

## Phases

| Phase | Status | Notes |
|---|---|---|
| 0 Audit & architecture | ✅ | Empty repo; Tauri 2 / React / Rust / SQLite chosen; `docs/architecture.md` |
| 1 Desktop foundation | ✅ | Shell, navigation, design system (dark + light), settings, SQLite migrations, error boundary, toasts, shortcuts |
| 2 NVIDIA provider | 🟡 | Complete and mock-tested; live calls unverified |
| 3 Project generation | ✅ | Plan → template → streamed AI files → validation |
| 4 Execution & preview | ✅ | Allowlisted processes, process groups, bounded logs, isolated preview |
| 5 Editing & Build Doctor | ✅ | Changesets, diffs, context preview, diagnostics, bounded repair loop |
| 6 Recovery & export | ✅ | History, revert, restore, export, shutdown cleanup, crash-state cleanup |
| Addendum: live activity | ✅ | Typed persisted events, filters, detail inspector, streaming drafts, terminal |
| Addendum: Git | ✅ | Real git; status/diff/stage/commit/branches/fetch/pull/push/conflicts/history; tested on real repos |
| Addendum: GitHub | 🟡 | gh-based auth, repo list/clone/create, publish with verified push. Publish flow tested end-to-end with a fake `gh` + local bare remotes; **live github.com unverified** (device-code endpoint fails through the sandbox proxy) |
| 7 Advanced | 🟡 | Done: project memory, readiness report, element picker, context transparency, script trust, change attribution. Planned: see `docs/IDEAS.md` |

## Evolution release (conversations, agent, imports, integrations)

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | `destroy is not a function` fixed + regression test | ✅ / 🟡 | Mechanism reproduced in a test (React 19 calls a non-function effect return on unmount). The three effects that could leak a value were converted; ESLint now rejects expression-bodied, async and by-reference effects (verified to flag the original code); the racy event subscription was replaced. **The exact crash did not reproduce on Linux** (dev and production builds, full UI e2e); the boundary now records the failing component to `logs/ui-errors.log` for confirmation on Windows. |
| 2 | Tauri app starts on Windows | ⛔ | Not run on Windows here; Windows target type-checks. See `docs/WINDOWS.md`. |
| 3–4 | Create a project, continue the conversation with a targeted change | ✅ | Rust agent tests + real-app UI e2e (mock model) |
| 5 | Agent inspects code before modifying it | ✅ | Tool steps recorded and asserted (search → read_file → proposal) |
| 6 | Import a file and reference it | ✅ | Parser/zip tests; attachments sent with a message; reference search tool. UI drag-and-drop tested only in jsdom. |
| 7 | Review, apply, reject, verify | ✅ | Existing changeset flow, UI e2e |
| 8 | Capabilities reflect real availability | ✅ | `capabilities_reflect_runtime_state` |
| 9 | Subagent assignments/results recorded | ✅ | Reviewer run asserted; steps stored with agent role |
| 10–11 | GitHub failures handled / real push | 🟡 | Existing fake-`gh` publish test; **live github.com push not verified** (no credentials in this environment) |
| 12 | Supabase connected vs unconfigured | 🟡 | Status distinguishes not connected / connected / linked / configured; key selection and validation unit-tested; **no live Supabase call made** |
| 13 | Secrets absent from logs, prompts, bundle, commits | ✅ (scoped) | Agent test asserts `.env` content never reaches the provider; redaction tests; frontend bundle scanned for key patterns; commit secret scan (existing) |
| 14 | Existing projects/history intact | ✅ | v1→v2 migration test keeps messages |
| 15 | Restart preserves data and conversations | ✅ | Rust test + UI e2e restart |

## Effortless release (dependency/install/build/preview reliability, auto-repair, beginner workspace)

### Root causes found in the code (not assumed)

| Reported symptom | Real cause | Fix |
|---|---|---|
| "Dependencies are not installed yet. Run Install first." | `start_preview` refused to run when an mtime comparison said an install was needed; nothing installed automatically | Preview installs automatically (`ensure_installed`), driven by a dependency fingerprint (`deps.rs`) |
| Dependency installation failing during recovery on Windows (likely) | Project roots were stored/used as `canonicalize()`d paths, i.e. `\\?\C:\…` on Windows; that path was the cwd for `npm.cmd`, and `cmd.exe` refuses verbatim/UNC cwds and falls back to `C:\Windows` | `dunce`-simplified paths everywhere a path leaves the sandbox; legacy stored paths are rewritten on open; npm runs as `node.exe npm-cli.js` when available. **Not reproduced on Windows here** (no Windows host) — see WINDOWS.md |
| "Preview live at localhost" but the site is broken; missing `uuid` | Readiness was a TCP connect only. Reproduced with real Vite 7: the server listens while `/src/App.tsx` answers 500 `Failed to resolve import "uuid"` | Readiness = HTTP 200 + per-run identity header + module-graph compile check; compile errors are shown with the exact message and file |
| Conflicting / duplicated `package.json` entries | `serde_json` (and npm's `JSON.parse`) silently keep the last duplicate key, so a second `"dependencies"` block wiped the first | Strict JSON parsing with duplicate-key rejection for every proposal and for the dependency state |
| `build failed` hid the preview | `verify_pipeline` never started the preview when the production build failed | Build and preview are judged and reported separately |
| Stale/duplicate preview state | A single `previews` URL map and an exit watcher that could clear a newer server's state | Per-project state machine with generations; events of older generations are ignored |

### Acceptance criteria

| Criterion | Status | Evidence |
|---|---|---|
| Create a project without npm commands | ✅ | `start_create_app`; real-app UI test creates "Small Todo List" from Home → plan → code → real install/build → preview |
| Successful project displays its running site inside Genius | ✅ | Lifecycle test renders it in headless Chromium; UI test interacts with it inside the app's iframe |
| Follow-up request modifies the same project and updates the preview | ✅ | Lifecycle test (HMR, same run id) + UI test (footer appears after Apply) |
| Missing dependencies detected correctly | ✅ | `deps.rs` tests (missing, version change, lockfile change, scoped names, deleted node_modules); lifecycle test step 2 and 8 |
| Malformed proposals rejected before damaging files | ✅ | `proposal.rs` tests; lifecycle test step 9 (files byte-identical afterwards); UI test sees the block in Activity |
| Build/runtime failures have clear diagnostics and a repair workflow | ✅ | Compile error with file + message; `TypeScript error in src/App.tsx line 4: …`; Fix automatically repaired a missing dependency in both the lifecycle test and the real app |
| Project and conversation survive restarts | ✅ | Lifecycle test step 13; UI test restart |
| Existing features remain accessible | ✅ | All earlier Rust/agent/publish/e2e tests pass unchanged; UI test exercises terminal, history, Git commit, readiness; view walk covers every panel |
| Windows desktop runtime | ⛔ | Not available here. Windows target type-checks (`cargo check --target x86_64-pc-windows-gnu`); Windows npm resolution is unit-tested on Linux; the `\\?\` cwd fix is reasoned from documented `cmd.exe` behaviour, not observed |
| Live NVIDIA / GitHub / Supabase | ⛔ | Unchanged from before; no credentials or network access to those services in this environment |

## Known limitations

- **Runtime errors after load** are reported by the preview page itself; *Fix automatically* can repair them, but Genius cannot replay your clicks, so it asks you to reload and confirm.
- **Phantom dependencies**: a package that is only installed transitively (e.g. `nanoid` via Vite) works in the preview; proposals that import it without declaring it are still blocked.
- **One project type** (Vite + React + TypeScript web app). Desktop targets are shown as "not yet supported".
- **Whole-file AI edits.** Large files cost more tokens; patch mode is planned.
- **Model capability detection** is a probe for chat + streaming only; function calling / JSON mode are deliberately not used (text protocol works with every model).
- **Linux without a Secret Service** keeps the NVIDIA key in memory only (the UI says so); it must be re-entered after restart.
- **The element picker** describes the DOM element (selector, text, attributes, HTML); it does not yet map to an exact source line.
- **GitHub CLI** is required for GitHub features; without it, Settings explains how to install it. Plain git remotes still work.
- **Bundle size:** the UI bundle is ~1 MB (CodeMirror languages); acceptable for a desktop app, could be code-split.
