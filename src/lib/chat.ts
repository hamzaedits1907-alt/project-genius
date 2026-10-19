// Small pure helpers for the conversation UI (grouping, labels), kept out of components so they can be tested.
import type { ChatConversation } from './types';

export type HistoryGroup = { label: string; items: ChatConversation[] };

/** Conversations grouped by recency (Today, Yesterday, Previous 7 days, Previous 30 days, Older), newest first. */
export function groupByRecency(list: ChatConversation[], now: Date = new Date()): HistoryGroup[] {
  const day = 86_400_000;
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const buckets: HistoryGroup[] = [
    { label: 'Today', items: [] },
    { label: 'Yesterday', items: [] },
    { label: 'Previous 7 days', items: [] },
    { label: 'Previous 30 days', items: [] },
    { label: 'Older', items: [] },
  ];
  for (const c of [...list].sort((a, b) => Date.parse(b.updatedAt) - Date.parse(a.updatedAt))) {
    const t = Date.parse(c.updatedAt);
    const age = Number.isFinite(t) ? startOfToday - t : Infinity;
    const i = age <= 0 ? 0 : age <= day ? 1 : age <= 7 * day ? 2 : age <= 30 * day ? 3 : 4;
    buckets[i].items.push(c);
  }
  return buckets.filter((b) => b.items.length > 0);
}

/** The web pages a Chat answer really read (from the tool log), host + url, without duplicates. */
export function webSourcesOf(report: { webSources?: unknown } | undefined): { url: string; host: string }[] {
  const raw = Array.isArray(report?.webSources) ? report!.webSources : [];
  const seen = new Set<string>();
  const out: { url: string; host: string }[] = [];
  for (const u of raw) {
    if (typeof u !== 'string' || seen.has(u) || !/^https?:\/\//i.test(u)) continue;
    seen.add(u);
    let host = u;
    try {
      host = new URL(u).host.replace(/^www\./, '');
    } catch {
      /* keep the raw string */
    }
    out.push({ url: u, host });
  }
  return out;
}

/** Facts about how an answer was produced, for the "Details" disclosure (nothing here is invented). */
export function answerDetails(report: Record<string, unknown> | undefined): { label: string; value: string }[] {
  if (!report) return [];
  const rows: { label: string; value: string }[] = [];
  const model = typeof report.model === 'string' ? report.model : '';
  const reported = typeof report.reportedModel === 'string' ? report.reportedModel : '';
  if (model) rows.push({ label: 'Model', value: model });
  if (reported && reported !== model) rows.push({ label: 'Provider labelled it', value: reported });
  const vision = report.vision as { route?: string; answeredBy?: string; images?: number; reason?: string } | null | undefined;
  if (vision?.route) rows.push({ label: 'Image', value: `${vision.images ?? 1} image(s) sent to ${vision.answeredBy ?? 'a vision model'}${vision.reason ? ` — ${vision.reason}` : ''}` });
  const usage = report.usage as { totalTokens?: number; promptTokens?: number; completionTokens?: number; calls?: number } | undefined;
  if (usage?.totalTokens) rows.push({ label: 'Tokens', value: `${usage.totalTokens} (${usage.promptTokens ?? 0} in, ${usage.completionTokens ?? 0} out) over ${usage.calls ?? 1} request(s)` });
  const timing = report.timing as { totalMs?: number } | undefined;
  if (timing?.totalMs) rows.push({ label: 'Time', value: `${(timing.totalMs / 1000).toFixed(1)} s` });
  const calls = Array.isArray(report.modelCalls) ? (report.modelCalls as { firstTokenMs?: number | null }[]) : [];
  const first = calls[0]?.firstTokenMs;
  if (typeof first === 'number' && first > 0) rows.push({ label: 'First token', value: `${(first / 1000).toFixed(1)} s` });
  const searches = typeof report.webSearches === 'number' ? report.webSearches : 0;
  if (searches) rows.push({ label: 'Web searches', value: String(searches) });
  return rows;
}
