// Whether a Genius feature is usable for the signed-in user. The desktop app has no accounts, so everything
// is available there; Genius Web installs a resolver backed by the database (`my_access()`).
import { useSyncExternalStore } from 'react';

export interface Gate {
  available: boolean;
  /** ok | needs_pro | switched_off | not_rolled_out | suspended | loading */
  reason: string;
  name: string;
}

let resolver: ((key: string) => Gate) | null = null;
let version = 0;
const listeners = new Set<() => void>();

export function setFeatureResolver(fn: ((key: string) => Gate) | null) {
  resolver = fn;
  version++;
  listeners.forEach((l) => l());
}

export function gateOf(key: string): Gate {
  return resolver ? resolver(key) : { available: true, reason: 'ok', name: key };
}

/** Subscribes the component to access changes. */
export function useFeature(key: string): Gate {
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => version,
  );
  return gateOf(key);
}

export const GATE_TEXT: Record<string, string> = {
  needs_pro: 'This is part of Genius Pro. The owner of this Genius Web deployment can give you access.',
  switched_off: 'This feature is switched off at the moment.',
  not_rolled_out: 'This feature is being rolled out in stages and is not open to your account yet.',
  suspended: 'Your account is suspended.',
  loading: 'Checking your access…',
};
