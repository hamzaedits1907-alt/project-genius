# Roadmap and priorities

Product direction (v0.5): **Genius is a general-purpose conversational AI first.** Building websites and applications is a capability (the Code workspace), not the product's identity. Audio, voice and microphone features are explicitly out of scope.

Architecture: **Genius Core** — four layers sharing one execution engine: Intelligence (chat, research, tools, files, images), Project Brain and memory, a reliable execution engine (evidence-based task state, budgets, checkpoints) and agents / Mission Control. The 100 capabilities below are product directions, not a one-release promise; related ones are built as subsystems, not as 100 panels.

Status words (grounded in evidence, see [STATUS.md](STATUS.md) and [TESTING.md](TESTING.md)):
**Implemented and verified** · **Implemented but incompletely verified** · **Partially implemented** · **Deferred** · **Not implemented** · **Blocked**. Currently blocked items: none.

## Summary
- Implemented and verified: **4**
- Implemented but incompletely verified: **7**
- Partially implemented: **39**
- Deferred: **1**
- Not implemented: **49**
- Blocked: **0**

## Genius Web (branch `genius-web-launch`)

| Area | Status |
|---|---|
| Browser app: Chat, Research (cited), Code (writes and previews code; runs nothing), Agent Mode (web tools only), Projects, Settings, Account | Implemented; engine and runtime tested with a scripted model and stand-in tools (`web-engine`, `web-chat`, `web-runtime`); the UI was looked at in a real browser in local-only mode. **Not yet run against a configured Supabase project or a live provider from the browser** |
| Accounts, synced history, preferences, entitlements, feature flags, early access, owner admin | Implemented as migrations + admin functions; **18 row-level-security tests pass on a real PostgreSQL engine (PGlite)**; not yet run on a live Supabase project |
| Web proxy (search, page reading) and provider relay (Netlify functions) | Implemented; SSRF rules and redirect handling tested against a real local HTTP server (`tests/netlify`); not deployed yet |
| Public landing / download site | Implemented; download links resolve to real release assets |
| Billing | **Not implemented on purpose** (no live billing until you authorise it) |

## Foundations (previous release)

