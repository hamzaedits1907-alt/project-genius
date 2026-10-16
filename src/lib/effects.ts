// Effect helpers that make cleanup correctness structural instead of a convention.
//
// React 19 stores whatever an effect callback returns and, on unmount, calls it if it is not
// `undefined` — without checking that it is a function. Returning anything else (a Promise from
// an async callback, `null`, the result of `listen()`, a value from a concise arrow body) crashes
// the view with "destroy is not a function". These helpers keep async subscriptions out of the
// effect's return value and verify that every cleanup they install really is a function.
import { useEffect, useRef } from 'react';

export type Cleanup = () => void;

/** Thrown (and reported by the error boundary) when a subscription yields something that is not a cleanup function. */
export class InvalidCleanupError extends Error {
  constructor(name: string, value: unknown) {
    super(`Subscription "${name}" returned ${describe(value)} instead of a cleanup function`);
    this.name = 'InvalidCleanupError';
  }
}

function describe(v: unknown): string {
  if (v === null) return 'null';
  if (v instanceof Promise) return 'a Promise';
  return typeof v === 'object' ? 'an object' : `a ${typeof v}`;
}

/** Returns `value` if it is a function (or undefined → no-op); otherwise throws a descriptive error. */
export function asCleanup(name: string, value: unknown): Cleanup {
  if (value === undefined) return () => undefined;
  if (typeof value === 'function') return value as Cleanup;
  throw new InvalidCleanupError(name, value);
}

/**
 * Subscribe while mounted. `subscribe` may return a cleanup or a Promise of one (e.g. Tauri's
 * `listen`). If the component unmounts before the Promise settles — which StrictMode does on
 * every mount in development — the subscription is released as soon as it resolves, so
 * listeners never leak or double-fire. Errors are reported through `onError` (default:
 * console.error) rather than swallowed.
 */
export function useSubscription(
  name: string,
  subscribe: () => Cleanup | Promise<Cleanup> | void,
  deps: readonly unknown[],
  onError: (e: unknown) => void = (e) => console.error(`[${name}]`, e),
): void {
  const errRef = useRef(onError);
  errRef.current = onError;
  useEffect(() => {
    let disposed = false;
    let cleanup: Cleanup | null = null;
    const release = (c: Cleanup) => {
      try {
        c();
      } catch (e) {
        errRef.current(e);
      }
    };
    try {
      const result = subscribe();
      if (result instanceof Promise) {
        result.then(
          (value) => {
            try {
              const c = asCleanup(name, value);
              if (disposed) release(c);
              else cleanup = c;
            } catch (e) {
              errRef.current(e);
            }
          },
          (e) => errRef.current(e),
        );
      } else {
        cleanup = asCleanup(name, result);
      }
    } catch (e) {
      errRef.current(e);
    }
    return () => {
      disposed = true;
      if (cleanup) release(cleanup);
      cleanup = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
