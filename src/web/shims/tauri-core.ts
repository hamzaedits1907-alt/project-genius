// Browser replacement for '@tauri-apps/api/core': `invoke` runs the command in the web runtime.
import { getRuntime } from '../context';

export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return getRuntime().invoke(cmd, args ?? {}) as Promise<T>;
  } catch (e) {
    return Promise.reject(e);
  }
}

export const isTauri = () => false;
export const convertFileSrc = (p: string) => p;
