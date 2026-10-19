-- Agent Mode missions of Genius Web (the plan, steps, results and verification state of each mission).
create table public.missions (
  id uuid primary key,
  user_id uuid not null default auth.uid() references auth.users (id) on delete cascade,
  title text not null check (char_length(title) <= 300),
  goal text not null check (char_length(goal) <= 20000),
  status text not null,
  data jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);
create index missions_user_updated on public.missions (user_id, updated_at desc);
alter table public.missions enable row level security;
create policy missions_select_own on public.missions for select to authenticated using (user_id = auth.uid());
create policy missions_insert_own on public.missions for insert to authenticated with check (user_id = auth.uid() and public.is_active());
create policy missions_update_own on public.missions for update to authenticated using (user_id = auth.uid() and public.is_active()) with check (user_id = auth.uid());
create policy missions_delete_own on public.missions for delete to authenticated using (user_id = auth.uid() and public.is_active());
