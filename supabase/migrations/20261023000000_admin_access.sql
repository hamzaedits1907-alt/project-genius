-- Owner control over WHO may use Genius: open to everyone, early access only, or closed — plus invites,
-- a waitlist, per-user limit overrides, user content removal and system statistics.
--
-- Same rules as the earlier admin migrations: every table has row level security, clients never write these
-- tables directly, and every change goes through a SECURITY DEFINER function that checks is_admin() on the
-- server and writes an audit record. Locking is enforced in the database: while Genius is locked for a user,
-- is_active() is false for them, so every write policy (conversations, messages, sync, key vault, ...) refuses
-- them, my_access() reports "locked" for every feature, and the web proxy and relay refuse the request.
-- Administrators are never locked out.

-- ---------------------------------------------------------------------------------------------- settings
insert into public.app_settings (key, value) values
  ('access', '{"mode": "open", "cohort": "early", "message": "", "waitlist": true}')
on conflict (key) do nothing;

insert into public.early_access_cohorts (id, name, description) values ('early', 'Early access', 'People allowed in while Genius is in early access')
on conflict (id) do nothing;

-- ---------------------------------------------------------------------------------------------- tables
create table public.access_requests (
  user_id uuid primary key references auth.users (id) on delete cascade,
  email text not null default '',
  note text not null default '' check (char_length(note) <= 500),
  status text not null default 'pending' check (status in ('pending', 'approved', 'declined')),
  created_at timestamptz not null default now(),
  decided_at timestamptz,
  decided_by uuid
);
alter table public.access_requests enable row level security;
create policy access_requests_own on public.access_requests for select to authenticated using (user_id = auth.uid());
revoke insert, update, delete on public.access_requests from authenticated, anon;

create table public.access_invites (
  email text primary key check (email = lower(email) and char_length(email) <= 254),
  cohort text not null references public.early_access_cohorts (id) on delete cascade,
  created_at timestamptz not null default now(),
  created_by uuid,
  claimed_by uuid,
  claimed_at timestamptz
);
alter table public.access_invites enable row level security; -- no client policy: invites are visible to administrators through functions only
revoke all on public.access_invites from authenticated, anon;

create table public.user_limit_overrides (
  user_id uuid primary key references auth.users (id) on delete cascade,
  limits jsonb not null default '{}'::jsonb,
  note text not null default '' check (char_length(note) <= 500),
  updated_at timestamptz not null default now(),
  updated_by uuid
);
alter table public.user_limit_overrides enable row level security;
create policy user_limit_overrides_own on public.user_limit_overrides for select to authenticated using (user_id = auth.uid());
revoke insert, update, delete on public.user_limit_overrides from authenticated, anon;

-- ---------------------------------------------------------------------------------------------- access decision
create function public.access_allowed() returns boolean
language sql stable security definer set search_path = public as $$
  select case
    when auth.uid() is null then false
    when exists (select 1 from public.admin_users a where a.user_id = auth.uid()) then true
    else case coalesce((select value ->> 'mode' from public.app_settings where key = 'access'), 'open')
      when 'open' then true
      when 'early' then exists (
        select 1 from public.cohort_members m
        where m.user_id = auth.uid() and m.cohort_id = coalesce((select value ->> 'cohort' from public.app_settings where key = 'access'), 'early'))
      else false
    end
  end;
$$;

-- "Active" now also means "allowed in": this is what makes the lock a database-enforced rule.
create or replace function public.is_active() returns boolean
language sql stable security definer set search_path = public as $$
  select exists (select 1 from public.profiles p where p.id = auth.uid() and p.status = 'active') and public.access_allowed();
$$;

-- ---------------------------------------------------------------------------------------------- my_access / my_context
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
  acc jsonb := coalesce((select value from public.app_settings where key = 'access'), '{"mode": "open"}');
  in_maintenance boolean;
  is_locked boolean;
  req text;
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
  is_locked := not public.access_allowed();
  select status into req from public.access_requests where user_id = uid;
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
      when is_locked then 'locked'
      when coalesce(fl.kill, false) then 'switched_off'
      when not deployed then 'not_rolled_out'
      when not plan_ok then 'needs_pro'
      else 'ok' end;
    result := result || jsonb_build_object(f.key, jsonb_build_object('available', reason = 'ok', 'reason', reason, 'name', f.name, 'description', f.description, 'minPlan', f.min_plan));
  end loop;
  return jsonb_build_object('signedIn', true, 'status', coalesce(prof.status, 'active'), 'plan', plan_now, 'planExpiresAt', ent.expires_at, 'isAdmin', public.is_admin(),
    'maintenance', case when in_maintenance then maint else null end,
    'access', jsonb_build_object('mode', acc ->> 'mode', 'locked', is_locked, 'message', coalesce(acc ->> 'message', ''), 'waitlist', coalesce((acc ->> 'waitlist')::boolean, true), 'request', req),
    'features', result);
