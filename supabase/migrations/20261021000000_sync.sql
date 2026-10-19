-- Conversation sync for every Genius client (desktop, web, Android, iOS).
--
-- The account's conversations live here, owned by the signed-in user (row level security from the first
-- migration still applies to every statement below: all functions are SECURITY INVOKER). What this adds:
--   * Tombstones. A deleted conversation or message stays as a row with deleted_at set, so other devices
--     learn about the deletion instead of re-uploading the old copy.
--   * Deterministic order. Messages sort by (created_at, seq); seq is assigned by the database.
--   * A change cursor. synced_at is stamped by the server on every insert/update; clients pull "everything
--     after (synced_at, id)" and apply it idempotently.
--   * sync_push: an idempotent, validated batch upsert (retries and duplicate submissions are harmless;
--     tombstones win; otherwise the newest update wins).
--   * conversation_summaries: the sidebar list with message count and snippet in one query.
-- Provider API keys never reach these tables (messages are conversation text only).

-- ---------------------------------------------------------------------------------------------- columns
alter table public.conversations
  add column deleted_at timestamptz,
  add column meta_updated_at timestamptz not null default now(),
  add column synced_at timestamptz not null default clock_timestamp();

alter table public.messages
  add column deleted_at timestamptz,
  add column seq bigint generated always as identity,
  add column updated_at timestamptz not null default now(),
  add column synced_at timestamptz not null default clock_timestamp();

create index conversations_user_synced on public.conversations (user_id, synced_at, id);
create index messages_user_synced on public.messages (user_id, synced_at, id);
create index messages_conversation_order on public.messages (conversation_id, created_at, seq);

-- Every write is stamped by the server clock; a plain UPDATE of a message also moves its updated_at.
create function public.stamp_sync() returns trigger
language plpgsql as $$
begin
  new.synced_at := clock_timestamp();
  if tg_op = 'UPDATE' then
    if tg_table_name = 'messages' then
      if new.updated_at = old.updated_at then new.updated_at := now(); end if;
    elsif new.title is distinct from old.title and new.meta_updated_at = old.meta_updated_at then
      new.meta_updated_at := now();
    end if;
  end if;
  return new;
end;
$$;
create trigger conversations_stamp before insert or update on public.conversations for each row execute function public.stamp_sync();
create trigger messages_stamp before insert or update on public.messages for each row execute function public.stamp_sync();

-- ---------------------------------------------------------------------------------------------- pull
-- Rows changed after the cursor (synced_at, id), oldest first. Desktop clients ask for chat and research
-- conversations only; Code sessions and Agent missions of Genius Web stay in the browser.
create function public.sync_pull_conversations(p_after_ts timestamptz default null, p_after_id uuid default null, p_limit integer default 200)
returns jsonb language sql stable set search_path = public as $$
  select coalesce(jsonb_agg(x.j order by x.ts, x.id), '[]'::jsonb) from (
    select c.synced_at as ts, c.id,
      jsonb_build_object('id', c.id, 'mode', c.mode, 'title', c.title, 'origin', c.origin, 'createdAt', c.created_at, 'updatedAt', c.updated_at,
        'metaUpdatedAt', c.meta_updated_at, 'deletedAt', c.deleted_at, 'syncedAt', c.synced_at) as j
    from public.conversations c
    where c.user_id = auth.uid()
      and c.mode in ('chat', 'research')
      and coalesce(c.origin ->> 'flavor', '') <> 'code'
      and (p_after_ts is null or (c.synced_at, c.id) > (p_after_ts, coalesce(p_after_id, '00000000-0000-0000-0000-000000000000'::uuid)))
    order by c.synced_at, c.id
    limit least(greatest(coalesce(p_limit, 200), 1), 500)
  ) x;
$$;

