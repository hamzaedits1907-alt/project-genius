-- Advanced owner administration: plan limits, site controls (maintenance, sign-ups), announcements,
-- private admin notes, richer user search, bulk actions, usage analytics and quota enforcement.
--
-- Same rules as the first migration: every table has row level security, clients never write these
-- tables directly, and every administrative change goes through a SECURITY DEFINER function that checks
-- is_admin() on the server and writes an audit record. Nothing here can grant the administrator role:
-- admin_users is still filled only by the owner in the SQL editor (docs/ADMIN-BOOTSTRAP.md).

-- ---------------------------------------------------------------------------------------------- settings
-- Non-secret deployment settings. Readable by every signed-in user (the app needs maintenance mode and the
-- limits), writable only through admin_set_setting. NEVER put keys or secrets here.
create table public.app_settings (
  key text primary key check (key ~ '^[a-z0-9_]{2,40}$'),
  value jsonb not null,
  updated_at timestamptz not null default now(),
  updated_by uuid
);
alter table public.app_settings enable row level security;
create policy app_settings_read on public.app_settings for select to authenticated using (true);
revoke insert, update, delete on public.app_settings from authenticated, anon;

insert into public.app_settings (key, value) values
  ('maintenance', '{"enabled": false, "message": ""}'),
  ('signups', '{"open": true}'),
  -- Limits are Genius-controlled caps (null = no cap). They bound Genius features, never provider billing.
  ('limits_free', '{"maxModelCalls": 60, "maxToolCalls": 80, "maxTaskMinutes": 15, "maxConcurrentTasks": 2, "maxRetries": 2, "contextChars": 120000, "dailyAgentMissions": 3, "dailyWebToolCalls": 150, "dailyResearch": 10}'),
  ('limits_pro', '{"maxModelCalls": 100, "maxToolCalls": 150, "maxTaskMinutes": 45, "maxConcurrentTasks": 6, "maxRetries": 3, "contextChars": 400000, "dailyAgentMissions": null, "dailyWebToolCalls": 1500, "dailyResearch": null}')
on conflict (key) do nothing;

-- ---------------------------------------------------------------------------------------------- announcements
create table public.announcements (
  id uuid primary key default gen_random_uuid(),
  title text not null check (char_length(title) between 1 and 120),
  body text not null default '' check (char_length(body) <= 2000),
  level text not null default 'info' check (level in ('info', 'warning', 'critical')),
  active boolean not null default true,
  starts_at timestamptz not null default now(),
  ends_at timestamptz,
  created_at timestamptz not null default now(),
  created_by uuid
);
alter table public.announcements enable row level security;
create policy announcements_read on public.announcements for select to authenticated
  using (public.is_admin() or (active and starts_at <= now() and (ends_at is null or ends_at > now())));
revoke insert, update, delete on public.announcements from authenticated, anon;

-- ---------------------------------------------------------------------------------------------- admin notes
create table public.admin_notes (
  user_id uuid primary key references auth.users (id) on delete cascade,
  note text not null default '' check (char_length(note) <= 4000),
  updated_at timestamptz not null default now(),
  updated_by uuid
);
alter table public.admin_notes enable row level security;
create policy admin_notes_read on public.admin_notes for select to authenticated using (public.is_admin());
revoke insert, update, delete on public.admin_notes from authenticated, anon;

-- ---------------------------------------------------------------------------------------------- maintenance mode
-- my_access() is replaced so that, during maintenance, every feature reports "maintenance" for ordinary
-- users (administrators keep working so they can end it). The web proxy reads the same answer.
create or replace function public.my_access() returns jsonb
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
  maint jsonb := coalesce((select value from public.app_settings where key = 'maintenance'), '{"enabled": false}');
  in_maintenance boolean;
