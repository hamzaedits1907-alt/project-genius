# Changelog

## [0.7.0] — One account everywhere: synced conversations, keys that follow your account, Android app, access control

**Before you update the website/database:** run the three new SQL files in the Supabase SQL editor, in this order (the app keeps working on the old schema until you do): `20261021000000_sync.sql`, `20261022000000_key_vault.sql`, `20261023000000_admin_access.sql`.

- **Fixed: Genius Web showed a blank page right after sign-in.** A selector returned a new empty list on every render, which React 19 / zustand 5 treats as an endless loop (React error #185). Found by reproducing it on the live site; fixed, covered by a regression test, and a root error boundary now shows a message instead of a blank page. Also fixed on the web: the first message after connecting NVIDIA failed silently because the relay permission was hidden in Settings (it is now asked in the connect panel), and the "best model" it picked could be one that returns HTTP 404 (a model is now chosen only after it really answered).
- **Fixed: Chat answered "Which A340 do you mean?" instead of answering.** Chat now interprets the question, searches the web first for anything specific or recent, and never answers a factual question with only a question; if one still slips through it is nudged once automatically. It must never invent: when it cannot find something it says what the sources show and what stays unverified. Verified live with the real model and the real web (desktop core and deployed site).
- **Same conversations on every device.** Conversations and messages are stored per account with stable ids, tombstones for deleted conversations/messages and a deterministic order. The web reads and writes them directly (live refresh through Supabase Realtime, with polling as a fallback); the desktop app keeps a local copy that syncs both ways (outbox, merge, "newest title wins", unsent local edits are never overwritten), uploads the history already on the computer once (never duplicated), and asks before merging a computer that belongs to another account. Chat and Research conversations sync; Code projects, attachments and Code sessions stay where they were made.
- **Your AI keys follow your account (encrypted).** Settings → Account → Key vault: choose a passphrase once; keys are encrypted on your device (AES-GCM, PBKDF2 with 600,000 rounds) before they reach your account, so the database and the site owner see only ciphertext. Sign in on a new device, type the passphrase once, and the keys are there. There is no recovery for a forgotten passphrase (reset the vault and paste the keys again).
- **Android app.** The signed-in web app packaged with Capacitor: phone layout (menu drawer, safe areas, touch targets), keys and the vault passphrase in the Android Keystore / iOS Keychain, back button and keyboard handling. `Genius_0.7.0_android-debug.apk` is attached to the release (see docs/MOBILE.md: until release-signing secrets are added it is signed with a debug key and is named accordingly). The iOS project is included and compiles in CI; installing on an iPhone needs your Apple developer account.
- **Owner access control.** Admin → Access & invites: open to everyone / early access only / closed for everyone, a message for people who are locked out, invitations by e-mail (people who already have an account are let in at once, new ones the moment they register), a waitlist with Let in / Decline, per-user limit overrides, removal of a person's content (typed confirmation), more statistics. Enforced by the database, the web functions and the interface; administrators are never locked out.
- **Fixed: the owner admin panel in the desktop app.** It was never wired in (the account button always said "not an administrator"). Now opens from the account button for administrators; the desktop build ships the public account-service address so signing in works without pasting anything.
- **New tool: data_query.** Exact answers about an attached CSV/TSV/JSON table: filter, group, sum/mean/min/max/median/count, sort and list rows, computed from the whole file (same behaviour on the web and in the desktop core).
- **Code Mode: big apps no longer end as "cut off" drafts.** A plan is capped at 20 files, the app is written three files at a time with a digest of what already exists (dependencies first), truly cut-off files are rewritten alone, remaining gaps are closed, and a file that is followed by another file without an END line (or ends a normally finished answer) is no longer wrongly reported as cut off — this also affected the Build agent. Plus: a provider request that is accepted and never answered now fails after 90 s and moves to the next model instead of hanging.
- **Build and CI.** CI also runs the web and mobile builds, the database tests (77 on a real PostgreSQL engine), builds the debug APK and starts it on an Android emulator, and compiles the iOS project on a macOS runner.

## [0.6.0] — Code Mode repaired, accounts on desktop, advanced admin, Genius Web

- **Fixed: Code Mode could say "build succeeded" while the preview showed the starter page.** Three faults combined: the model's cut-off or forgotten files (typically `src/App.tsx`, which is written last) were silently dropped; a generation that was only *proposed* (waiting for approval) or whose build failed still counted as a finished task, so the interface jumped to a preview of the old files; and the verification built the starter template and called it a success. Now: Genius checks the output (cut-off files, a missing or starter `App.tsx`, imports that point at files nobody wrote) and asks the model for exactly those files, a bounded number of times; a project whose app is still the starter page **fails** instead of being built; the first application in a starter-only project is applied directly (every file stays revertible); a generation for a project with real code is explicitly *awaiting review* and a banner says the files are drafts that are not in the project; after the build Genius starts the preview, checks the dev server really serves the generated app and opens it in a real browser (when Chrome/Edge is installed) — only then is it reported as *verified*, otherwise *built*. A build run on the old files while edits are only staged no longer counts as verification. Regression: a full Personal Finance Tracker built through the real pipeline (real npm, tsc, Vite, Vitest, headless Chrome with add/edit/delete/search/persistence, review → apply, follow-up change).
- **Accounts on the desktop.** Sign in or register in Settings → Account with the same Supabase account as Genius Web (optional; Genius works without it). No e-mail confirmation step is required when the project has it switched off (a Supabase setting; see docs/KURULUM-TR.md). Clear messages for wrong password, existing account, network failure.
- **Provider key remembered.** Desktop: kept in the OS credential store across restarts (unchanged). Web: new "Remember this key on this device" — AES-GCM with a non-exportable Web Crypto key in IndexedDB; never sent to Supabase or a Genius server, never synced; session-only remains available.
- **Advanced owner admin.** Search/filter/sort/page users, bulk Pro/Free and cohort changes, per-user drawer (plan, grants, cohorts, private note, usage, history), usage charts, plan limits editor, maintenance mode, registration switch, announcements, audit filters and CSV export, read-only administrator list, system checks. All through audited, server-checked database functions (new migration `20261020000000_admin_advanced.sql`, 9 new database tests).
- **Public site.** New "Get a free NVIDIA API key" guide, Discord invite, no GitHub repository links.
- Earlier unreleased work, shipped now:

- **Genius Web.** A real browser version of Genius with accounts (Supabase), synced conversations, Chat, Research, Code (a code assistant that writes and previews code but cannot run it), Agent Mode (web tools only), Projects, Settings and an owner admin panel. Your provider key stays in the browser tab; OpenAI, Anthropic and Gemini are called directly, NVIDIA through a relay that stores nothing. See `docs/DEPLOY-NETLIFY-SUPABASE.md`.
- **Public site.** A landing and download page with real release links, provider instructions, changelog, FAQ and honest limits.
- **Fixed: malformed tool calls.** Calls written as `<web_search {"query": "…"}>`, `<websearch {…}` (no closing bracket) or `<todo>{…}</todo>` are now read as real calls; URLs in Markdown links are cleaned; raw tool markup can no longer appear in an answer (one correction round, then it is removed); repeated identical failures are blocked.
- **Fixed: research citations.** Citation numbers no longer become in-page anchors that the desktop WebView turned into `http://tauri.localhost/#src-1`; they link to the page that was really fetched, or are plain text. Links that point at the app or the computer are never shown as sources.
- **Fixed: identity.** The product creator is a setting (default Hamza) and is never confused with the user; the assistant keeps earlier conversation facts.
- **Research on URLs.** Script-built pages are labelled as such instead of guessed at (no invented logins or databases); metadata is included.
- **Agent Mode.** A new screen: describe a goal, Genius plans and starts it, shows steps, tool activity and approvals, and ends with a report that says *Completed, not verified* unless a real build or test passed. A live mission against NVIDIA was run (plan → isolated copy → approval → real file).
- **Interface refresh.** New visual system, model chip with provider diagnostics in the composer, Projects page, keyboard shortcuts (Ctrl+/).
- **Files.** Excel (.xlsx) files are read as tables.
- **Providers.** Auto model choice for OpenAI, Anthropic and Gemini (contract-tested only; no live call).
- **Project Brain.** Call relations: where a symbol is defined, which files use it, what a file calls.
- Tests: 18 row-level-security tests on a real PostgreSQL engine, proxy/relay SSRF tests against a real local server, web engine and runtime tests.

## [0.5.0]

Genius is a general-purpose assistant first; building apps is one of its tools.

- **New conversation-first interface.** A collapsible sidebar with a prominent *New chat*, workspaces (Chat, Research, Code, Agents), searchable history grouped by date, recent projects and a *Files* library. The home screen is a conversation with a welcome screen and a larger composer (attach, paste or drop files, Web on/off, Research). Messages render Markdown, code with copy, tables, **mathematics (KaTeX)**, picture thumbnails that open again from history, the web pages an answer really read, and an optional *Details* panel. Provider failures are explained with a Retry; running answers show their tool steps, a timer and Stop. The project builder moved under **Code**.
- **Fixed: wrong self-identification.** The chat prompt used to hard-code the model saved in Settings, while Auto routing or the provider could serve another — so an answer could claim one model while the message metadata showed another. The prompt no longer names a model; "who are you" is answered "I'm Genius, the AI assistant built into Project Genius". Only when asked which model it runs on does Genius read measured facts (`runtime_info`). Metadata keeps the **requested** model, the **provider-reported** label and per-request tokens and timings separately.
- **Fixed: images.** A picture now goes straight to a vision model as one request (no describe-then-answer round trip, no re-describing on every tool step); the type is read from the file's bytes, not its extension; the first image of a fresh install no longer fails because the model list had not been loaded yet; the picture is not re-uploaded with unrelated follow-ups; and a message was previously sent twice (once from history, once as the new turn). Measured live against NVIDIA with a red image named `blue.png`: 54.7 s → 9.3 s for the image answer, 79.5 s → 1.5 s for a greeting (single samples).
- **Everyday chat is faster by design.** A new `chat` routing role weighs speed more than agent work, effort follows the size of the request, and hidden reasoning is only used at Max effort.
- **New tools for chat:** `calculate` (exact arithmetic, no scripting) and `data_summary` (computed statistics of an attached CSV/TSV/JSON), plus `runtime_info`.
- **Evidence-based task lifecycle:** reports now say `verified`, `completed_unverified`, `failed` or `answered`.
- Fixed number formatting in the agents UI on non-English systems.
- **Project Brain.** Genius keeps an incrementally updated map of each project: type, entry points, build/test commands, modules, symbols with line numbers and the import graph (TypeScript, JavaScript, Python, Rust). Ask where something is, or what a change affects and which tests to run, in the new *Project Brain* panel or through the `project_brain` tool. Verified source facts and recorded history (memory, earlier changes, Git) are shown separately, and notes that mention files that no longer exist are flagged as possibly stale. It is a pattern scan, not a compiler.
- **Limits.** Settings, AI provider, Limits caps model calls, tool calls, minutes, tokens, concurrent tasks and provider retries per task. A task that hits a limit stops as *blocked*, names the limit and keeps its work. These are not a spending cap.
- **Honest usage.** Token counts are what the provider reports; requests without a report are shown as *estimated* and never mixed in.
- **Connection diagnostics.** One report replaces "Test connection": key, address, models, model access, a real test message, image input and (optionally) tool-call format, each with what to do next. The key is never shown.
- **More providers.** OpenAI, Anthropic and Google Gemini have native adapters (OpenAI-compatible endpoints too), each with its own key slot. They are covered by contract tests only; **no live call to them has been made**, and Auto model routing remains NVIDIA-only.
- **Tests:** fixed a flawed live NVIDIA test (the model was right to skip reading a file already in its prompt) and added a deterministic test for the cause.
- Windows updater: tested successfully by hand by the author (user-reported).

## [0.4.0]

Genius becomes a platform: chat, research, agents and a real browser.

- **Genius Chat and Genius Research.** A general-purpose assistant that needs no project, with saved conversations, search, attachments, tables and citations. Research reads real web pages and keeps a sources ledger: every citation points to a page that was actually fetched, uncited claims are flagged, and a research report can become a project spec.
- **Custom agents, teams and Mission Control.** Create agents with their own instructions, tools, folder scope, network permission, budget and approval mode; assign tasks or whole missions. A coordinator plans a dependency graph, tasks run in isolated copies of your project, conflicts are never overwritten, changes need your approval, and everything can be cancelled or retried. Statuses are real.
- **Genius can open your app in a real browser.** It checks the console, failed requests, interactions (click, type, assert), responsive layouts and accessibility, and reports exactly what was and was not checked.
- **Stronger regression tests.** A realistic dashboard (widgets, theme switch, persistence, mobile layout) is built, run in a real browser, changed and rolled back automatically, plus tests for missing exports, unresolved imports, wrong hook contracts and missing dependencies.
- **Safer web access.** More SSRF protections (IPv6 tricks, numeric hosts, mixed DNS answers) and prompt-injection notices for web pages.
- **Updates:** safer downloads (checked before they replace anything), automatic rollback, and Windows update verification in CI.

## [0.3.2]

Genius gets far more reliable at using its tools.

- **Tool calls that just work.** Genius understands every common way models write tool calls (JSON, OpenAI-style, XML-parameter styles, argument aliases, files as a map or list) and checks the arguments before running anything. A bad call gets one precise example back, never a silent failure.
- **No more loops.** The same failing call is blocked after two tries with clear guidance, repeated format mistakes switch Genius to the most robust file format, and every recovery is bounded.
- **Smarter Auto and Max.** Auto learns from what each model actually does on your key (speed, failures, bad tool calls, context limits) and avoids the ones that struggle. Max adds a planning pass and a final review for complex work. Works with no model selected.
- **Genius can look at your running app.** A new inspection tool checks what rendered, layout problems, accessibility (labels, contrast, headings) and mobile/desktop views in the real preview.
- **Dashboard regression test:** a real build (install, typecheck, build, dev server, follow-up change, rollback) driven through malformed tool calls and real compiler errors.

## [0.3.1]

- **Fixed:** steering Genius while it works, rewinding and compacting a conversation, and the slash-command menu were blocked by the app's permission list in 0.3.0 and did nothing. They work now.
- **Fixed (Windows):** crash locations from the preview showed full `C:\…` paths instead of project-relative ones, so Genius could not point at the failing file.
- Every release is now checked end to end on Windows (tests, the app's permission list, and the real installers) before it is published.

## [0.3.0]

Genius gets a new brain: faster, far more reliable, and it checks its own work.

**A smarter, faster agent**
- **Auto model (Max mode).** No more picking models: Genius chooses the best available NVIDIA model for each job (coding, planning, quick tasks, images) and switches automatically if one is busy or unavailable. Your favourite, Nemotron 3 Super, is first in line for coding.
- **Effort levels.** Fast, Balanced or Max (default): how hard Genius thinks, checks and reviews.
- **Live thinking.** Watch the model reason in real time ("Thought for 12s").
- **Precise edits.** Genius changes just the lines that need changing instead of rewriting whole files, which is much faster and safer.
- **Many actions at once.** Reads, writes and checks are batched in a single step instead of one at a time.
- **Never stuck on formatting.** Genius understands every common way models write tool calls and code, so a formatting slip no longer wastes a whole step.
- **Autopilot.** Genius applies its changes, runs the type check, reads the errors and fixes them itself before it answers. Every step is a checkpoint you can undo.
- **Sees your app crash.** Runtime errors, failed imports and blank (black/white) screens in the preview are captured. Genius can read them, and a **Fix it** button sends them straight to it.
- **Self-healing first builds.** A new app is type-checked and repaired automatically. When the Build Doctor can't produce a fix, the full agent takes over.
- **Skills.** Built-in expert playbooks for dashboards, landing pages, web apps, stores, games, portfolios and blogs, forms, animation, debugging and testing. Add your own in `.genius/skills/`.
- **GENIUS.md project instructions** that every task follows (`/init` writes one for you).
- **New specialist agents:** Explorer (maps big codebases) and Debugger (finds root causes).
- **Adds packages for you** (`add_dependency`) and installs them before checking.

**Chat like a modern assistant**
- **Slash commands:** /fix, /review, /test, /feature, /design, /explain, /refactor, /optimize, /a11y, /responsive, /seo, /darkmode, /docs, /security, /init, plus /plan, /effort, /autopilot, /undo, /compact, /clear, /model and /help.
- **@file mentions** pin files into the request.
- **Steer while it works:** send messages mid-task; Genius reads them after its current step.
- **Edit, retry, rewind:** edit any earlier message and resend, retry an answer, or rewind the conversation. File changes made since then are undone.
- **Compact** long conversations into a summary. A context meter shows when it's time.
- Token usage, model and duration under each answer.

**More**
- **Blueprint gallery** on Home: 18 detailed starting points (SaaS site, portfolio, dashboard, store, kanban, finance tracker, games…).
- **Improve my prompt:** turns a rough idea into a detailed brief.
- **Command palette** (Ctrl/Cmd+K) for everything.
- **Deploy your website** to GitHub Pages in one click.
- **Notifications** when Genius finishes while you're in another window.

## [0.2.1]

- **Skip this version.** Not ready to update? Skip a release and Genius won't nag you about it (undo any time in Settings → Updates & about).
- One-command releases with `scripts/release.sh`, with a fallback to GitHub's "Run workflow" button.

## [0.2.0]

Meet the new Genius.

- **Genius has a face.** An animated character that shows what it's doing — listening, thinking, working, happy when it's done.
- **Simple mode (new default).** Just a chat and your running app. Describe an idea on Home and Genius plans it, writes it and starts it for you. **Advanced mode** keeps the full IDE (files, terminal, Git, history). Switch any time from the top bar.
- **Quick setup.** Paste your NVIDIA key on Home and Genius picks a strong coding model for you.
- **Smarter agent.** Chats naturally, asks clarifying questions with one-click answers, keeps a live checklist, explains each step, and builds whole multi-page websites in batches.
- **More agents.** New designer and tester specialists join the architect, reviewer and UI experts.
- **Internet access.** Genius can search the web and read documentation (private networks are always blocked; turn it off in Settings).
- **Better design.** A built-in design guide so generated apps look polished out of the box.
- **Automatic updates.** Genius checks GitHub for new versions, downloads the right installer, verifies its SHA-256 checksum and updates itself.