create function public.sync_pull_messages(p_after_ts timestamptz default null, p_after_id uuid default null, p_limit integer default 300)
returns jsonb language sql stable set search_path = public as $$
  select coalesce(jsonb_agg(x.j order by x.ts, x.id), '[]'::jsonb) from (
    select m.synced_at as ts, m.id,
      jsonb_build_object('id', m.id, 'conversationId', m.conversation_id, 'role', m.role, 'kind', m.kind, 'content', m.content, 'meta', m.meta,
        'taskId', m.task_id, 'createdAt', m.created_at, 'updatedAt', m.updated_at, 'deletedAt', m.deleted_at, 'seq', m.seq, 'syncedAt', m.synced_at) as j
    from public.messages m
    join public.conversations c on c.id = m.conversation_id
    where m.user_id = auth.uid()
      and c.mode in ('chat', 'research')
      and coalesce(c.origin ->> 'flavor', '') <> 'code'
      and (p_after_ts is null or (m.synced_at, m.id) > (p_after_ts, coalesce(p_after_id, '00000000-0000-0000-0000-000000000000'::uuid)))
    order by m.synced_at, m.id
    limit least(greatest(coalesce(p_limit, 300), 1), 500)
  ) x;
$$;

-- ---------------------------------------------------------------------------------------------- push
create function public.sync_push(p_conversations jsonb default '[]', p_messages jsonb default '[]')
returns jsonb language plpgsql set search_path = public as $$
declare
  c jsonb;
  m jsonb;
  n_conv integer := 0;
  n_msg integer := 0;
  v_updated timestamptz;
  v_deleted timestamptz;
  v_ts timestamptz;
begin
  if auth.uid() is null then raise exception 'sign in first' using errcode = '28000'; end if;
  if jsonb_typeof(p_conversations) <> 'array' or jsonb_typeof(p_messages) <> 'array' then raise exception 'conversations and messages must be arrays' using errcode = '22023'; end if;
  if jsonb_array_length(p_conversations) > 300 or jsonb_array_length(p_messages) > 500 then raise exception 'batch too large' using errcode = '22023'; end if;

  for c in select * from jsonb_array_elements(p_conversations) loop
    if (c ->> 'mode') not in ('chat', 'research') then raise exception 'unsupported conversation mode' using errcode = '22023'; end if;
    v_updated := least(coalesce((c ->> 'metaUpdatedAt')::timestamptz, now()), now() + interval '1 minute');
    v_deleted := (c ->> 'deletedAt')::timestamptz;
    insert into public.conversations (id, mode, title, origin, created_at, updated_at, meta_updated_at, deleted_at)
    values ((c ->> 'id')::uuid, c ->> 'mode', left(coalesce(nullif(c ->> 'title', ''), 'New conversation'), 200), nullif(c -> 'origin', 'null'::jsonb),
            coalesce((c ->> 'createdAt')::timestamptz, now()), coalesce((c ->> 'updatedAt')::timestamptz, now()), v_updated, v_deleted)
    on conflict (id) do update set
      title = case when excluded.meta_updated_at > conversations.meta_updated_at then excluded.title else conversations.title end,
      origin = case when excluded.meta_updated_at > conversations.meta_updated_at then excluded.origin else conversations.origin end,
      meta_updated_at = greatest(conversations.meta_updated_at, excluded.meta_updated_at),
      updated_at = greatest(conversations.updated_at, excluded.updated_at),
      deleted_at = coalesce(conversations.deleted_at, excluded.deleted_at)
    where conversations.user_id = auth.uid();
    n_conv := n_conv + 1;
  end loop;

  for m in select * from jsonb_array_elements(p_messages) loop
    if (m ->> 'role') not in ('user', 'assistant', 'system') then raise exception 'unsupported message role' using errcode = '22023'; end if;
    v_ts := coalesce((m ->> 'createdAt')::timestamptz, now());
    v_updated := least(coalesce((m ->> 'updatedAt')::timestamptz, v_ts), now() + interval '1 minute');
    v_deleted := (m ->> 'deletedAt')::timestamptz;
    insert into public.messages (id, conversation_id, role, kind, content, meta, task_id, created_at, updated_at, deleted_at)
    values ((m ->> 'id')::uuid, (m ->> 'conversationId')::uuid, m ->> 'role', coalesce(nullif(m ->> 'kind', ''), 'chat'), coalesce(m ->> 'content', ''),
            coalesce(m -> 'meta', '{}'::jsonb), nullif(m ->> 'taskId', ''), v_ts, v_updated, v_deleted)
    on conflict (id) do update set
      content = case when excluded.updated_at > messages.updated_at then excluded.content else messages.content end,
      meta = case when excluded.updated_at > messages.updated_at then excluded.meta else messages.meta end,
      kind = case when excluded.updated_at > messages.updated_at then excluded.kind else messages.kind end,
      deleted_at = coalesce(messages.deleted_at, excluded.deleted_at)
    where messages.user_id = auth.uid();
    n_msg := n_msg + 1;
  end loop;

  return jsonb_build_object('conversations', n_conv, 'messages', n_msg);
