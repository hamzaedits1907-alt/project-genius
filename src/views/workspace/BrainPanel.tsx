import { useCallback, useEffect, useState } from 'react';
import { Network, RefreshCw, Search } from 'lucide-react';
import type { BrainAnswer, BrainOverview } from '../../lib/types';
import { api, errorMessage } from '../../lib/api';
import { timeAgo } from '../../lib/logic';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout, Spinner } from '../../components/ui';

/**
 * Project Brain: what Genius has verified about this project by scanning its files (overview, key
 * files, commands), and a question box. Verified source facts and recorded history (memory, earlier
 * changes, Git) are shown in separate sections on purpose; recorded notes that no longer match the
 * code are flagged.
 */
export default function BrainPanel() {
  const project = useWorkspace((s) => s.project);
  const toast = useApp((s) => s.toast);
  const [overview, setOverview] = useState<BrainOverview | null>(null);
  const [busy, setBusy] = useState(false);
  const [query, setQuery] = useState('');
  const [impact, setImpact] = useState('');
  const [answer, setAnswer] = useState<BrainAnswer | null>(null);
  const projectId = project?.id;

  const load = useCallback(() => {
    if (!projectId) return;
    setBusy(true);
    api
      .brainOverview(projectId)
      .then(setOverview)
      .catch((e) => toast('bad', errorMessage(e)))
      .finally(() => setBusy(false));
  }, [projectId, toast]);
  useEffect(() => {
    setAnswer(null);
    load();
  }, [load]);

  if (!project) return null;
  const ask = async () => {
    const paths = impact.split(/[\n,]/).map((p) => p.trim()).filter(Boolean);
    if (!query.trim() && !paths.length) return;
    setBusy(true);
    try {
      setAnswer(await api.brainAsk(project.id, query, paths));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="col" style={{ gap: 12, padding: 12, overflow: 'auto' }}>
      <div className="row">
        <Network size={16} />
        <strong className="grow">Project Brain</strong>
        <Button size="sm" variant="ghost" icon={<RefreshCw size={13} />} loading={busy && !answer} onClick={load} aria-label="Rescan project">
          Rescan
        </Button>
      </div>
      {!overview && busy && <Spinner />}
      {overview && overview.files === 0 && <div className="faint small">No supported source files yet (TypeScript, JavaScript, Python, Rust). The Brain fills in as the project grows.</div>}
      {overview && overview.files > 0 && (
        <div className="col" style={{ gap: 8 }}>
          <div className="small">
            {overview.projectType.map((t) => (
              <Badge key={t} tone="accent">
                {t}
              </Badge>
            ))}{' '}
            {overview.files} files · {overview.lines.toLocaleString()} lines · {overview.symbols} symbols · {overview.tests} test files
          </div>
          <div className="faint tiny" title={overview.provenance}>
            Verified by file scan {timeAgo(overview.indexedAt)}
            {overview.truncated ? ' · partial (project is larger than the index limit)' : ''} · pattern-based, not a compiler
          </div>
          {overview.entryPoints.length > 0 && (
            <div className="small">
              <strong>Entry points:</strong> <span className="mono">{overview.entryPoints.join(', ')}</span>
            </div>
          )}
          {overview.commands.length > 0 && (
            <div className="small">
              <strong>Commands:</strong>
              {overview.commands.map((c) => (
                <div key={c.command + c.purpose} title={c.source}>
                  {c.purpose}: <span className="mono">{c.command}</span>
                </div>
              ))}
            </div>
          )}
          {overview.keyFiles.length > 0 && (
            <div className="small">
              <strong>Key files:</strong>
              {overview.keyFiles.map((k) => (
                <div key={k.path}>
                  <span className="mono">{k.path}</span> <span className="faint tiny">{k.reason}</span>
                </div>
              ))}
            </div>
          )}
          {overview.unresolvedImports > 0 && <div className="faint tiny">{overview.unresolvedImports} relative imports matched no file in the project.</div>}
        </div>
      )}

      <div className="col" style={{ gap: 6 }}>
        <input className="input" placeholder="Find a symbol or topic (e.g. formatPrice)" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void ask()} aria-label="Ask the Project Brain" />
        <input className="input mono" placeholder="What does changing these files affect? (paths, comma-separated)" value={impact} onChange={(e) => setImpact(e.target.value)} onKeyDown={(e) => e.key === 'Enter' && void ask()} aria-label="Files to analyse for impact" />
        <div>
          <Button size="sm" icon={<Search size={13} />} loading={busy && !!overview} onClick={() => void ask()} disabled={!query.trim() && !impact.trim()}>
            Ask
          </Button>
        </div>
      </div>

      {answer && <AnswerView a={answer} />}
    </div>
  );
}

function AnswerView({ a }: { a: BrainAnswer }) {
  return (
    <div className="col" style={{ gap: 10 }}>
      <div>
        <strong className="small">Verified source facts</strong>
        {a.hits.length === 0 && !a.impact && <div className="faint small">No matching symbols or files.</div>}
        {a.hits.map((h) => (
          <div key={h.path + h.name + h.line} className="small">
            <span className="faint tiny">{h.kind}</span> <strong>{h.name}</strong> <span className="mono faint">{h.path}:{h.line}</span>
          </div>
        ))}
        {a.excerpts.slice(0, 2).map((e) => (
          <pre key={e.path + e.line} className="mono tiny" style={{ margin: '6px 0', overflow: 'auto' }}>
            {e.path}:{e.line}
            {'\n'}
            {e.text}
          </pre>
        ))}
        {a.impact && (
          <div className="small" style={{ marginTop: 6 }}>
            <strong>Affected if {a.impact.changed.join(', ') || 'these files'} change:</strong>
            {a.impact.unknown.length > 0 && <div className="faint tiny">Not in the index: {a.impact.unknown.join(', ')}</div>}
            {a.impact.affected.length === 0 && <div className="faint">No other scanned file imports them.</div>}
            {a.impact.affected.map((x) => (
              <div key={x.path} className="mono tiny">
                {'  '.repeat(x.depth - 1)}↳ {x.path}
              </div>
            ))}
            {a.impact.tests.length > 0 && (
              <div>
                Tests to run: <span className="mono">{a.impact.tests.join(', ')}</span>
              </div>
            )}
            <div className="faint tiny">{a.impact.note}</div>
          </div>
        )}
      </div>
      {(a.notes.length > 0 || a.pastChanges.length > 0 || a.gitHistory.length > 0) && (
        <div>
          <strong className="small">Recorded history</strong> <span className="faint tiny">— not verified against the code today</span>
          {a.notes.map((n, i) => (
            <div key={i} className="small" style={{ marginTop: 4 }}>
              <Badge tone={n.origin === 'agent inference' ? 'warn' : 'ok'}>{n.origin}</Badge> <span className="faint tiny">{n.updated}</span> {n.text}
              {n.stale.length > 0 && <Callout tone="warn">Possibly stale: {n.stale.join('; ')}</Callout>}
            </div>
          ))}
          {a.pastChanges.map((c, i) => (
            <div key={i} className="small">
              Change “{c.title}” <span className="faint tiny">({c.status}, {c.when})</span>: <span className="mono tiny">{c.files.join(', ')}</span>
            </div>
          ))}
          {a.gitHistory.map((g, i) => (
            <div key={i} className="small">
              <span className="mono tiny">{g.short}</span> <span className="faint tiny">{g.date}</span> {g.subject} <span className="mono faint tiny">{g.path}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
