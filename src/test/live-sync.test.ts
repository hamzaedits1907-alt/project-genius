import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { startLiveRefresh } from '../web/liveSync';

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe('live refresh', () => {
  it('collapses a burst of remote changes into one refresh', async () => {
    const refresh = vi.fn(async () => undefined);
    const stop = startLiveRefresh(null, 'u1', { refresh, busy: () => false, pollMs: 1000, debounceMs: 100 });
    window.dispatchEvent(new Event('online'));
    window.dispatchEvent(new Event('online'));
    window.dispatchEvent(new Event('online'));
    await vi.advanceTimersByTimeAsync(150);
    expect(refresh).toHaveBeenCalledTimes(1);
    stop();
  });

  it('polls as the fallback when realtime is not available', async () => {
    const refresh = vi.fn(async () => undefined);
    const stop = startLiveRefresh(null, 'u1', { refresh, busy: () => false, pollMs: 1000, debounceMs: 10 });
    await vi.advanceTimersByTimeAsync(2500);
    expect(refresh.mock.calls.length).toBeGreaterThanOrEqual(2);
    stop();
  });

  it('never refreshes while this client is answering, and stops after stop()', async () => {
    const refresh = vi.fn(async () => undefined);
    let busy = true;
    const stop = startLiveRefresh(null, 'u1', { refresh, busy: () => busy, pollMs: 500, debounceMs: 10 });
    await vi.advanceTimersByTimeAsync(1200);
    expect(refresh).not.toHaveBeenCalled();
    busy = false;
    await vi.advanceTimersByTimeAsync(600);
    expect(refresh).toHaveBeenCalled();
    stop();
    const n = refresh.mock.calls.length;
    await vi.advanceTimersByTimeAsync(3000);
    expect(refresh.mock.calls.length).toBe(n);
  });
});