end;
$$;

-- Delete a message and everything after it (rewind / "regenerate from here"), as tombstones.
create function public.delete_messages_from(p_conversation uuid, p_message uuid) returns integer
language plpgsql set search_path = public as $$
declare n integer;
begin
  with pivot as (select created_at, seq from public.messages where id = p_message and conversation_id = p_conversation and user_id = auth.uid())
  update public.messages m set deleted_at = now()
  from pivot
  where m.conversation_id = p_conversation and m.user_id = auth.uid() and m.deleted_at is null and (m.created_at, m.seq) >= (pivot.created_at, pivot.seq);
  get diagnostics n = row_count;
  return n;
end;
$$;

-- Soft-delete one conversation (the tombstone is what the other devices act on).
create function public.delete_conversation(p_id uuid) returns void
language plpgsql set search_path = public as $$
begin
  update public.conversations set deleted_at = coalesce(deleted_at, now()) where id = p_id and user_id = auth.uid();
end;
$$;

-- ---------------------------------------------------------------------------------------------- sidebar list
create function public.conversation_summaries(p_mode text default null, p_query text default null, p_limit integer default 100, p_before timestamptz default null)
returns table (id uuid, mode text, title text, origin jsonb, created_at timestamptz, updated_at timestamptz, message_count bigint, snippet text)
language sql stable set search_path = public as $$
  select c.id, c.mode, c.title, c.origin, c.created_at, c.updated_at,
         (select count(*) from public.messages m where m.conversation_id = c.id and m.deleted_at is null) as message_count,
         coalesce((select left(regexp_replace(m.content, '\s+', ' ', 'g'), 140) from public.messages m where m.conversation_id = c.id and m.deleted_at is null
                   order by m.created_at desc, m.seq desc limit 1), '') as snippet
  from public.conversations c
  where c.user_id = auth.uid() and c.deleted_at is null
    and (p_mode is null or c.mode = p_mode)
    and (p_before is null or c.updated_at < p_before)
    and (nullif(btrim(p_query), '') is null
         or c.title ilike '%' || replace(replace(replace(btrim(p_query), '\', '\\'), '%', '\%'), '_', '\_') || '%'
         or exists (select 1 from public.messages m where m.conversation_id = c.id and m.deleted_at is null
                    and m.content ilike '%' || replace(replace(replace(btrim(p_query), '\', '\\'), '%', '\%'), '_', '\_') || '%'))
  order by c.updated_at desc, c.id
  limit least(greatest(coalesce(p_limit, 100), 1), 300);
$$;

-- ---------------------------------------------------------------------------------------------- access
do $$
declare fn text;
begin
  foreach fn in array array[
    'sync_pull_conversations(timestamptz, uuid, integer)', 'sync_pull_messages(timestamptz, uuid, integer)', 'sync_push(jsonb, jsonb)',
    'delete_messages_from(uuid, uuid)', 'delete_conversation(uuid)', 'conversation_summaries(text, text, integer, timestamptz)'
  ] loop
    execute format('revoke all on function public.%s from public, anon', fn);
    execute format('grant execute on function public.%s to authenticated', fn);
  end loop;
end $$;

-- Live updates for connected clients (Supabase Realtime). Skipped on databases without the publication.
do $$
begin
  if exists (select 1 from pg_publication where pubname = 'supabase_realtime') then
    if not exists (select 1 from pg_publication_tables where pubname = 'supabase_realtime' and schemaname = 'public' and tablename = 'conversations') then
      alter publication supabase_realtime add table public.conversations;
    end if;
    if not exists (select 1 from pg_publication_tables where pubname = 'supabase_realtime' and schemaname = 'public' and tablename = 'messages') then
      alter publication supabase_realtime add table public.messages;
    end if;
  end if;
end $$;