end;
$$;
grant execute on function public.my_access() to authenticated;

create or replace function public.my_context() returns jsonb
language plpgsql stable security definer set search_path = public as $$
declare
  pro boolean := exists (select 1 from public.entitlements e where e.user_id = auth.uid() and e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()));
begin
  if auth.uid() is null then return jsonb_build_object('signedIn', false); end if;
  return jsonb_build_object(
    'limits', coalesce((select value from public.app_settings where key = case when pro then 'limits_pro' else 'limits_free' end), '{}'::jsonb)
              || coalesce((select o.limits from public.user_limit_overrides o where o.user_id = auth.uid()), '{}'::jsonb),
    'announcements', coalesce((select jsonb_agg(jsonb_build_object('id', a.id, 'title', a.title, 'body', a.body, 'level', a.level) order by a.created_at desc)
       from public.announcements a where a.active and a.starts_at <= now() and (a.ends_at is null or a.ends_at > now())), '[]'::jsonb));
end;
$$;
grant execute on function public.my_context() to authenticated;

-- Daily caps honour a per-user override (an owner can give one person more, or fewer).
create or replace function public.enforce_usage_quota() returns trigger
language plpgsql security definer set search_path = public as $$
declare
  pro boolean := exists (select 1 from public.entitlements e where e.user_id = new.user_id and e.plan = 'pro' and (e.expires_at is null or e.expires_at > now()));
  lim jsonb := coalesce((select value from public.app_settings where key = case when pro then 'limits_pro' else 'limits_free' end), '{}'::jsonb)
               || coalesce((select o.limits from public.user_limit_overrides o where o.user_id = new.user_id), '{}'::jsonb);
  cap_key text := case new.kind when 'agent' then 'dailyAgentMissions' when 'web_tool' then 'dailyWebToolCalls' when 'research' then 'dailyResearch' else null end;
  cap integer;
  used integer;
begin
  if cap_key is null or public.is_admin() then return new; end if;
  cap := nullif(lim ->> cap_key, '')::integer;
  if cap is null then return new; end if;
  select count(*) into used from public.usage_events where user_id = new.user_id and kind = new.kind and created_at > now() - interval '24 hours';
  if used >= cap then
    raise exception 'Daily limit reached for % (% per 24 hours).', new.kind, cap using errcode = '53400';
  end if;
  return new;
end;
$$;

-- ---------------------------------------------------------------------------------------------- user-facing: ask to be let in
create function public.request_access(p_note text default '') returns text
language plpgsql security definer set search_path = public as $$
declare uid uuid := auth.uid();
begin
  if uid is null then raise exception 'sign in first' using errcode = '28000'; end if;
  if public.access_allowed() then return 'approved'; end if;
  insert into public.access_requests (user_id, email, note) values (uid, coalesce((select email from public.profiles where id = uid), ''), left(coalesce(p_note, ''), 500))
    on conflict (user_id) do update set note = excluded.note, status = case when public.access_requests.status = 'declined' then 'declined' else 'pending' end;
  return (select status from public.access_requests where user_id = uid);
end;
$$;

-- An invited address joins the early-access cohort the moment it registers.
create function public.claim_access_invite() returns trigger
language plpgsql security definer set search_path = public as $$
declare inv public.access_invites;
begin
  select * into inv from public.access_invites where email = lower(coalesce(new.email, '')) and claimed_by is null;
  if found then
    insert into public.cohort_members (cohort_id, user_id) values (inv.cohort, new.id) on conflict do nothing;
    update public.access_invites set claimed_by = new.id, claimed_at = now() where email = inv.email;
  end if;
  return new;
end;
$$;
create trigger auth_users_claim_invite after insert on auth.users for each row execute function public.claim_access_invite();