begin
  if uid is null then
    return jsonb_build_object('signedIn', false);
  end if;
  select * into prof from public.profiles where id = uid;
  select * into ent from public.entitlements where user_id = uid;
  if found and ent.plan = 'pro' and (ent.expires_at is null or ent.expires_at > now()) then
    plan_now := 'pro';
  end if;
  in_maintenance := coalesce((maint ->> 'enabled')::boolean, false) and not public.is_admin();
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
      when in_maintenance then 'maintenance'
      when coalesce(fl.kill, false) then 'switched_off'
      when not deployed then 'not_rolled_out'
      when not plan_ok then 'needs_pro'
      else 'ok' end;
    result := result || jsonb_build_object(f.key, jsonb_build_object('available', reason = 'ok', 'reason', reason, 'name', f.name, 'description', f.description, 'minPlan', f.min_plan));
  end loop;
  return jsonb_build_object('signedIn', true, 'status', coalesce(prof.status, 'active'), 'plan', plan_now, 'planExpiresAt', ent.expires_at, 'isAdmin', public.is_admin(),
    'maintenance', case when in_maintenance then maint else null end, 'features', result);
end;
$$;
grant execute on function public.my_access() to authenticated;

-- What the app needs to render: the caller's limits and the announcements currently in force.
create function public.my_context() returns jsonb
language plpgsql stable security definer set search_path = public as $$
declare
  pro boolean := exists (select 1 from public.entitlements e where e.user_id = auth.uid() and e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()));
begin
  if auth.uid() is null then return jsonb_build_object('signedIn', false); end if;
  return jsonb_build_object(
    'limits', (select value from public.app_settings where key = case when pro then 'limits_pro' else 'limits_free' end),
    'announcements', coalesce((select jsonb_agg(jsonb_build_object('id', a.id, 'title', a.title, 'body', a.body, 'level', a.level) order by a.created_at desc)
       from public.announcements a where a.active and a.starts_at <= now() and (a.ends_at is null or a.ends_at > now())), '[]'::jsonb));
end;
$$;
grant execute on function public.my_context() to authenticated;

-- ---------------------------------------------------------------------------------------------- sign-ups
create function public.enforce_signups() returns trigger
language plpgsql security definer set search_path = public as $$
begin
  if coalesce((select (value ->> 'open')::boolean from public.app_settings where key = 'signups'), true) = false then
    raise exception 'New registrations are closed right now.' using errcode = '42501';
  end if;
  return new;
end;
$$;
create trigger auth_users_signups before insert on auth.users for each row execute function public.enforce_signups();

-- ---------------------------------------------------------------------------------------------- quota enforcement
-- Genius-feature use that is recorded server-side is capped per plan. The cap is only as strong as the
-- recording: provider spending (BYOK) is invisible here, and a modified client that never records an event
-- is not stopped by this trigger — expensive Genius operations also check at the web proxy.
create function public.enforce_usage_quota() returns trigger
language plpgsql security definer set search_path = public as $$
declare
  pro boolean := exists (select 1 from public.entitlements e where e.user_id = new.user_id and e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()));
  lim jsonb := (select value from public.app_settings where key = case when pro then 'limits_pro' else 'limits_free' end);
  cap_key text := case new.kind when 'agent' then 'dailyAgentMissions' when 'web_tool' then 'dailyWebToolCalls' when 'research' then 'dailyResearch' else null end;
  cap integer;
  used integer;
begin
  if cap_key is null or lim is null or public.is_admin() then return new; end if;
  cap := nullif(lim ->> cap_key, '')::integer;
  if cap is null then return new; end if;
  select count(*) into used from public.usage_events where user_id = new.user_id and kind = new.kind and created_at > now() - interval '24 hours';
  if used >= cap then
    raise exception 'Daily limit reached for % (% per 24 hours on your plan).', new.kind, cap using errcode = '53400';
  end if;
  return new;
end;
$$;
create trigger usage_events_quota before insert on public.usage_events for each row execute function public.enforce_usage_quota();

