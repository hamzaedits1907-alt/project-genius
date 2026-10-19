import { useEffect, useState } from 'react';
import { BadgeCheck, ShieldAlert } from 'lucide-react';
import { openExternal } from '../../lib/platform';
import type { ResearchSource } from '../../lib/types';

/** The sources ledger of a research conversation: only pages that were really fetched. */
export function SourcesPanel({ sources }: { sources: ResearchSource[] }) {
  const [hl, setHl] = useState<number | null>(null);
  useEffect(() => {
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ n: number; handled: boolean } | number>).detail;
      const n = typeof d === 'number' ? d : d.n;
      if (typeof d === 'object') d.handled = true; // a panel is showing: the page does not need to open
      setHl(n);
      document.getElementById(`ledger-${n}`)?.scrollIntoView?.({ block: 'nearest', behavior: 'smooth' });
    };
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
                    void openExternal(s.url);
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
