import { useState } from 'react';
import { api, errorMessage } from '../../lib/api';
import type { SearchHit } from '../../lib/types';
import { useWorkspace } from '../../store/workspace';
import { Empty, Spinner } from '../../components/ui';

export default function SearchPanel() {
  const projectId = useWorkspace((s) => s.projectId);
  const openFile = useWorkspace((s) => s.openFile);
  const [q, setQ] = useState('');
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const run = async () => {
    if (!projectId || !q.trim()) return;
    setBusy(true);
    setErr(null);
    try {
      setHits(await api.searchFiles(projectId, q));
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const grouped = (hits ?? []).reduce<Record<string, SearchHit[]>>((acc, h) => ((acc[h.path] ??= []).push(h), acc), {});
  return (
    <>
      <div className="panel-head">
        <span className="panel-title">Search</span>
      </div>
      <div style={{ padding: 8 }}>
        <input className="input" placeholder="Search in files (Enter)" value={q} onChange={(e) => setQ(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void run()} aria-label="Search text" autoFocus />
      </div>
      <div className="scroll grow">
        {busy && (
          <div className="row muted small" style={{ padding: 10 }}>
            <Spinner /> Searching…
          </div>
        )}
        {err && <div className="small" style={{ color: 'var(--bad)', padding: 10 }}>{err}</div>}
        {hits && hits.length === 0 && <Empty title="No matches" />}
        {Object.entries(grouped).map(([path, hs]) => (
          <div key={path}>
            <div className="small" style={{ padding: '6px 10px 2px', fontWeight: 600 }}>
              {path}
            </div>
            {hs.map((h) => (
              <button key={`${h.path}:${h.line}`} className="tree-row" style={{ paddingLeft: 18 }} onClick={() => void openFile(h.path)}>
                <span className="faint mono tiny" style={{ width: 30 }}>
                  {h.line}
                </span>
                <span className="ellipsis mono tiny">{h.text}</span>
              </button>
            ))}
          </div>
        ))}
      </div>
    </>
  );
}