-- ---------------------------------------------------------------------------------------------- admin functions
create function public.admin_set_access(p_mode text, p_cohort text default 'early', p_message text default '', p_waitlist boolean default true) returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if p_mode not in ('open', 'early', 'closed') then raise exception 'mode must be open, early or closed' using errcode = '22023'; end if;
  if p_cohort !~ '^[a-z0-9_-]{2,40}$' then raise exception 'invalid cohort id' using errcode = '22023'; end if;
  if char_length(coalesce(p_message, '')) > 500 then raise exception 'the message is too long' using errcode = '22023'; end if;
  insert into public.early_access_cohorts (id, name) values (p_cohort, initcap(replace(p_cohort, '-', ' '))) on conflict (id) do nothing;
  insert into public.app_settings (key, value, updated_at, updated_by)
    values ('access', jsonb_build_object('mode', p_mode, 'cohort', p_cohort, 'message', coalesce(p_message, ''), 'waitlist', coalesce(p_waitlist, true)), now(), auth.uid())
    on conflict (key) do update set value = excluded.value, updated_at = now(), updated_by = auth.uid();
  perform public.admin_log('set_access', null, jsonb_build_object('mode', p_mode, 'cohort', p_cohort, 'waitlist', p_waitlist));
end;
$$;

create function public.admin_access_overview() returns jsonb
language plpgsql stable security definer set search_path = public as $$
declare
  acc jsonb := coalesce((select value from public.app_settings where key = 'access'), '{"mode": "open", "cohort": "early"}');
  coh text := coalesce(acc ->> 'cohort', 'early');
begin
  perform public.admin_require();
  return jsonb_build_object(
    'access', acc,
    'users', (select count(*) from public.profiles),
    'admins', (select count(*) from public.admin_users),
    'cohortMembers', (select count(*) from public.cohort_members where cohort_id = coh),
    'allowedNow', case acc ->> 'mode'
      when 'open' then (select count(*) from public.profiles)
      when 'early' then (select count(distinct u) from (select user_id u from public.cohort_members where cohort_id = coh union select user_id from public.admin_users) s)
      else (select count(*) from public.admin_users) end,
    'pendingRequests', (select count(*) from public.access_requests where status = 'pending'),
    'invitesPending', (select count(*) from public.access_invites where claimed_by is null),
    'invitesClaimed', (select count(*) from public.access_invites where claimed_by is not null),
    'cohorts', coalesce((select jsonb_agg(jsonb_build_object('id', c.id, 'name', c.name, 'members', (select count(*) from public.cohort_members m where m.cohort_id = c.id)) order by c.created_at) from public.early_access_cohorts c), '[]'::jsonb));
end;
$$;

create function public.admin_list_access_requests(p_status text default 'pending') returns table (user_id uuid, email text, note text, status text, created_at timestamptz, decided_at timestamptz)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select r.user_id, r.email, r.note, r.status, r.created_at, r.decided_at from public.access_requests r
    where p_status = 'all' or r.status = p_status order by r.created_at desc limit 500;
end;
$$;

create function public.admin_decide_access_request(p_user uuid, p_approve boolean) returns void
language plpgsql security definer set search_path = public as $$
declare coh text := coalesce((select value ->> 'cohort' from public.app_settings where key = 'access'), 'early');
begin
  perform public.admin_require();
  if not exists (select 1 from public.access_requests where user_id = p_user) then raise exception 'no such request' using errcode = '22023'; end if;
  insert into public.early_access_cohorts (id, name) values (coh, initcap(replace(coh, '-', ' '))) on conflict (id) do nothing;
  if p_approve then
    insert into public.cohort_members (cohort_id, user_id) values (coh, p_user) on conflict do nothing;
  end if;
  update public.access_requests set status = case when p_approve then 'approved' else 'declined' end, decided_at = now(), decided_by = auth.uid() where user_id = p_user;
  perform public.admin_log(case when p_approve then 'access_approve' else 'access_decline' end, p_user, jsonb_build_object('cohort', coh));
end;
$$;

create function public.admin_invite_emails(p_emails text[], p_cohort text default null) returns jsonb
language plpgsql security definer set search_path = public as $$
declare
  coh text := coalesce(p_cohort, (select value ->> 'cohort' from public.app_settings where key = 'access'), 'early');
  e text;
  uid uuid;
  invited integer := 0;
  added integer := 0;
  invalid text[] := '{}';