| Foundation | Status |
|---|---|
| Conversation-first UI, model identity, image routing, Markdown/KaTeX, task lifecycle | Implemented; identity and image routing live-verified (single samples), the rest by mock tests |
| Execution budgets (model calls, tool calls, minutes, tokens, concurrency, provider retries) | Implemented and tested with a stand-in provider; stops as `blocked` and says which limit |
| Measured vs estimated token usage | Implemented; provider numbers are never invented or merged with estimates |
| Provider diagnostics (one report: key, address, models, model access, test message, vision, tool format) | Implemented; **live-verified on NVIDIA**, contract-tested for others |
| Provider adapters: NVIDIA, OpenAI, Anthropic (Messages), Gemini (generateContent), OpenAI-compatible | Implemented and contract-tested over HTTP against a stand-in; **no live call to OpenAI, Anthropic or Gemini has been made** |
| Project Brain v1 | Partially implemented (see #8) |
| Windows installer + updater | **User-reported successful manual test on Windows** (not re-run by the assistant); the CI Windows job and updater workflow are separate automated evidence |
| Agents and Mission Control | Implemented in 0.4.0; the Agent City-style control centre and certification remain deferred |

## Capabilities 1–20: Autonomous development

| # | Capability | Status | Evidence / what is missing |
|---|---|---|---|
| 1 | Genius Autopilot | Implemented but incompletely verified | Autopilot mode applies edits as checkpoints and verifies with real checks; covered by mock-model + real-tool tests (`dashboard`, `agent`). No live-model end-to-end site build was run in this release. |
| 2 | Auto/Max | Implemented and verified | Auto routing and effort levels: unit tests plus live NVIDIA (`live_nvidia`). NVIDIA is tuned (families, observed latency and failures); OpenAI, Anthropic and Gemini now pick the best known family among the models the key lists (`router.rs` unit tests, contract-level only: no live call to those providers). Custom endpoints run the model you choose. |
| 3 | Self-Healing Engine | Implemented but incompletely verified | Build Doctor and diagnostics repair real compiler errors in `dashboard` tests (6 repair scenarios) with a scripted model; not exercised with a live model. |
| 4 | Vision Loop | Implemented but incompletely verified | Real headless-browser inspection (console, network, interactions, responsive, a11y) passes `browser` tests; live image understanding verified once; the screenshot→vision-model→fix loop was not run live. |
| 5 | Website Factory | Partially implemented | Generates Vite + React + TypeScript projects with routes; one project type only. |
| 6 | Agent Swarm | Implemented but incompletely verified | Custom agents, teams and missions with dependencies: `missions` tests and 22 UI tests (scripted model). No live multi-agent mission. |
| 7 | Tool Intelligence | Implemented and verified | Strict argument validation, alias normalization, bounded recovery: unit tests; live tool call verified (`live_nvidia`, provider deep check). New `project_brain` tool follows the same schema path. |
| 8 | Project Brain | Partially implemented | v1 (0.5.0): incremental pattern-scan index (TS/JS/Python/Rust), entry points, commands, symbols, import graph, ranked retrieval, change impact, memory/changes/Git history kept separate and flagged stale. v2 (this branch): call relations — which files use a symbol and which symbols a file uses, preferring files that import the definition, with scope-aware answers (`brain.rs`, `tests/brain.rs`). Still a text-pattern scan: no types, macros or dynamic dispatch, no embeddings. |
| 9 | Time Machine | Implemented and verified | Changesets, diffs, revert, restore-to-point, stale-proposal refusal: unit and e2e tests on real files. |
| 10 | Visual Design Studio | Partially implemented | Element picker exists; it does not map to a source line. |
| 11 | Anything-to-App | Partially implemented | Building blocks only (image understanding + code generation). No dedicated reference-to-app workflow. |
| 12 | Genius Arena | Not implemented | No alternative-solution comparison. |
| 13 | Full-Stack Autopilot | Partially implemented | Frontend generation plus Supabase status/keys; no backend, auth or database generation. |
| 14 | One-Click Launch | Partially implemented | Publish to GitHub exists (fake-`gh` tests); no deployment. |
| 15 | Security Sentinel | Partially implemented | Secret scanning and SSRF hardening exist; no dependency/licence audit. |
| 16 | Research Brain | Implemented but incompletely verified | Research mode with sources ledger and citation validation (`chat_research` tests with mock pages, plus the same loop in Genius Web: `web-chat` tests). New: loose tool dialects (`<websearch {…}`, `<todo>{…}</todo>`) are parsed as calls, raw tool markup never reaches an answer, citations link to the fetched page (never `tauri.localhost`), script-built pages are labelled instead of guessed at. The live internet path was not exercised by these tests. |
| 17 | Codebase Rescue | Partially implemented | Build Doctor repairs existing projects; Project Brain now maps them; no dedicated import-and-rescue flow. |
| 18 | Mission Control | Implemented but incompletely verified | Mission board, controls, approvals: UI + `missions` tests with a scripted model. New Agent Mode screen (automatic start, approval card, activity log, honest report) and an opt-in **live** mission against NVIDIA (`live_mission`: plan in 2.5 s, one task, isolated copy, approval, real file created, 'verification unknown' reported). Control-centre visuals are not an Agent City. |
| 19 | Living Memory | Partially implemented | Project memory with provenance (user / repo / agent); Brain flags stale entries. No conflict resolution. |
| 20 | Genius Core | Partially implemented | Evidence-based lifecycle: verified, completed_unverified, answered, failed, blocked; the Agent Mode UI shows *Completed, not verified* unless a real build or test passed (`agent-mode` tests). Genius Web shares the same engine rules (tool protocol, ledger, limits). |

## Capabilities 21–40: Broader product

| # | Capability | Status | Evidence / what is missing |
|---|---|---|---|
| 21 | Genius Spec | Partially implemented | Project spec/plan generation exists; no requirements/acceptance-criteria workflow. |
| 22 | User Simulation Lab | Partially implemented | Browser interaction checks (click, type, assert); no persona journeys. |
| 23 | Native App Forge | Not implemented | — |
| 24 | Genius Game Studio | Not implemented | — |
| 25 | AI Asset Factory | Not implemented | — |
| 26 | API Integration Foundry | Not implemented | Only GitHub and Supabase status integrations. |
| 27 | Data Studio | Partially implemented | `data_summary` for attached CSV/TSV/JSON; no schemas, migrations or recovery. |
| 28 | Issue-to-PR Autopilot | Not implemented | — |
| 29 | Deep Review | Partially implemented | Reviewer subagent on large Max proposals (scripted-model test). |
| 30 | Multi-Repository Architect | Not implemented | — |
| 31 | Extension Marketplace | Not implemented | Local skills only. |
| 32 | Voice Pair Programmer | Deferred | Audio and voice are out of scope by decision. |
| 33 | Collaboration Hub | Not implemented | — |
| 34 | Global Quality Lab | Partially implemented | Browser a11y checks (labels, contrast, headings) and responsive views; no localization/keyboard audit. |
| 35 | Performance Lab | Not implemented | — |
| 36 | Production Observer | Not implemented | No telemetry by design. |
| 37 | Genius Sandbox | Partially implemented | Workspace path sandbox, fixed command allow-list, approvals, per-agent filesystem/network scope. No OS-level isolation. |
| 38 | Model Economics | Partially implemented | Per-model success/latency stats and per-request usage; new budgets and measured-vs-estimated usage. No cost view (BYOK). |
| 39 | Release Engineer | Not implemented | Genius's own release pipeline exists; it is not a feature for user projects. |
| 40 | Project Guardian | Not implemented | — |

## Capabilities 41–60: Reliability and operations

| # | Capability | Status | Evidence / what is missing |
|---|---|---|---|
| 41 | CI Failure Detective | Not implemented | — |
| 42 | Test Architect | Not implemented | — |
| 43 | Browser DevTools Analyst | Implemented and verified | Console, failed requests, DOM, interactions via a real browser: `browser` (6) and `dashboard` (8) tests. |
| 44 | Environment Doctor | Partially implemented | Tool detection, capability report and the new provider diagnostics; no runtime/toolchain repair. |
| 45 | Dependency Migration Engineer | Not implemented | — |
| 46 | API Contract Guardian | Not implemented | — |
| 47 | Database Migration Rehearsal | Not implemented | — |
| 48 | Data Pipeline Builder | Not implemented | — |
| 49 | Multi-Tenant SaaS Architect | Not implemented | — |
| 50 | Workflow Engine | Partially implemented | Missions with dependencies, retries and budgets; no durable queues or scheduling. |
| 51 | Safe Rollout Engineer | Not implemented | — |
| 52 | Regression Bisector | Not implemented | — |
| 53 | Reproducible Builds | Not implemented | Genius releases carry a manifest with commit and run URL; not a product feature. |
| 54 | Environment Parity Checker | Not implemented | — |
| 55 | Test Data Factory | Not implemented | — |
| 56 | Privacy-Safe Analytics | Not implemented | No analytics exist. |
| 57 | Design System Governance | Not implemented | — |
| 58 | License and IP Auditor | Not implemented | — |
| 59 | Feedback-to-Fix | Not implemented | — |
| 60 | Project Recovery Mode | Partially implemented | Checkpoints, rewind, stale-task cleanup after restart; interrupted work is not resumed. |

## Capabilities 61–80: Advanced features

| # | Capability | Status | Evidence / what is missing |
|---|---|---|---|
| 61 | Genius Desktop Operator | Not implemented | — |
| 62 | Genius Live Artifacts | Not implemented | Tables, Markdown and KaTeX render; no interactive charts or mini-apps. |
| 63 | Cross-AI Importer | Not implemented | — |
| 64 | Genius Workflow Recorder | Not implemented | Slash commands and skills exist. |
| 65 | Genius Transparent Brain | Partially implemented | Reports list files read, checks, failures, usage and what is not verified; hidden reasoning is not exposed. |
| 66 | Pixel-Perfect Design Lab | Not implemented | — |
| 67 | Project Archaeologist | Partially implemented | Project Brain overview and impact map unfamiliar repositories; single project, no cross-repo. |
| 68 | Genius Fork Worlds | Partially implemented | Agent tasks run in isolated workspaces; no parallel alternative implementations. |
| 69 | Genius Proof Ledger | Partially implemented | Each report records real exit codes/checks; no cross-task ledger. |
| 70 | Genius Project Universe | Not implemented | Brain is per project by design. |
| 71 | Genius Privacy Shield | Partially implemented | Secret redaction in logs/events, secret scan, `.env` never sent; no PII detection. |
| 72 | Genius Memory Doctor | Partially implemented | Stale-memory flags in Brain answers (missing files, old agent inference); no conflict detection or cleanup UI. |
| 73 | Agent Certification Lab | Not implemented | — |
| 74 | Genius Connection Doctor | Implemented but incompletely verified | Provider diagnostics: live NVIDIA passes; Anthropic/Gemini/OpenAI by contract tests only. Genius Web adds browser-access facts verified against the real endpoints (preflight 2026-10-19): OpenAI, Anthropic and Gemini allow browser calls, NVIDIA does not (relay required). |
| 75 | Genius Design DNA | Not implemented | — |
| 76 | Genius Model Tournament | Not implemented | — |
| 77 | Genius Learn Mode | Not implemented | — |
| 78 | Genius Bug Capsule | Partially implemented | Redacted UI error log with Copy details; no packaged bundle. |
| 79 | Genius Handover Kit | Partially implemented | Project export without secrets; no handoff documentation. |
| 80 | Genius Command Bar | Partially implemented | Quick open and slash commands; no global command palette. |

## Capabilities 81–100: Additional product ideas

| # | Capability | Status | Evidence / what is missing |
|---|---|---|---|
| 81 | Genius Conversation Branches | Not implemented | Rewind exists. |
| 82 | Context Cockpit | Not implemented | Only a context-budget setting. |
| 83 | Genius Intent Router | Partially implemented | Request complexity → effort and tool use by the model; no explicit router. |
| 84 | Genius Action Preview | Partially implemented | File changes and dependency installs are previewed/approved; other actions are not. |
| 85 | Genius Model Handoff | Partially implemented | Conversations are provider-independent and tools are text-based, so switching provider continues a conversation; not tested mid-conversation. |
| 86 | Genius Universal Search | Partially implemented | Conversation search (titles and message text, desktop and web), file search and quick open; not unified. |
| 87 | Genius Decision Ledger | Partially implemented | Memory entries of kind decision are recalled by the Brain; no dedicated ledger. |
| 88 | Genius Screen Understanding | Partially implemented | Screenshots to vision models; not combined with accessible UI info. |
| 89 | Genius Golden Journeys | Not implemented | — |
| 90 | Genius Code-to-Docs | Not implemented | — |
| 91 | Genius Compatibility Matrix | Not implemented | Genius's own CI builds Linux/Windows/macOS; not a product feature. |
| 92 | Genius API Sandbox Generator | Not implemented | — |
| 93 | Genius Change Impact Forecast | Partially implemented | Import-graph impact and tests to run (Brain); explicitly a lower bound. |
| 94 | Genius Recovery Journal | Partially implemented | Task events are persisted; no resume. |
| 95 | Genius Permission Simulator | Not implemented | Permissions are enforced and tested, not simulated. |
| 96 | Genius Model Economics Advisor | Not implemented | Stats exist; no advisor. |
| 97 | Genius Quality Profiles | Partially implemented | Fast/Balanced/Max set verification rounds; no custom profiles. |
| 98 | Genius Provider Resilience | Partially implemented | Retries, model fallback chains (also in Genius Web), rejected-parameter memory, classified errors; no cross-provider failover. |
| 99 | Genius Release Intelligence | Not implemented | Changelog is written by hand. |
| 100 | Genius Outcome Benchmark | Not implemented | — |

