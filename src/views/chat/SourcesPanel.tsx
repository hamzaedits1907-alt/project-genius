import { useEffect, useState } from 'react';
import { BadgeCheck, ShieldAlert } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import type { ResearchSource } from '../../lib/types';

/** The sources ledger of a research conversation: only pages that were really fetched. */
export function SourcesPanel({ sources }: { sources: ResearchSource[] }) {
  const [hl, setHl] = useState<number | null>(null);
  useEffect(() => {
    const on = (e: Event) => setHl(Number((e as CustomEvent).detail));
    window.addEventListener('genius-cite', on);
    return () => window.removeEventListener('genius-cite', on);
  }, []);
  return (
    <aside className="sources-panel" aria-label="Sources">
      <h3>Sources ({sources.length})</h3>
      {sources.length === 0 ? (
        <p className="faint small">Pages Genius reads appear here. Search results are not sources until the page was actually fetched.</p>
      ) : (
        <ol>
          {sources.map((s) => (
            <li key={s.id} id={`ledger-${s.n}`} className={hl === s.n ? 'hl' : ''} value={s.n}>
              <div className="row" style={{ gap: 6 }}>
                <span className="src-n">[{s.n}]</span>
                <a
                  href={s.url}
                  className="grow ellipsis"
                  title={s.url}
                  onClick={(e) => {
                    e.preventDefault();
                    void openUrl(s.url).catch(() => window.open(s.url, '_blank', 'noopener'));
                  }}
                >
                  {s.title || s.url}
                </a>
                {s.official && (
                  <span className="badge ok" title="The domain looks like official documentation or a primary source">
                    <BadgeCheck size={11} /> official
                  </span>
                )}
              </div>
              <div className="faint tiny ellipsis">{s.url}</div>
              <div className="faint tiny">
                Fetched {s.fetchedAt.slice(0, 16).replace('T', ' ')} · HTTP {s.status} · <span className="mono" title={s.contentHash}>{s.contentHash.slice(0, 10)}</span>
              </div>
              <div className="small">{s.usedFor ? <>Used for: {s.usedFor}</> : <span className="faint">Fetched, not cited in the report</span>}</div>
              {s.flags.length > 0 && (
                <div className="small bad-text">
                  <ShieldAlert size={12} /> Contains text aimed at the AI (ignored): “{s.flags[0]}”
                </div>
              )}
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
