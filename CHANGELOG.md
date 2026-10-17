# Changelog

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

**Chat like with Claude**
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
