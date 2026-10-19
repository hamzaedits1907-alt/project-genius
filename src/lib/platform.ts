// Where Genius is running, and what that environment can do. One build serves the Tauri desktop app;
// a second build (vite.web.config.ts) serves the browser. Code that needs a native capability asks
// here instead of calling Tauri directly, so the browser never pretends to have a terminal or a
// filesystem.
import { openUrl } from '@tauri-apps/plugin-opener';

declare const __GENIUS_TARGET__: string | undefined;

/** True in the browser build (Genius Web). */
export const isWeb: boolean = typeof __GENIUS_TARGET__ !== 'undefined' && __GENIUS_TARGET__ === 'web';

export interface Capabilities {
  /** Local project folders, terminal, git, build and preview (desktop only). */
  localProjects: boolean;
  /** Provider keys kept in the operating system's credential store (desktop only). */
  secureKeyStore: boolean;
  /** Accounts, synced conversations and the owner admin panel (browser only). */
  accounts: boolean;
  /** Updates through GitHub Releases (desktop only). */
  selfUpdate: boolean;
}

export const capabilities: Capabilities = isWeb
  ? { localProjects: false, secureKeyStore: false, accounts: true, selfUpdate: false }
  : { localProjects: true, secureKeyStore: true, accounts: false, selfUpdate: true };

const PRIVATE_V4 = /^(10\.|127\.|0\.|169\.254\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.)/;

/**
 * Whether `href` is an ordinary public web address that may be shown as a link. Anything that points
 * at the app itself or the user's machine (`http://tauri.localhost/#src-1`, `localhost`, private
 * ranges, other schemes) is not a source and is never rendered as one.
 */
export function isPublicHttpUrl(href: string): boolean {
  let u: URL;
  try {
    u = new URL(href);
  } catch {
    return false;
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return false;
  const h = u.hostname.toLowerCase().replace(/\.$/, '');
  if (!h || h === 'localhost' || h.endsWith('.localhost') || h.endsWith('.local') || h.endsWith('.internal')) return false;
  if (h === '[::1]' || h === '::1' || h.startsWith('[fe80') || h.startsWith('[fc') || h.startsWith('[fd')) return false;
  if (PRIVATE_V4.test(h)) return false;
  return true;
}

/** Open an external address in the system browser (desktop) or a new tab (web). Refuses non-public addresses. */
export async function openExternal(href: string): Promise<boolean> {
  if (!isPublicHttpUrl(href)) return false;
  if (!isWeb) {
    try {
      await openUrl(href);
      return true;
    } catch {
      /* fall through to the browser API */
    }
  }
  window.open(href, '_blank', 'noopener,noreferrer');
  return true;
}
