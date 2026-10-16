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

## Known limitations

- **One project type** (Vite + React + TypeScript web app). Desktop targets are shown as "not yet supported".
- **Whole-file AI edits.** Large files cost more tokens; patch mode is planned.
- **Model capability detection** is a probe for chat + streaming only; function calling / JSON mode are deliberately not used (text protocol works with every model).
- **Linux without a Secret Service** keeps the NVIDIA key in memory only (the UI says so); it must be re-entered after restart.
- **The element picker** describes the DOM element (selector, text, attributes, HTML); it does not yet map to an exact source line.
- **GitHub CLI** is required for GitHub features; without it, Settings explains how to install it. Plain git remotes still work.
- **Bundle size:** the UI bundle is ~1 MB (CodeMirror languages); acceptable for a desktop app, could be code-split.
