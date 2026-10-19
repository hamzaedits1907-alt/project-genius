-- Genius Web: accounts, synced conversations, preferences, entitlements, feature flags, early access
-- and the owner admin boundary.
--
-- Security model (read this before changing anything):
--   * Every table has row level security. A signed-in user can only touch rows whose user_id is theirs.
--   * Provider API keys are NEVER stored here. `user_preferences.settings` is non-secret configuration
--     (theme, model name, assistant name, ...) and a trigger rejects anything that looks like a key.
--   * Administrators are the rows of `admin_users`. That table has RLS enabled and NO policy, so no client
--     (anon or authenticated) can read or change it; rows are added by the project owner in the SQL editor.
--   * All administrative writes go through SECURITY DEFINER functions that check is_admin() on the
--     server and write an audit record in the same transaction. There is no client policy that lets a
--     user edit their own plan, status, grants or flags.
--   * The service-role key is not needed by the browser app and must never be shipped to it.


-- ---------------------------------------------------------------------------------------------- profiles
create table public.profiles (
  id uuid primary key references auth.users (id) on delete cascade,
  email text not null,
  display_name text check (display_name is null or char_length(display_name) <= 80),
  status text not null default 'active' check (status in ('active', 'suspended')),
  suspended_reason text,
  created_at timestamptz not null default now(),
  last_seen_at timestamptz
);

create table public.admin_users (
  user_id uuid primary key references auth.users (id) on delete cascade,
  created_at timestamptz not null default now()
);
alter table public.admin_users enable row level security; -- deliberately no policy

create function public.is_active() returns boolean
language sql stable security definer set search_path = public as $$
  select exists (select 1 from public.profiles p where p.id = auth.uid() and p.status = 'active');
$$;

create function public.is_admin() returns boolean
language sql stable security definer set search_path = public as $$
  select public.is_active() and exists (select 1 from public.admin_users a where a.user_id = auth.uid());
$$;

create function public.handle_new_user() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  insert into public.profiles (id, email) values (new.id, coalesce(new.email, ''))
  on conflict (id) do nothing;
  return new;
end;
$$;
create trigger on_auth_user_created after insert on auth.users for each row execute function public.handle_new_user();

alter table public.profiles enable row level security;
create policy profiles_select_own on public.profiles for select to authenticated using (id = auth.uid());
create policy profiles_select_admin on public.profiles for select to authenticated using (public.is_admin());
create policy profiles_update_own on public.profiles for update to authenticated using (id = auth.uid() and public.is_active()) with check (id = auth.uid());
-- A user may change their display name and nothing else (status and email are not theirs to edit).
revoke update on public.profiles from authenticated, anon;
grant update (display_name, last_seen_at) on public.profiles to authenticated;

