import { useEffect, useState } from 'react';
import { api } from '../../../lib/api';
import type { SlashCommand } from '../../../lib/types';
import { BUILTIN_COMMANDS, mergeCommands } from './commands';

let cache: Promise<SlashCommand[]> | null = null;

/** The backend's command list, fetched once per app session (built-ins if it cannot answer). */
export function loadCommands(): Promise<SlashCommand[]> {
  cache ??= api.listCommands().then(
    (list) => mergeCommands(Array.isArray(list) ? list : []),
    () => BUILTIN_COMMANDS,
  );
  return cache;
}

/** Test hook: forget the cached list. */
export function resetCommandCache() {
  cache = null;
}

export function useCommands(): SlashCommand[] {
  const [commands, setCommands] = useState<SlashCommand[]>(BUILTIN_COMMANDS);
  useEffect(() => {
    let live = true;
    void loadCommands().then((c) => {
      if (live) setCommands(c);
    });
    return () => {
      live = false;
    };
  }, []);
  return commands;
}
