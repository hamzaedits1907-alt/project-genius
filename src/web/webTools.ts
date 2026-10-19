// Web search and page reading for Genius Web, through the signed-in web proxy (/api/web). A browser cannot
// fetch arbitrary pages itself (cross-origin rules), and the proxy is also where the SSRF protection
// lives: it refuses private and local addresses and re-checks every redirect. The proxy checks the
// caller's session, suspension status and the "web_tools" kill switch before doing anything.
import type { WebPage, WebToolsApi } from './engine/chat';

export function proxyTools(opts: { url: string; accessToken: () => Promise<string>; fetchImpl?: typeof fetch }): WebToolsApi {
  const call = async <T>(body: Record<string, unknown>, signal?: AbortSignal): Promise<T> => {
    const f = opts.fetchImpl ?? fetch;
    const res = await f(opts.url, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${await opts.accessToken()}` }, body: JSON.stringify(body), signal });
    const text = await res.text();
    let j: unknown = null;
    try {
      j = JSON.parse(text);
    } catch {
      /* not JSON */
    }
    if (!res.ok) throw new Error((j as { error?: string } | null)?.error ?? `The web proxy answered HTTP ${res.status}.`);
    return j as T;
  };
  return {
    search: async (query, signal) => (await call<{ results: { title: string; url: string; snippet: string }[] }>({ action: 'search', query }, signal)).results,
    fetch: async (url, signal) => (await call<{ page: WebPage }>({ action: 'fetch', url }, signal)).page,
  };
}

/** Local-only development mode has no proxy: web tools say so honestly. */
export const noWebTools: WebToolsApi = {
  search: async () => {
    throw new Error('Web search needs the Genius web proxy, which is not available in local-only development mode.');
  },
  fetch: async () => {
    throw new Error('Reading pages needs the Genius web proxy, which is not available in local-only development mode.');
  },
};
