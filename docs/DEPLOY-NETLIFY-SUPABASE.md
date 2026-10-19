# Deploying Genius Web (Netlify + Supabase)

Genius Web is the browser version of Genius. One Netlify site serves:

| Address | What |
|---|---|
| `/` | public landing and download page |
| `/app/chat`, `/app/research`, `/app/code`, `/app/agents`, `/app/projects`, `/app/settings` | the signed-in app (single-page app, same code as the desktop interface) |
| `/admin` | owner admin panel (the database decides who may use it) |
| `/api/web` | web search and page reading (signed-in users only) |
| `/api/relay` | request relay for providers that block browsers (NVIDIA, custom endpoints) |

The desktop app (Tauri) is a separate pipeline and is not built on Netlify.

## Which branch to import

Import **`genius-web-launch`**. It contains tested Genius Web code; `main` is left as it was (it is what the desktop releases are cut from).

## Netlify build settings

Netlify reads `netlify.toml`, so the dashboard fields can stay empty. For reference:

| Setting | Value |
|---|---|
| Base directory | *(empty — repository root)* |
| Build command | `npm ci && npm run build:web` |
| Publish directory | `dist-web` |
| Functions directory | `netlify/functions` |
| Node | 20 (set in `netlify.toml`) |

## Environment variables (Netlify → Site configuration → Environment variables)

| Name | Value | Secret? |
|---|---|---|
| `VITE_SUPABASE_URL` | your project URL, `https://<ref>.supabase.co` | no (public) |
| `VITE_SUPABASE_ANON_KEY` | the project's **anon / publishable** key | no (public by design; protected by row level security) |

The functions read the same two values at runtime. **Never** add the service-role key to Netlify for this app: nothing needs it, and it must not reach a browser.

Local development: copy `.env.example` to `.env.local`, fill the two values, run `npm run dev:web`.
Without Supabase you can try the interface with `VITE_GENIUS_LOCAL=1 npm run dev:web` (development only: conversations stay in the browser, no web search, never enabled in production builds).

## Supabase

1. Use the organization you already have: create a project in it (or reuse one).
2. Apply the migrations in `supabase/migrations` in order (SQL Editor → paste each file → Run, or `supabase link` + `supabase db push`):
   `20261019000000_genius_web.sql`, `20261019000100_genius_web_storage.sql`, `20261019000200_missions.sql`, `20261020000000_admin_advanced.sql` (advanced admin: settings, limits, maintenance, announcements, quotas, bulk actions).
3. Authentication → URL Configuration:
   - **Site URL**: `https://<your-site>.netlify.app/app`
   - **Redirect URLs**: `https://<your-site>.netlify.app/app` (add your own domain too if you use one). Do not add wildcards such as `*`.
4. Authentication → Sign In / Providers → Email: enabled. **Confirm email: switch it OFF** if you want sign-up to open the app immediately (desktop and web). This is a Supabase setting that application code cannot change. Genius also works with it on (the sign-up screen then asks the user to open the e-mailed link). Administrators are never derived from e-mail addresses, so turning it off does not weaken the admin boundary.
5. Become the owner (one time, by user id — see `docs/ADMIN-BOOTSTRAP.md`): sign up, copy your **User UID** (Authentication → Users) and run
   `insert into public.admin_users (user_id) values ('USER-UID');`
   (Only this SQL path can create administrators; no client can. An existing row stays valid.)
6. Desktop sign-in uses the same project: set the GitHub repository **variables** `VITE_SUPABASE_URL` and `VITE_SUPABASE_ANON_KEY` (Settings → Secrets and variables → Actions → Variables) before cutting a release, or let users paste them once in the app (Settings → Account).

## What stays private

- AI provider keys: never stored in Supabase or Netlify; in the browser they live in the tab (sessionStorage) or — if the user ticks "Remember this key on this device" — encrypted (AES-GCM, non-exportable Web Crypto key) in that browser's IndexedDB, never synced; on the desktop in the OS credential store. They are sent only to the provider (directly, or through `/api/relay` for NVIDIA/custom endpoints, which forwards and stores nothing).
- Conversations, sources, files, preferences: stored per user; row level security allows only the owner to read or change them (tested on real PostgreSQL in `tests/db/rls.test.ts`).
- Administrators: the `admin_users` table has no client policy at all; all admin actions are database functions that re-check the caller and write `admin_audit`.

## Verified vs not yet verified

| Verified | Not yet |
|---|---|
| RLS and admin functions on a real PostgreSQL engine (PGlite) with the real migrations | The same migrations on a **live Supabase project**, sign-up emails, redirect URLs |
| Proxy/relay SSRF rules against a real local HTTP server; session and feature checks with a stand-in Supabase | The functions running on Netlify; Netlify function time limits with long streams |
| Browser CORS of the providers (preflight checked 2026-10-19) | A real chat from the browser to each provider |
| Interface in a real browser (local-only mode) | The deployed site |

After deploying, run the checklist: sign up → confirm email → sign in → paste a key (Settings) → Test connection → send a chat → run a research question → open `/admin` as owner → make a second account Pro → check Agent Mode appears for it.

## Limits to know

- Netlify functions have a maximum run time; a very long streamed NVIDIA answer through the relay can be cut. Use a direct provider (OpenAI, Anthropic, Gemini) for long answers.
- `/api/relay` accepts custom HTTPS endpoints that resolve to public addresses; the address is checked but not pinned for the connection (a DNS-rebinding window exists for custom endpoints; NVIDIA is allow-listed).
- Previews in the Code workspace show HTML and CSS only: the app's content-security policy blocks scripts inside previews.
