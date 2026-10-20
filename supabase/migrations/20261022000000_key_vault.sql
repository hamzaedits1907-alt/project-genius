-- Key vault: the user's AI provider keys, stored in their Genius account so every device (desktop, web,
-- Android, iOS) has them after signing in — WITHOUT the database ever being able to read them.
--
-- How: each client encrypts the keys itself (AES-GCM, 256-bit) with a key derived from a vault passphrase
-- only the user knows (PBKDF2-SHA256, 600,000 iterations, random salt). This table stores only the salt,
-- the iteration count, a small "verifier" (to tell a wrong passphrase from a right one) and one ciphertext
-- per provider. Supabase, the project owner and anyone with database access see ciphertext only. There is
-- no recovery: a forgotten passphrase means resetting the vault and entering the keys again.
--
-- Defence in depth: a trigger refuses anything that looks like a raw provider key or is too large, so even a
-- buggy or malicious client cannot store a plaintext key here. Row level security keeps each vault private.

create table public.key_vault (
  user_id uuid primary key default auth.uid() references auth.users (id) on delete cascade,
  version integer not null default 1,
  kdf_salt text not null check (char_length(kdf_salt) between 16 and 64),
  kdf_iterations integer not null check (kdf_iterations between 100000 and 5000000),
  verifier_iv text not null check (char_length(verifier_iv) between 8 and 32),
  verifier_ct text not null check (char_length(verifier_ct) between 16 and 256),
  entries jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

alter table public.key_vault enable row level security;
create policy key_vault_select_own on public.key_vault for select to authenticated using (user_id = auth.uid());
create policy key_vault_insert_own on public.key_vault for insert to authenticated with check (user_id = auth.uid() and public.is_active());
create policy key_vault_update_own on public.key_vault for update to authenticated using (user_id = auth.uid() and public.is_active()) with check (user_id = auth.uid());
create policy key_vault_delete_own on public.key_vault for delete to authenticated using (user_id = auth.uid());

create function public.key_vault_guard() returns trigger
language plpgsql as $$
declare
  provider text;
  entry jsonb;
begin
  if jsonb_typeof(new.entries) <> 'object' then
    raise exception 'entries must be an object' using errcode = '22023';
  end if;
  if octet_length(new.entries::text) > 32768 then
    raise exception 'vault is too large' using errcode = '22023';
  end if;
  for provider, entry in select * from jsonb_each(new.entries) loop
    if provider not in ('nvidia', 'openai', 'anthropic', 'gemini', 'custom') then
      raise exception 'unknown provider %', provider using errcode = '22023';
    end if;
    if jsonb_typeof(entry) <> 'object' or not (entry ? 'iv' and entry ? 'ct') or jsonb_typeof(entry -> 'iv') <> 'string' or jsonb_typeof(entry -> 'ct') <> 'string' then
      raise exception 'each entry needs an iv and a ciphertext' using errcode = '22023';
    end if;
    if (entry ->> 'ct') !~ '^[A-Za-z0-9+/=]{16,}$' or char_length(entry ->> 'ct') > 2048 or (entry ->> 'iv') !~ '^[A-Za-z0-9+/=]{8,32}$' then
      raise exception 'entries must be base64 ciphertext' using errcode = '22023';
    end if;
  end loop;
  -- raw provider keys contain "-" or "_" prefixes that standard base64 never has
  if lower(new.entries::text) ~ '(sk-[a-z0-9_-]{10,}|nvapi-[a-z0-9_-]{6,}|sk-ant-|bearer\s)' then
    raise exception 'a provider key must be encrypted before it is stored' using errcode = '22023';
  end if;
  new.updated_at := now();
  return new;
end;
$$;
create trigger key_vault_guard before insert or update on public.key_vault for each row execute function public.key_vault_guard();
