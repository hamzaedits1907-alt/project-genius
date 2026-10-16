# Ideas — beyond the MVP

Each idea is rated for **value** (to someone building real software with Genius), **complexity**, and a proposed **phase**. ✅ = implemented in this version.

## Implemented

| Idea | Value | Notes |
|---|---|---|
| ✅ **Build Doctor** | Very high | Parses real tsc/esbuild/Vite/Vitest output, sends only relevant files + focused log, records attempts, stops after N, forbids "fixes" that disable checks. |
| ✅ **Changeset history with revert and restore-to-point** | Very high | Works without Git. Every AI and manual edit is reviewable and reversible; stale proposals can't overwrite newer edits. |
| ✅ **Project memory** | High | Architecture decisions and conventions (seeded from the plan, user-editable) are included in every AI request so later changes stay consistent. |
| ✅ **Context transparency** | High | Before sending, the composer shows exactly which files (and how many KB) go to NVIDIA and why; files can be pinned or excluded. Secrets are never eligible. |
| ✅ **Readiness report** | High | Pass/fail/unknown from real results only: deps installed, last build/test (and whether files changed since), secret scan, README, Git clean, pushed, preview. |
| ✅ **Preview element picker** | High | Click an element in the running app → its description (tag, selector path, text, attributes, HTML) is attached to the next change request. Implemented with a serve-only Vite plugin loaded through a wrapper config in `node_modules/.genius/`, so the project itself is untouched. |
| ✅ **Script trust** | High (security) | npm scripts are trusted only if identical to the template; otherwise the exact script text must be approved before running. AI-written `package.json` can't add lifecycle scripts or non-registry deps. |
| ✅ **Genius-vs-external change attribution in Git** | Medium-high | The Git panel marks files changed by Genius since the last commit and offers "Stage Genius changes", so unrelated user edits aren't swept into AI commits. |
| ✅ **Model verification** | Medium | One-click 16-token probe proves a model answers chat and streams with *your* key; results are shown in the picker. |
| ✅ **Retry-safe GitHub publish** | Medium | Each step is persisted; retries reuse the created repository and verify the push with `git ls-remote`. |

## Next (Phase 7–8)

| Idea | Value | Complexity | Notes |
|---|---|---|---|
| **Source-mapped element picker** | High | Medium | Add a dev-only Babel/SWC transform that stamps `data-genius-src="file:line"` so the picker returns the exact component file and line, pinned into context automatically. |
| **Visual before/after comparison** | High | Medium | Screenshot the preview before applying a proposal (via the webview capture API) and after the verify build; show side by side in the review. |
| **Project understanding map** | High | Medium | Parse imports with a lightweight TS parser (or `es-module-lexer`) to show components, pages and dependencies as a graph; also improves context selection (follow imports). |
| **Patch-mode edits for large files** | High | Medium | For files over ~400 lines ask for search/replace blocks with exact anchors, validate they apply uniquely, fall back to whole-file. Cuts tokens and latency. |
| **Native function calling when verified** | Medium | Medium | Detect `tools` support per model (probe), and use structured tool calls for read/search/propose actions; keep the text protocol as fallback. |
| **Test generation & coverage loop** | High | Medium | "Add tests for this component" flow, run Vitest with coverage, feed uncovered lines back. |
| **Deploy preparation** | Medium | Low-Medium | Generate and validate config for static hosts (Netlify/Vercel/GitHub Pages) and run `vite build` + `vite preview` smoke check; never deploy without explicit action. |
| **Second project type: Tauri desktop app** | High | High | Template + Rust toolchain detection + `tauri build`. Only enable once builds are verified on all three OSes. |
| **Reusable component library across projects** | Medium | Medium | Save a component (with its CSS and tests) to a personal library and insert it into other projects as a reviewable changeset. |
| **Token/cost meter** | Medium | Low | Show `usage` returned by NVIDIA per task and per project. |
| **Dependency audit** | Medium | Low | Run `npm audit --json` on demand and summarise; propose upgrades as changesets. |
| **Multi-provider abstraction** | Low (for now) | Low | `nvidia.rs` is already OpenAI-compatible; a provider trait would allow self-hosted NIM profiles side by side. Deliberately not exposed: the spec requires NVIDIA. |

## Considered and rejected (for now)

* **Fully autonomous "agent loop" that runs arbitrary commands.** Conflicts with the safety model; the allowlisted operations + Build Doctor cover the real need.
* **Embedding a terminal emulator (PTY).** Useful for power users, but it would bypass approvals and the allowlist. The read-only, real-output terminal is the safer default.
* **Storing GitHub tokens ourselves via a custom OAuth app.** Requires shipping a client ID and duplicating what gh already does securely.