-- ---------------------------------------------------------------------------------------------- admin functions
create function public.admin_set_setting(setting text, new_value jsonb, why text default null) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if setting not in ('maintenance', 'signups', 'limits_free', 'limits_pro') then raise exception 'unknown setting' using errcode = '22023'; end if;
  if new_value::text ~* '(sk-|nvapi-|api[_-]?key|secret|password)' then raise exception 'settings must not contain keys or secrets' using errcode = '22023'; end if;
  if setting like 'limits_%' then
    if jsonb_typeof(new_value) <> 'object' then raise exception 'limits must be an object' using errcode = '22023'; end if;
    if exists (select 1 from jsonb_each(new_value) e where e.value <> 'null'::jsonb and (jsonb_typeof(e.value) <> 'number' or (e.value)::text::numeric < 0)) then
      raise exception 'every limit must be a non-negative number or null' using errcode = '22023';
    end if;
  end if;
  insert into public.app_settings (key, value, updated_at, updated_by) values (setting, new_value, now(), auth.uid())
    on conflict (key) do update set value = excluded.value, updated_at = now(), updated_by = auth.uid();
  perform public.admin_log('set_setting', null, jsonb_build_object('setting', setting, 'value', new_value, 'why', why));
end;
$$;

create function public.admin_save_announcement(ann_id uuid, ann_title text, ann_body text, ann_level text, ann_active boolean, ann_ends timestamptz default null) returns uuid
language plpgsql security definer set search_path = public as $$
declare rid uuid;
begin
  perform public.admin_require();
  if ann_id is null then
    insert into public.announcements (title, body, level, active, ends_at, created_by) values (ann_title, ann_body, ann_level, ann_active, ann_ends, auth.uid()) returning id into rid;
  else
    update public.announcements set title = ann_title, body = ann_body, level = ann_level, active = ann_active, ends_at = ann_ends where id = ann_id returning id into rid;
    if rid is null then raise exception 'unknown announcement' using errcode = '22023'; end if;
  end if;
  perform public.admin_log('save_announcement', null, jsonb_build_object('id', rid, 'title', ann_title, 'level', ann_level, 'active', ann_active));
  return rid;
end;
$$;

create function public.admin_delete_announcement(ann_id uuid) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  delete from public.announcements where id = ann_id;
  perform public.admin_log('delete_announcement', null, jsonb_build_object('id', ann_id));
end;
$$;

create function public.admin_list_announcements() returns setof public.announcements
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select * from public.announcements order by created_at desc limit 100;
end;
$$;

create function public.admin_set_note(target uuid, new_note text) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  insert into public.admin_notes (user_id, note, updated_at, updated_by) values (target, coalesce(new_note, ''), now(), auth.uid())
    on conflict (user_id) do update set note = excluded.note, updated_at = now(), updated_by = auth.uid();
  perform public.admin_log('set_note', target, jsonb_build_object('length', char_length(coalesce(new_note, ''))));
end;
$$;

create function public.admin_delete_cohort(cohort text) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  update public.feature_flags set cohort_id = null where cohort_id = cohort;
  delete from public.early_access_cohorts where id = cohort;
  perform public.admin_log('delete_cohort', null, jsonb_build_object('cohort', cohort));
end;
$$;

create function public.admin_revoke_all_grants(target uuid) returns integer
language plpgsql security definer set search_path = public as $$
declare n integer;
begin
  perform public.admin_require();
  delete from public.feature_grants where user_id = target;
  get diagnostics n = row_count;
  perform public.admin_log('revoke_all_grants', target, jsonb_build_object('count', n));
  return n;
end;
$$;

-- Bulk plan change. Bounded (200 ids) so a typo cannot rewrite the whole user base.
create function public.admin_bulk_set_plan(targets uuid[], new_plan text, until timestamptz default null, note text default null) returns integer
language plpgsql security definer set search_path = public as $$
declare t uuid; n integer := 0;
begin
  perform public.admin_require();
  if new_plan not in ('free', 'pro') then raise exception 'unknown plan' using errcode = '22023'; end if;
  if coalesce(array_length(targets, 1), 0) > 200 then raise exception 'at most 200 users at a time' using errcode = '22023'; end if;
  foreach t in array coalesce(targets, '{}') loop
    insert into public.entitlements (user_id, plan, expires_at, note, updated_at, updated_by) values (t, new_plan, until, note, now(), auth.uid())
      on conflict (user_id) do update set plan = excluded.plan, expires_at = excluded.expires_at, note = excluded.note, updated_at = now(), updated_by = auth.uid();
    n := n + 1;
  end loop;
  perform public.admin_log('bulk_set_plan', null, jsonb_build_object('count', n, 'plan', new_plan, 'until', until));
  return n;