-- ---------------------------------------------------------------------------------------------- conversations
create table public.conversations (
  id uuid primary key default gen_random_uuid(),
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  mode text not null check (mode in ('chat', 'research', 'code', 'agent')),
  title text not null default 'New conversation' check (char_length(title) <= 200),
  origin jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index conversations_user_updated on public.conversations (user_id, updated_at desc);

create table public.messages (
  id uuid primary key default gen_random_uuid(),
  conversation_id uuid not null references public.conversations (id) on delete cascade,
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  role text not null check (role in ('user', 'assistant', 'system')),
  kind text not null default 'chat',
  content text not null check (char_length(content) <= 400000),
  meta jsonb not null default '{}'::jsonb,
  task_id text,
  created_at timestamptz not null default now()
);
create index messages_conversation on public.messages (conversation_id, created_at);

create table public.research_sources (
  id uuid primary key default gen_random_uuid(),
  conversation_id uuid not null references public.conversations (id) on delete cascade,
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  task_id text,
  n integer not null,
  url text not null,
  requested_url text not null,
  title text not null default '',
  status integer not null default 200,
  fetched_at timestamptz not null default now(),
  content_hash text not null,
  snippet text not null default '',
  used_for text not null default '',
  flags jsonb not null default '[]'::jsonb,
  unique (conversation_id, n)
);

create table public.attachments (
  id uuid primary key default gen_random_uuid(),
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  conversation_id uuid references public.conversations (id) on delete cascade,
  name text not null check (char_length(name) <= 255),
  kind text not null,
  mime text not null,
  size bigint not null check (size >= 0),
  scope text not null default 'message',
  status text not null default 'ready',
  error text,
  text_content text not null default '' check (char_length(text_content) <= 2000000),
  image_path text, -- object path inside the private "attachments" bucket: <user_id>/<id>
  meta jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

-- Owner-only access with a writable-while-active rule for every user data table.
do $$
declare t text;
begin
  foreach t in array array['conversations', 'messages', 'research_sources', 'attachments'] loop
    execute format('alter table public.%I enable row level security', t);
    execute format('create policy %I on public.%I for select to authenticated using (user_id = auth.uid())', t || '_select_own', t);
    execute format('create policy %I on public.%I for insert to authenticated with check (user_id = auth.uid() and public.is_active())', t || '_insert_own', t);
    execute format('create policy %I on public.%I for update to authenticated using (user_id = auth.uid() and public.is_active()) with check (user_id = auth.uid())', t || '_update_own', t);
    execute format('create policy %I on public.%I for delete to authenticated using (user_id = auth.uid() and public.is_active())', t || '_delete_own', t);
  end loop;
end $$;

-- A message or source may only be attached to a conversation the same user owns.
create function public.require_own_conversation() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if new.conversation_id is not null and not exists (select 1 from public.conversations c where c.id = new.conversation_id and c.user_id = new.user_id) then
    raise exception 'conversation does not belong to this user' using errcode = '42501';
  end if;
  return new;
end;
$$;
create trigger messages_own_conversation before insert or update on public.messages for each row execute function public.require_own_conversation();
create trigger sources_own_conversation before insert or update on public.research_sources for each row execute function public.require_own_conversation();
create trigger attachments_own_conversation before insert or update on public.attachments for each row execute function public.require_own_conversation();

create function public.touch_conversation() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  update public.conversations set updated_at = now() where id = new.conversation_id;
  return new;
end;
$$;
create trigger messages_touch after insert on public.messages for each row execute function public.touch_conversation();

-- ---------------------------------------------------------------------------------------------- preferences
create table public.user_preferences (
  user_id uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  settings jsonb not null default '{}'::jsonb,
  updated_at timestamptz not null default now()
);
alter table public.user_preferences enable row level security;
create policy prefs_select_own on public.user_preferences for select to authenticated using (user_id = auth.uid());
create policy prefs_insert_own on public.user_preferences for insert to authenticated with check (user_id = auth.uid() and public.is_active());
create policy prefs_update_own on public.user_preferences for update to authenticated using (user_id = auth.uid() and public.is_active()) with check (user_id = auth.uid());
create policy prefs_delete_own on public.user_preferences for delete to authenticated using (user_id = auth.uid());

-- Defence in depth: provider keys must never be saved here, even by a buggy client.
create function public.reject_secrets() returns trigger
language plpgsql as $$
declare txt text := lower(new.settings::text);
begin
  if txt ~ '(sk-[a-z0-9_-]{16,}|nvapi-[a-z0-9_-]{10,}|sk-ant-[a-z0-9_-]{10,}|aiza[a-z0-9_-]{20,}|"api_?key"\s*:|"secret"\s*:|bearer\s+[a-z0-9._-]{20,})' then
    raise exception 'preferences must not contain provider keys or secrets' using errcode = '22023';
  end if;
  new.updated_at := now();
  return new;
end;
$$;
create trigger prefs_reject_secrets before insert or update on public.user_preferences for each row execute function public.reject_secrets();

-- ---------------------------------------------------------------------------------------------- usage counters
create table public.usage_events (
  id bigint generated always as identity primary key,
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  kind text not null check (kind in ('chat', 'research', 'agent', 'code', 'web_tool')),
  created_at timestamptz not null default now()
);
create index usage_events_user_time on public.usage_events (user_id, created_at desc);
alter table public.usage_events enable row level security;
create policy usage_select_own on public.usage_events for select to authenticated using (user_id = auth.uid());
create policy usage_insert_own on public.usage_events for insert to authenticated with check (user_id = auth.uid() and public.is_active());

-- ---------------------------------------------------------------------------------------------- features, plans, flags
-- A feature is something Genius offers. Its flag controls whether it is deployed (rollout, kill switch);
-- the entitlement system controls whether a given user may use it. The two are separate on purpose.
create table public.features (
  key text primary key,
  name text not null,
  description text not null,
  min_plan text not null default 'free' check (min_plan in ('free', 'pro'))
);
create table public.feature_flags (
  key text primary key references public.features (key) on delete cascade,
  enabled boolean not null default true,
  kill boolean not null default false,
  rollout_percent integer not null default 100 check (rollout_percent between 0 and 100),
  cohort_id text,
  note text,
  updated_at timestamptz not null default now(),
  updated_by uuid
);
create table public.early_access_cohorts (
  id text primary key check (id ~ '^[a-z0-9_-]{2,40}$'),
  name text not null,
  description text not null default '',
  created_at timestamptz not null default now()
);
create table public.cohort_members (
  cohort_id text not null references public.early_access_cohorts (id) on delete cascade,
  user_id uuid not null references auth.users (id) on delete cascade,
  added_at timestamptz not null default now(),
  primary key (cohort_id, user_id)
);
create table public.entitlements (
  user_id uuid primary key references auth.users (id) on delete cascade,
  plan text not null default 'free' check (plan in ('free', 'pro')),
  expires_at timestamptz,
  note text,
  updated_at timestamptz not null default now(),
  updated_by uuid
);
create table public.feature_grants (
  id uuid primary key default gen_random_uuid(),
  user_id uuid not null references auth.users (id) on delete cascade,
  feature text not null references public.features (key) on delete cascade,
  expires_at timestamptz,
  reason text,
  granted_by uuid,
  created_at timestamptz not null default now(),
  unique (user_id, feature)
);
create table public.admin_audit (
  id bigint generated always as identity primary key,
  at timestamptz not null default now(),
  actor uuid not null,
  action text not null,
  target_user uuid,
  detail jsonb not null default '{}'::jsonb
);

alter table public.features enable row level security;
alter table public.feature_flags enable row level security;
alter table public.early_access_cohorts enable row level security;
alter table public.cohort_members enable row level security;
alter table public.entitlements enable row level security;
alter table public.feature_grants enable row level security;
alter table public.admin_audit enable row level security;

-- Everyone signed in may read what the product offers; only their own entitlement rows; admins read all.
create policy features_read on public.features for select to authenticated using (true);
create policy flags_read on public.feature_flags for select to authenticated using (true);
create policy cohorts_admin_read on public.early_access_cohorts for select to authenticated using (public.is_admin());
create policy members_read on public.cohort_members for select to authenticated using (user_id = auth.uid() or public.is_admin());
create policy entitlements_read on public.entitlements for select to authenticated using (user_id = auth.uid() or public.is_admin());
create policy grants_read on public.feature_grants for select to authenticated using (user_id = auth.uid() or public.is_admin());
create policy audit_admin_read on public.admin_audit for select to authenticated using (public.is_admin());
-- No insert/update/delete policies on these tables: they change only through the functions below.

insert into public.features (key, name, description, min_plan) values
  ('chat', 'Chat', 'Conversations with your own model.', 'free'),
  ('research', 'Research', 'Source-grounded research with citations.', 'free'),
  ('web_tools', 'Web search and page reading', 'Search the web and read public pages.', 'free'),
  ('file_analysis', 'Files and images', 'Analyse images, PDFs, spreadsheets and documents.', 'free'),
  ('agent_mode', 'Agent Mode', 'Multi-step missions with a visible plan, tool activity and a verified report.', 'pro'),
  ('custom_agents', 'Custom agents and teams', 'Your own specialist agents.', 'pro'),
  ('extended_budgets', 'Higher task budgets', 'More model calls, tool calls and minutes per task (Genius limits, not provider billing).', 'pro'),
  ('project_workflows', 'Extended project workflows', 'Project memory and multi-step project workflows.', 'pro'),
  ('advanced_verification', 'Advanced verification', 'Automatic acceptance checks and verification reports.', 'pro')
on conflict (key) do nothing;
insert into public.feature_flags (key, enabled, rollout_percent)
  select key, true, 100 from public.features on conflict (key) do nothing;

-- Deterministic 0..99 bucket for staged rollouts.
create function public.rollout_bucket(uid uuid, feature text) returns integer
language sql immutable as $$
  select (('x' || substr(md5(uid::text || ':' || feature), 1, 8))::bit(32)::bigint % 100)::integer;
$$;

create function public.my_access() returns jsonb
language plpgsql stable security definer set search_path = public as $$
declare
  uid uuid := auth.uid();
  prof public.profiles;
  ent public.entitlements;
  plan_now text := 'free';
  result jsonb := '{}'::jsonb;
  f record;
  fl public.feature_flags;
  grant_ok boolean;
  plan_ok boolean;
  deployed boolean;
  reason text;
begin
  if uid is null then
    return jsonb_build_object('signedIn', false);
  end if;
  select * into prof from public.profiles where id = uid;
  select * into ent from public.entitlements where user_id = uid;
  if found and ent.plan = 'pro' and (ent.expires_at is null or ent.expires_at > now()) then
    plan_now := 'pro';
  end if;
  for f in select * from public.features order by key loop
    select * into fl from public.feature_flags where key = f.key;
    deployed := coalesce(fl.enabled, true) and not coalesce(fl.kill, false)
      and (coalesce(fl.rollout_percent, 100) >= 100
           or public.rollout_bucket(uid, f.key) < coalesce(fl.rollout_percent, 100)
           or (fl.cohort_id is not null and exists (select 1 from public.cohort_members m where m.cohort_id = fl.cohort_id and m.user_id = uid)));
    grant_ok := exists (select 1 from public.feature_grants g where g.user_id = uid and g.feature = f.key and (g.expires_at is null or g.expires_at > now()));
    plan_ok := f.min_plan = 'free' or plan_now = 'pro' or grant_ok;
    reason := case
      when prof.status is distinct from 'active' then 'suspended'
      when coalesce(fl.kill, false) then 'switched_off'
      when not deployed then 'not_rolled_out'
      when not plan_ok then 'needs_pro'
      else 'ok' end;
    result := result || jsonb_build_object(f.key, jsonb_build_object('available', reason = 'ok', 'reason', reason, 'name', f.name, 'description', f.description, 'minPlan', f.min_plan));
  end loop;
  return jsonb_build_object('signedIn', true, 'status', coalesce(prof.status, 'active'), 'plan', plan_now, 'planExpiresAt', ent.expires_at, 'isAdmin', public.is_admin(), 'features', result);
end;
$$;
grant execute on function public.my_access() to authenticated;

-- ---------------------------------------------------------------------------------------------- admin functions
create function public.admin_require() returns uuid
language plpgsql stable security definer set search_path = public as $$
begin
  if not public.is_admin() then
    raise exception 'administrator access required' using errcode = '42501';
  end if;
  return auth.uid();
end;
$$;

create function public.admin_log(act text, target uuid, detail jsonb) returns void
language sql security definer set search_path = public as $$
  insert into public.admin_audit (actor, action, target_user, detail) values (auth.uid(), act, target, coalesce(detail, '{}'::jsonb));
$$;
revoke all on function public.admin_log(text, uuid, jsonb) from public, anon, authenticated;

create function public.admin_list_users(search text default '', lim integer default 50)
returns table (id uuid, email text, display_name text, status text, plan text, plan_expires_at timestamptz, created_at timestamptz, conversations bigint, messages bigint, is_admin boolean)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query
    select p.id, p.email, p.display_name, p.status,
           case when e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()) then 'pro' else 'free' end,
           e.expires_at, p.created_at,
           (select count(*) from public.conversations c where c.user_id = p.id),
           (select count(*) from public.messages m where m.user_id = p.id),
           exists (select 1 from public.admin_users a where a.user_id = p.id)
    from public.profiles p left join public.entitlements e on e.user_id = p.id
    where search = '' or p.email ilike '%' || search || '%' or coalesce(p.display_name, '') ilike '%' || search || '%'
    order by p.created_at desc
    limit least(greatest(lim, 1), 200);
end;
$$;

create function public.admin_set_plan(target uuid, new_plan text, until timestamptz default null, note text default null) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if new_plan not in ('free', 'pro') then raise exception 'unknown plan' using errcode = '22023'; end if;
  insert into public.entitlements (user_id, plan, expires_at, note, updated_at, updated_by)
    values (target, new_plan, until, note, now(), auth.uid())
    on conflict (user_id) do update set plan = excluded.plan, expires_at = excluded.expires_at, note = excluded.note, updated_at = now(), updated_by = auth.uid();
  perform public.admin_log('set_plan', target, jsonb_build_object('plan', new_plan, 'until', until, 'note', note));
end;
$$;

create function public.admin_grant_feature(target uuid, feat text, until timestamptz default null, why text default null) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  insert into public.feature_grants (user_id, feature, expires_at, reason, granted_by) values (target, feat, until, why, auth.uid())
    on conflict (user_id, feature) do update set expires_at = excluded.expires_at, reason = excluded.reason, granted_by = auth.uid(), created_at = now();
  perform public.admin_log('grant_feature', target, jsonb_build_object('feature', feat, 'until', until, 'reason', why));
end;
$$;

create function public.admin_revoke_feature(target uuid, feat text) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  delete from public.feature_grants where user_id = target and feature = feat;
  perform public.admin_log('revoke_feature', target, jsonb_build_object('feature', feat));
end;
$$;

create function public.admin_set_flag(feat text, is_enabled boolean, is_kill boolean, rollout integer, cohort text default null, why text default null) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  update public.feature_flags set enabled = is_enabled, kill = is_kill, rollout_percent = rollout, cohort_id = cohort, note = why, updated_at = now(), updated_by = auth.uid() where key = feat;
  if not found then raise exception 'unknown feature' using errcode = '22023'; end if;
  perform public.admin_log('set_flag', null, jsonb_build_object('feature', feat, 'enabled', is_enabled, 'kill', is_kill, 'rollout', rollout, 'cohort', cohort, 'note', why));
end;
$$;

create function public.admin_set_status(target uuid, new_status text, why text default null) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if new_status not in ('active', 'suspended') then raise exception 'unknown status' using errcode = '22023'; end if;
  if target = auth.uid() then raise exception 'you cannot change your own account status' using errcode = '42501'; end if;
  if exists (select 1 from public.admin_users where user_id = target) then raise exception 'administrators cannot be suspended here' using errcode = '42501'; end if;
  update public.profiles set status = new_status, suspended_reason = case when new_status = 'suspended' then why else null end where id = target;
  perform public.admin_log('set_status', target, jsonb_build_object('status', new_status, 'reason', why));
end;
$$;

create function public.admin_save_cohort(cohort text, cohort_name text, cohort_desc text default '') returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  insert into public.early_access_cohorts (id, name, description) values (cohort, cohort_name, cohort_desc)
    on conflict (id) do update set name = excluded.name, description = excluded.description;
  perform public.admin_log('save_cohort', null, jsonb_build_object('cohort', cohort, 'name', cohort_name));
end;
$$;

create function public.admin_cohort_member(cohort text, target uuid, add_member boolean) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if add_member then
    insert into public.cohort_members (cohort_id, user_id) values (cohort, target) on conflict do nothing;
  else
    delete from public.cohort_members where cohort_id = cohort and user_id = target;
  end if;
  perform public.admin_log(case when add_member then 'cohort_add' else 'cohort_remove' end, target, jsonb_build_object('cohort', cohort));
end;
$$;

create function public.admin_usage_summary() returns jsonb
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return jsonb_build_object(
    'users', (select count(*) from public.profiles),
    'suspended', (select count(*) from public.profiles where status = 'suspended'),
    'pro', (select count(*) from public.entitlements where plan = 'pro' and (expires_at is null or expires_at > now())),
    'conversations', (select count(*) from public.conversations),
    'messages', (select count(*) from public.messages),
    'activeLast7Days', (select count(distinct user_id) from public.usage_events where created_at > now() - interval '7 days'),
    'eventsLast7Days', (select jsonb_object_agg(kind, n) from (select kind, count(*) n from public.usage_events where created_at > now() - interval '7 days' group by kind) k)
  );
end;
$$;

do $$
declare fn text;
begin
  foreach fn in array array[
    'admin_list_users(text, integer)', 'admin_set_plan(uuid, text, timestamptz, text)', 'admin_grant_feature(uuid, text, timestamptz, text)',
    'admin_revoke_feature(uuid, text)', 'admin_set_flag(text, boolean, boolean, integer, text, text)', 'admin_set_status(uuid, text, text)',
    'admin_save_cohort(text, text, text)', 'admin_cohort_member(text, uuid, boolean)', 'admin_usage_summary()', 'admin_require()'
  ] loop
    execute format('revoke all on function public.%s from public, anon', fn);
    execute format('grant execute on function public.%s to authenticated', fn);
  end loop;
end $$;

-- Table-level privileges: the API roles get only what the policies above allow them to use.
revoke all on all tables in schema public from anon;
revoke insert, update, delete on public.features, public.feature_flags, public.early_access_cohorts, public.cohort_members, public.entitlements, public.feature_grants, public.admin_audit from authenticated;
revoke all on public.admin_users from authenticated, anon;
