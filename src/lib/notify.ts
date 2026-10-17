// "Done" notifications: when a long AI task finishes (or needs the user) while the window is
// in the background, flash the window title and show a system notification. Uses the Web
// Notification API; where that is unavailable or denied, a soft chime plays instead.
import type { ActivityEvent } from './types';

const KEY = 'genius.notifications';

/** Task kinds that take long enough to be worth a notification. */
export const LONG_TASKS = ['plan', 'generate', 'agent', 'change', 'repair'];

export function notificationsEnabled(): boolean {
  try {
    return localStorage.getItem(KEY) !== 'off';
  } catch {
    return true;
  }
}

export function setNotificationsEnabled(on: boolean) {
  try {
    localStorage.setItem(KEY, on ? 'on' : 'off');
  } catch {
    /* storage unavailable: the default (on) applies */
  }
  if (on) void requestPermission();
}

export interface Notice {
  /** Short headline, also used for the flashing window title. */
  headline: string;
  body: string;
  /** Collapses repeated notices for the same event. */
  tag: string;
}

const DONE_TEXT: Record<string, string> = {
  plan: 'The plan is ready for your review.',
  generate: 'Your app has been built.',
  repair: 'Build Doctor has finished.',
};

/** What to announce for an activity event, or null when it is not worth interrupting for. */
export function noticeFor(ev: ActivityEvent, projectName: string): Notice | null {
  const name = projectName || 'your project';
  if (ev.kind === 'task') {
    const kind = typeof ev.detail.taskKind === 'string' ? ev.detail.taskKind : '';
    if (!LONG_TASKS.includes(kind)) return null;
    if (ev.status === 'completed') return { headline: `Genius finished — ${name}`, body: DONE_TEXT[kind] ?? `Done: ${ev.title}`, tag: ev.id };
    if (ev.status === 'failed') return { headline: `Genius needs you — ${name}`, body: `${ev.title} did not finish. Open Genius to see what happened.`, tag: ev.id };
    return null;
  }
  if ((ev.kind === 'approval' || ev.kind === 'changeset') && ev.status === 'awaiting_approval') {
    return { headline: `Genius needs your approval — ${name}`, body: ev.title, tag: ev.id };
  }
  return null;
}

/** True while the user is looking at Genius. */
export function windowFocused(): boolean {
  if (typeof document === 'undefined') return true;
  return document.visibilityState !== 'hidden' && document.hasFocus();
}

// ------------------------------------------------------------------ title flash

let flashTimer: ReturnType<typeof setInterval> | null = null;
let baseTitle: string | null = null;

/** Alternates the window title with the message until the window regains focus. */
export function flashTitle(message: string) {
  if (typeof document === 'undefined') return;
  stopFlash();
  baseTitle = document.title;
  const text = `● ${message}`;
  let on = true;
  document.title = text;
  flashTimer = setInterval(() => {
    on = !on;
    document.title = on ? text : (baseTitle ?? 'Genius');
  }, 1100);
  window.addEventListener('focus', stopFlash, { once: true });
}

export function stopFlash() {
  if (flashTimer) clearInterval(flashTimer);
  flashTimer = null;
  if (baseTitle !== null) document.title = baseTitle;
  baseTitle = null;
  window.removeEventListener('focus', stopFlash);
}

// ------------------------------------------------------------------ system notification + chime

async function requestPermission(): Promise<NotificationPermission | 'unsupported'> {
  if (typeof Notification === 'undefined') return 'unsupported';
  if (Notification.permission !== 'default') return Notification.permission;
  try {
    return await Notification.requestPermission();
  } catch {
    return 'denied';
  }
}

/** A short, quiet two-note chime (no audio files needed). */
export function chime() {
  try {
    const Ctx = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!Ctx) return;
    const ctx = new Ctx();
    const t0 = ctx.currentTime;
    [659.25, 987.77].forEach((freq, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'sine';
      osc.frequency.value = freq;
      const start = t0 + i * 0.14;
      gain.gain.setValueAtTime(0, start);
      gain.gain.linearRampToValueAtTime(0.06, start + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.5);
      osc.connect(gain).connect(ctx.destination);
      osc.start(start);
      osc.stop(start + 0.55);
    });
    setTimeout(() => void ctx.close().catch(() => {}), 1200);
  } catch {
    /* audio unavailable: the title flash is enough */
  }
}

async function systemNotify(n: Notice, onClick?: () => void): Promise<boolean> {
  if ((await requestPermission()) !== 'granted') return false;
  try {
    const note = new Notification(n.headline, { body: n.body, tag: n.tag });
    note.onclick = () => {
      window.focus();
      onClick?.();
      note.close();
    };
    return true;
  } catch {
    return false; // some webviews expose the API but refuse to construct notifications
  }
}

const announced = new Set<string>();

/**
 * Announces the event if it matters and the user is away. Returns the notice that was shown
 * (null when nothing was shown). Each event/status pair is announced at most once.
 */
export async function notifyIfAway(ev: ActivityEvent, projectName: string, onClick?: () => void): Promise<Notice | null> {
  if (!notificationsEnabled() || windowFocused()) return null;
  const n = noticeFor(ev, projectName);
  if (!n) return null;
  const key = `${ev.id}:${ev.status}`;
  if (announced.has(key)) return null;
  announced.add(key);
  if (announced.size > 500) announced.clear();
  flashTitle(n.headline);
  if (!(await systemNotify(n, onClick))) chime();
  return n;
}