begin
  perform public.admin_require();
  if coalesce(array_length(p_emails, 1), 0) > 200 then raise exception 'at most 200 addresses at a time' using errcode = '22023'; end if;
  insert into public.early_access_cohorts (id, name) values (coh, initcap(replace(coh, '-', ' '))) on conflict (id) do nothing;
  foreach e in array coalesce(p_emails, '{}') loop
    e := lower(btrim(e));
    if e = '' then continue; end if;
    if e !~ '^[^@\s]+@[^@\s]+\.[^@\s]+$' or char_length(e) > 254 then invalid := invalid || e; continue; end if;
    select id into uid from public.profiles where lower(email) = e limit 1;
    if uid is not null then
      insert into public.cohort_members (cohort_id, user_id) values (coh, uid) on conflict do nothing;
      insert into public.access_invites (email, cohort, created_by, claimed_by, claimed_at) values (e, coh, auth.uid(), uid, now())
        on conflict (email) do update set cohort = excluded.cohort, claimed_by = excluded.claimed_by, claimed_at = excluded.claimed_at;
      added := added + 1;
    else
      insert into public.access_invites (email, cohort, created_by) values (e, coh, auth.uid())
        on conflict (email) do update set cohort = excluded.cohort where public.access_invites.claimed_by is null;
      invited := invited + 1;
    end if;
  end loop;
  perform public.admin_log('invite_emails', null, jsonb_build_object('cohort', coh, 'invited', invited, 'addedNow', added, 'invalid', coalesce(array_length(invalid, 1), 0)));
  return jsonb_build_object('invited', invited, 'addedNow', added, 'invalid', to_jsonb(invalid));
end;
$$;

create function public.admin_list_invites() returns table (email text, cohort text, created_at timestamptz, claimed_by uuid, claimed_at timestamptz)
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return query select i.email, i.cohort, i.created_at, i.claimed_by, i.claimed_at from public.access_invites i order by i.created_at desc limit 500;
end;
$$;

create function public.admin_revoke_invite(p_email text) returns void
language plpgsql security definer set search_path = public as $$
declare inv public.access_invites;
begin
  perform public.admin_require();
  select * into inv from public.access_invites where email = lower(btrim(p_email));
  if not found then return; end if;
  if inv.claimed_by is not null then
    delete from public.cohort_members where cohort_id = inv.cohort and user_id = inv.claimed_by;
  end if;
  delete from public.access_invites where email = inv.email;
  perform public.admin_log('revoke_invite', inv.claimed_by, jsonb_build_object('email', inv.email, 'cohort', inv.cohort));
end;
$$;

create function public.admin_set_user_limits(target uuid, p_limits jsonb, p_note text default '') returns void
language plpgsql security definer set search_path = public as $$
begin
  perform public.admin_require();
  if jsonb_typeof(p_limits) <> 'object' then raise exception 'limits must be an object' using errcode = '22023'; end if;
  if exists (select 1 from jsonb_object_keys(p_limits) k where k not in ('maxModelCalls', 'maxToolCalls', 'maxTaskMinutes', 'maxConcurrentTasks', 'maxRetries', 'contextChars', 'dailyAgentMissions', 'dailyWebToolCalls', 'dailyResearch')) then
    raise exception 'unknown limit' using errcode = '22023';
  end if;
  if exists (select 1 from jsonb_each(p_limits) e where e.value <> 'null'::jsonb and (jsonb_typeof(e.value) <> 'number' or (e.value)::text::numeric < 0)) then
    raise exception 'every limit must be a non-negative number or null' using errcode = '22023';
  end if;
  if p_limits = '{}'::jsonb then
    delete from public.user_limit_overrides where user_id = target;
  else
    insert into public.user_limit_overrides (user_id, limits, note, updated_at, updated_by) values (target, p_limits, left(coalesce(p_note, ''), 500), now(), auth.uid())
      on conflict (user_id) do update set limits = excluded.limits, note = excluded.note, updated_at = now(), updated_by = auth.uid();
  end if;
  perform public.admin_log('set_user_limits', target, jsonb_build_object('limits', p_limits));
end;
$$;

