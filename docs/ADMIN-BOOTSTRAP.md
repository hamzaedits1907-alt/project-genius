# Owner / administrator bootstrap and authentication notes

## Rules

- Administrators are the rows of `public.admin_users`. The table has row level security and **no policy**: no client (anon or signed-in) can read, insert, update or delete it. Only the project owner can add a row, in the Supabase SQL Editor (or with the service role from a trusted place).
- An administrator is identified by the **authenticated user id** (`auth.users.id`, a UUID). It is never derived from an e-mail address. With e-mail confirmation turned off, anyone can register with any address, so an e-mail match must never grant privileges; there is deliberately no trigger or function that does so.
- The admin panel (`/admin`) and every privileged operation call `SECURITY DEFINER` functions that call `admin_require()` (which checks `is_admin()` on the server) and write `admin_audit` in the same transaction. Hiding the route is only convenience. The panel cannot create, promote or remove administrators.
- Suspended users stop being administrators (`is_admin()` requires an active profile); administrators cannot suspend themselves or each other from the panel.

## One-time procedure

1. Register an account normally (web `/app` or desktop Settings → Account).
2. Supabase → Authentication → Users → copy **User UID** of your account.
3. SQL Editor:
   ```sql
   insert into public.admin_users (user_id) values ('<USER UID>');
   ```
4. Reload the app. The sidebar shows “owner”; click it for `/admin`.

An already existing legitimate row is kept. To inspect: `select a.user_id, u.email, a.created_at from public.admin_users a join auth.users u on u.id = a.user_id;`

To remove an administrator (also only in the SQL Editor): `delete from public.admin_users where user_id = '<USER UID>';`

## E-mail confirmation

Genius works with confirmation **off** (supported setup) and also with it on (the sign-up screen then tells the user to open the e-mailed link). The switch is a Supabase project setting — Authentication → Sign In / Providers → Email → **Confirm email** — and cannot be changed from application code. Password reset e-mails keep working either way and are separate from registration.

Consequences of turning it off, and what Genius does about them:

| Risk | Mitigation |
|---|---|
| Anyone can register with someone else's address | No privilege depends on the address; admins are UUID rows only. The address is not proof of identity, so Genius does not use it for anything but sign-in and password reset. |
| Spam sign-ups | Admin panel → Site controls → **Close registrations** (a database trigger rejects new users); Supabase CAPTCHA/rate limits can be added in the dashboard. |
| Unverified users reaching expensive features | Genius features are BYOK; Genius-side caps are in Plans & limits (daily missions, web tool calls). |

## Desktop and web share one account system

Both use the same Supabase project (`VITE_SUPABASE_URL` / `VITE_SUPABASE_ANON_KEY`, both public). The desktop build gets them from the release workflow (repository variables with the same names) or the user can paste them once in Settings → Account; a service-role or secret key is refused by the app. Sessions persist in the app's local storage and refresh automatically; an expired session returns to the sign-in form.

## Tested vs not tested

- Tested (real PostgreSQL via PGlite, real migrations): no client path to `admin_users`, cross-user isolation, every admin function refuses non-admins, bulk actions, maintenance, sign-up switch, quotas, announcements, audit (`tests/db/rls.test.ts`).
- **Not tested**: a live Supabase project (e-mail settings, redirect URLs, the trigger on `auth.users` in the hosted database). See the checklist in `DEPLOY-NETLIFY-SUPABASE.md`.
