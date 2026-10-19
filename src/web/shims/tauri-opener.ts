// Browser replacement for '@tauri-apps/plugin-opener'. Only public http(s) addresses are opened.
import { isPublicHttpUrl } from '../../lib/platform';

export async function openUrl(url: string): Promise<void> {
  if (!isPublicHttpUrl(url)) throw new Error('That address is not a public web address.');
  window.open(url, '_blank', 'noopener,noreferrer');
}

export async function openPath(): Promise<void> {
  throw new Error('Opening local files is only available in Genius Desktop.');
}