end;
$$;

create function public.admin_bulk_cohort(cohort text, targets uuid[], add_members boolean) returns integer
language plpgsql security definer set search_path = public as $$
declare t uuid; n integer := 0;
begin
  perform public.admin_require();
  if coalesce(array_length(targets, 1), 0) > 200 then raise exception 'at most 200 users at a time' using errcode = '22023'; end if;
  foreach t in array coalesce(targets, '{}') loop
    if add_members then
      insert into public.cohort_members (cohort_id, user_id) values (cohort, t) on conflict do nothing;
    else
      delete from public.cohort_members where cohort_id = cohort and user_id = t;
    end if;
    n := n + 1;
  end loop;
  perform public.admin_log(case when add_members then 'bulk_cohort_add' else 'bulk_cohort_remove' end, null, jsonb_build_object('cohort', cohort, 'count', n));
  return n;
end;
$$;

-- Search with filters, sorting and paging. total_count is the size of the whole result, not of the page.
create function public.admin_search_users(search text default '', plan_filter text default 'all', status_filter text default 'all', sort_by text default 'created', lim integer default 25, off integer default 0)
returns table (id uuid, email text, display_name text, status text, plan text, plan_expires_at timestamptz, created_at timestamptz, last_seen_at timestamptz,
               conversations bigint, messages bigint, events_30d bigint, is_admin boolean, total_count bigint)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query
    with base as (
      select p.id, p.email, p.display_name, p.status,
             case when e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()) then 'pro' else 'free' end as plan,
             e.expires_at as plan_expires_at, p.created_at, p.last_seen_at,
             (select count(*) from public.conversations c where c.user_id = p.id) as conversations,
             (select count(*) from public.messages m where m.user_id = p.id) as messages,
             (select count(*) from public.usage_events u where u.user_id = p.id and u.created_at > now() - interval '30 days') as events_30d,
             exists (select 1 from public.admin_users a where a.user_id = p.id) as is_admin
      from public.profiles p left join public.entitlements e on e.user_id = p.id
      where (search = '' or p.email ilike '%' || search || '%' or coalesce(p.display_name, '') ilike '%' || search || '%' or p.id::text = search)
    ), filtered as (
      select * from base where (plan_filter = 'all' or base.plan = plan_filter) and (status_filter = 'all' or base.status = status_filter)
    )
    select f.id, f.email, f.display_name, f.status, f.plan, f.plan_expires_at, f.created_at, f.last_seen_at, f.conversations, f.messages, f.events_30d, f.is_admin,
           (select count(*) from filtered)
    from filtered f
    order by
      case when sort_by = 'activity' then f.events_30d end desc nulls last,
      case when sort_by = 'seen' then f.last_seen_at end desc nulls last,
      case when sort_by = 'email' then f.email end asc,
      f.created_at desc
    limit least(greatest(lim, 1), 200) offset greatest(off, 0);
end;
$$;

