// Browser replacement for '@tauri-apps/api/event': events come from the web runtime's own bus.
import { getRuntime } from '../context';

export interface Event<T> {
  event: string;
  id: number;
  payload: T;
}
export type UnlistenFn = () => void;
export type EventCallback<T> = (event: Event<T>) => void;

let ids = 0;

export async function listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  return getRuntime().on(event, (payload) => handler({ event, id: ++ids, payload: payload as T }));
}

export async function once<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  const un = await listen<T>(event, (e) => {
    un();
    handler(e);
  });
  return un;
}

export async function emit(event: string, payload?: unknown): Promise<void> {
  getRuntime().emit(event, payload);
}