create function public.admin_user_extras(target uuid) returns jsonb
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return jsonb_build_object(
    'limits', (select o.limits from public.user_limit_overrides o where o.user_id = target),
    'limitsNote', (select o.note from public.user_limit_overrides o where o.user_id = target),
    'request', (select to_jsonb(r) - 'user_id' from public.access_requests r where r.user_id = target),
    'allowedNow', exists (select 1 from public.admin_users a where a.user_id = target)
       or coalesce((select value ->> 'mode' from public.app_settings where key = 'access'), 'open') = 'open'
       or exists (select 1 from public.cohort_members m where m.user_id = target and m.cohort_id = coalesce((select value ->> 'cohort' from public.app_settings where key = 'access'), 'early')),
    'hasKeyVault', exists (select 1 from public.key_vault v where v.user_id = target));
end;
$$;

-- Removes everything a person stored in Genius (their conversations, files, missions, preferences and key vault) but keeps the
-- account and the usage counters. Files in storage are not touched by SQL and remain until removed in the Supabase dashboard.
create function public.admin_purge_user_data(target uuid) returns jsonb
language plpgsql security definer set search_path = public as $$
declare c integer; a integer; m integer;
begin
  perform public.admin_require();
  if exists (select 1 from public.admin_users where user_id = target) then raise exception 'administrator data cannot be purged here' using errcode = '42501'; end if;
  delete from public.conversations where user_id = target;
  get diagnostics c = row_count;
  delete from public.attachments where user_id = target;
  get diagnostics a = row_count;
  delete from public.missions where user_id = target;
  get diagnostics m = row_count;
  delete from public.user_preferences where user_id = target;
  delete from public.key_vault where user_id = target;
  perform public.admin_log('purge_user_data', target, jsonb_build_object('conversations', c, 'attachments', a, 'missions', m));
  return jsonb_build_object('conversations', c, 'attachments', a, 'missions', m);
end;
$$;

create function public.admin_system_stats() returns jsonb
language plpgsql stable security definer set search_path = public as $$
begin
  perform public.admin_require();
  return jsonb_build_object(
    'users', (select count(*) from public.profiles),
    'newLast24h', (select count(*) from public.profiles where created_at > now() - interval '24 hours'),
    'newLast7d', (select count(*) from public.profiles where created_at > now() - interval '7 days'),
    'activeLast24h', (select count(distinct u) from (select user_id u from public.usage_events where created_at > now() - interval '24 hours' union select user_id from public.messages where created_at > now() - interval '24 hours') s),
    'activeLast7d', (select count(distinct u) from (select user_id u from public.usage_events where created_at > now() - interval '7 days' union select user_id from public.messages where created_at > now() - interval '7 days') s),
    'conversations', (select count(*) from public.conversations where deleted_at is null),
    'messages', (select count(*) from public.messages where deleted_at is null),
    'messagesLast24h', (select count(*) from public.messages where created_at > now() - interval '24 hours' and deleted_at is null),
    'attachments', (select count(*) from public.attachments),
    'attachmentBytes', (select coalesce(sum(size), 0) from public.attachments),
    'keyVaults', (select count(*) from public.key_vault),
    'withOverrides', (select count(*) from public.user_limit_overrides),
    'pendingRequests', (select count(*) from public.access_requests where status = 'pending'),
    'eventsLast24h', coalesce((select jsonb_object_agg(kind, n) from (select kind, count(*) n from public.usage_events where created_at > now() - interval '24 hours' group by kind) k), '{}'::jsonb));
end;
$$;

do $$
declare fn text;
begin
  foreach fn in array array[
    'request_access(text)', 'admin_set_access(text, text, text, boolean)', 'admin_access_overview()', 'admin_list_access_requests(text)',
    'admin_decide_access_request(uuid, boolean)', 'admin_invite_emails(text[], text)', 'admin_list_invites()', 'admin_revoke_invite(text)',
    'admin_set_user_limits(uuid, jsonb, text)', 'admin_user_extras(uuid)', 'admin_purge_user_data(uuid)', 'admin_system_stats()'
  ] loop
    execute format('revoke all on function public.%s from public, anon', fn);
    execute format('grant execute on function public.%s to authenticated', fn);
  end loop;
end $$;
revoke all on function public.access_allowed() from public, anon;
grant execute on function public.access_allowed() to authenticated;
revoke all on function public.claim_access_invite() from public, anon, authenticated;
