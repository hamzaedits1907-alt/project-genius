// "Done" notifications: what is announced, only while the window is in the background,
// once per event, with a title flash and a system notification or chime fallback.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { noticeFor, notifyIfAway, setNotificationsEnabled, stopFlash } from '../lib/notify';
import type { ActivityEvent } from '../lib/types';

const ev = (over: Partial<ActivityEvent>): ActivityEvent => ({ id: Math.random().toString(36), projectId: 'p1', taskId: 't1', kind: 'task', status: 'completed', title: 'Generate the application', detail: { taskKind: 'generate' }, ts: '', updatedTs: '', ...over });

describe('notices', () => {
  it('announces long AI tasks finishing or failing, and approvals', () => {
    expect(noticeFor(ev({}), 'Shop')).toMatchObject({ headline: 'Genius finished — Shop', body: 'Your app has been built.' });
    expect(noticeFor(ev({ detail: { taskKind: 'plan' } }), 'Shop')?.body).toMatch(/plan is ready/);
    expect(noticeFor(ev({ detail: { taskKind: 'agent' }, title: 'Genius: add a cart' }), 'Shop')?.body).toBe('Done: Genius: add a cart');
    expect(noticeFor(ev({ status: 'failed', detail: { taskKind: 'repair' } }), '')?.headline).toBe('Genius needs you — your project');
    expect(noticeFor(ev({ kind: 'approval', status: 'awaiting_approval', title: 'Install dependencies' }), 'Shop')).toMatchObject({ headline: 'Genius needs your approval — Shop', body: 'Install dependencies' });
    expect(noticeFor(ev({ kind: 'changeset', status: 'awaiting_approval', title: 'Review changes' }), 'Shop')).not.toBeNull();
  });

  it('ignores running, cancelled, short and unrelated events', () => {
    expect(noticeFor(ev({ status: 'running' }), 'x')).toBeNull();
    expect(noticeFor(ev({ status: 'cancelled' }), 'x')).toBeNull();
    expect(noticeFor(ev({ detail: { taskKind: 'verify' } }), 'x')).toBeNull();
    expect(noticeFor(ev({ kind: 'build', status: 'failed' }), 'x')).toBeNull();
  });
});

describe('notifyIfAway', () => {
  let focused = true;
  beforeEach(() => {
    focused = true;
    vi.spyOn(document, 'hasFocus').mockImplementation(() => focused);
    document.title = 'Genius';
    localStorage.removeItem('genius.notifications');
    vi.useFakeTimers();
  });
  afterEach(() => {
    stopFlash();
    vi.useRealTimers();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('stays quiet while the user is looking', async () => {
    expect(await notifyIfAway(ev({}), 'Shop')).toBeNull();
    expect(document.title).toBe('Genius');
  });

  it('flashes the title and shows a system notification once, until focus returns', async () => {
    const shown: { title: string; body?: string }[] = [];
    class FakeNotification {
      static permission = 'granted';
      static requestPermission = vi.fn(async () => 'granted');
      onclick: (() => void) | null = null;
      constructor(title: string, opts?: { body?: string }) {
        shown.push({ title, body: opts?.body });
      }
      close() {}
    }
    vi.stubGlobal('Notification', FakeNotification);
    focused = false;
    const e = ev({ detail: { taskKind: 'plan' } });
    expect(await notifyIfAway(e, 'Shop')).not.toBeNull();
    expect(shown).toEqual([{ title: 'Genius finished — Shop', body: 'The plan is ready for your review.' }]);
    expect(document.title).toBe('● Genius finished — Shop');
    vi.advanceTimersByTime(1100);
    expect(document.title).toBe('Genius');
    vi.advanceTimersByTime(1100);
    expect(document.title).toBe('● Genius finished — Shop');
    expect(await notifyIfAway(e, 'Shop')).toBeNull(); // same event: announced once
    window.dispatchEvent(new Event('focus'));
    expect(document.title).toBe('Genius');
    vi.advanceTimersByTime(5000);
    expect(document.title).toBe('Genius');
  });

  it('falls back to a chime without the Notification API, and respects the preference', async () => {
    vi.stubGlobal('Notification', undefined);
    class FakeAudio {
      currentTime = 0;
      destination = {};
      createOscillator() {
        return { type: '', frequency: { value: 0 }, connect: (g: unknown) => g, start() {}, stop() {} };
      }
      createGain() {
        return { gain: { setValueAtTime() {}, linearRampToValueAtTime() {}, exponentialRampToValueAtTime() {} }, connect: (d: unknown) => d };
      }
      close = async () => {};
    }
    const created = vi.fn();
    vi.stubGlobal('AudioContext', function (this: unknown) {
      created();
      return new FakeAudio();
    });
    focused = false;
    expect(await notifyIfAway(ev({ status: 'failed' }), 'Shop')).toMatchObject({ headline: 'Genius needs you — Shop' });
    expect(created).toHaveBeenCalledOnce();
    expect(document.title).toBe('● Genius needs you — Shop');

    setNotificationsEnabled(false);
    stopFlash();
    expect(await notifyIfAway(ev({}), 'Shop')).toBeNull();
    expect(document.title).toBe('Genius');
  });
});
