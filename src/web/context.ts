// The one WebRuntime of this page. Set by the web shell after sign-in; the Tauri shims route through it.
import type { WebRuntime } from './runtime';

let current: WebRuntime | null = null;

export function setRuntime(r: WebRuntime | null) {
  current = r;
}

export function getRuntime(): WebRuntime {
  if (!current) throw { kind: 'not_ready', message: 'Genius Web is not ready yet. Sign in first.' };
  return current;
}