create function public.admin_user_detail(target uuid) returns jsonb
language plpgsql stable security definer set search_path = public as $$
declare prof public.profiles; ent public.entitlements;
begin
  perform public.admin_require();
  select * into prof from public.profiles where id = target;
  if not found then raise exception 'unknown user' using errcode = '22023'; end if;
  select * into ent from public.entitlements where user_id = target;
  return jsonb_build_object(
    'profile', jsonb_build_object('id', prof.id, 'email', prof.email, 'displayName', prof.display_name, 'status', prof.status, 'suspendedReason', prof.suspended_reason, 'createdAt', prof.created_at, 'lastSeenAt', prof.last_seen_at),
    'isAdmin', exists (select 1 from public.admin_users a where a.user_id = target),
    'entitlement', case when ent.user_id is null then null else jsonb_build_object('plan', ent.plan, 'expiresAt', ent.expires_at, 'note', ent.note, 'updatedAt', ent.updated_at) end,
    'grants', coalesce((select jsonb_agg(jsonb_build_object('feature', g.feature, 'expiresAt', g.expires_at, 'reason', g.reason, 'createdAt', g.created_at) order by g.created_at desc) from public.feature_grants g where g.user_id = target), '[]'::jsonb),
    'cohorts', coalesce((select jsonb_agg(m.cohort_id) from public.cohort_members m where m.user_id = target), '[]'::jsonb),
    'note', coalesce((select n.note from public.admin_notes n where n.user_id = target), ''),
    'usage30d', coalesce((select jsonb_object_agg(k.kind, k.n) from (select kind, count(*) n from public.usage_events where user_id = target and created_at > now() - interval '30 days' group by kind) k), '{}'::jsonb),
    'counts', jsonb_build_object('conversations', (select count(*) from public.conversations where user_id = target), 'messages', (select count(*) from public.messages where user_id = target)),
    'audit', coalesce((select jsonb_agg(jsonb_build_object('at', a.at, 'action', a.action, 'detail', a.detail) order by a.id desc) from (select * from public.admin_audit where target_user = target order by id desc limit 25) a), '[]'::jsonb)
  );
end;
$$;

-- Counts per day and kind for the last N days (no content, no per-message data).
create function public.admin_usage_series(days integer default 30) returns table (day date, kind text, n bigint)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select (u.created_at at time zone 'utc')::date, u.kind, count(*) from public.usage_events u
    where u.created_at > now() - make_interval(days => least(greatest(days, 1), 180)) group by 1, 2 order by 1, 2;
end;
$$;

create function public.admin_top_users(days integer default 30, lim integer default 10) returns table (user_id uuid, email text, events bigint)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select u.user_id, p.email, count(*) from public.usage_events u join public.profiles p on p.id = u.user_id
    where u.created_at > now() - make_interval(days => least(greatest(days, 1), 180)) group by u.user_id, p.email order by 3 desc limit least(greatest(lim, 1), 50);
end;
$$;

create function public.admin_signups_series(days integer default 30) returns table (day date, n bigint)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select (p.created_at at time zone 'utc')::date, count(*) from public.profiles p
    where p.created_at > now() - make_interval(days => least(greatest(days, 1), 180)) group by 1 order by 1;
end;
$$;

-- Who the administrators are (read-only: the role cannot be granted or removed from the app).
create function public.admin_list_admins() returns table (user_id uuid, email text, since timestamptz)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select a.user_id, p.email, a.created_at from public.admin_users a left join public.profiles p on p.id = a.user_id order by a.created_at;
end;
$$;

do $$
declare fn text;
begin
  foreach fn in array array[
    'admin_set_setting(text, jsonb, text)', 'admin_save_announcement(uuid, text, text, text, boolean, timestamptz)', 'admin_delete_announcement(uuid)',
    'admin_list_announcements()', 'admin_set_note(uuid, text)', 'admin_delete_cohort(text)', 'admin_revoke_all_grants(uuid)',
    'admin_bulk_set_plan(uuid[], text, timestamptz, text)', 'admin_bulk_cohort(text, uuid[], boolean)',
    'admin_search_users(text, text, text, text, integer, integer)', 'admin_user_detail(uuid)', 'admin_usage_series(integer)',
    'admin_top_users(integer, integer)', 'admin_signups_series(integer)', 'admin_list_admins()'
  ] loop
    execute format('revoke all on function public.%s from public, anon', fn);
    execute format('grant execute on function public.%s to authenticated', fn);
  end loop;
end $$;
revoke all on function public.enforce_signups() from public, anon, authenticated;
revoke all on function public.enforce_usage_quota() from public, anon, authenticated;
