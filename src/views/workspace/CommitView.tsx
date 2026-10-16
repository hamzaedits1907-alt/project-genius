import { useEffect, useState } from 'react';
import { api, errorMessage } from '../../lib/api';
import { useWorkspace } from '../../store/workspace';
import { DiffText, Empty, Spinner } from '../../components/ui';

export default function CommitView({ sha }: { sha: string }) {
  const projectId = useWorkspace((s) => s.projectId);
  const [data, setData] = useState<{ files: [string, string][]; diff: string } | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    if (projectId) api.gitCommitDetail(projectId, sha).then(setData).catch((e) => setErr(errorMessage(e)));
  }, [projectId, sha]);
  if (err) return <Empty title="Could not load commit">{err}</Empty>;
  if (!data) return <Empty title="Loading commit…" icon={<Spinner />} />;
  return (
    <div className="scroll" style={{ height: '100%' }}>
      <div className="banner" style={{ background: 'var(--bg-1)' }}>
        <span className="mono small">{sha}</span>
        <span className="faint small">{data.files.length} file(s)</span>
      </div>
      <div style={{ padding: '6px 12px' }} className="small">
        {data.files.map(([st, p]) => (
          <div key={p} className="mono tiny">
            <strong>{st}</strong> {p}
          </div>
        ))}
      </div>
      <DiffText diff={data.diff} />
    </div>
  );
}
