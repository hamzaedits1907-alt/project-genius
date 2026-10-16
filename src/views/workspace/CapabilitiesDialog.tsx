import { useEffect, useState } from 'react';
import { ShieldCheck } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import type { CapabilityReport } from '../../lib/types';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout, Modal, Spinner } from '../../components/ui';

/** Genius's live self-description: every row comes from runtime state, with its evidence. */
export default function CapabilitiesDialog({ onClose }: { onClose: () => void }) {
  const projectId = useWorkspace((s) => s.projectId);
  const [report, setReport] = useState<CapabilityReport | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api
      .capabilityReport(projectId ?? undefined)
      .then(setReport)
      .catch((e) => setErr(errorMessage(e)));
  }, [projectId]);
  const tone = (s: string) => (s === 'available' || s === 'connected' ? 'ok' : s === 'unavailable' || s === 'not_connected' ? 'bad' : 'warn');
  return (
    <Modal title="What Genius can do right now" icon={<ShieldCheck size={16} />} onClose={onClose} wide footer={<Button onClick={onClose}>Close</Button>}>
      {err && <Callout tone="bad">{err}</Callout>}
      {!report && !err && <Spinner />}
      {report && (
        <div className="col" style={{ gap: 12 }}>
          <div className="small faint">
            Generated {new Date(report.generatedAt).toLocaleString()} from live state. Provider: {report.provider.name} · model {report.provider.model ?? 'none selected'}.
          </div>
          <table className="table small">
            <tbody>
              {report.capabilities.map((c) => (
                <tr key={c.id}>
                  <td>{c.label}</td>
                  <td>
                    <Badge tone={tone(c.status)}>{c.status}</Badge>
                  </td>
                  <td>
                    {c.evidence}
                    {c.remedy && <div className="faint">To enable: {c.remedy}</div>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div>
            <strong className="small">Integrations</strong>
            <ul className="small">
              {report.integrations.map((i) => (
                <li key={i.id}>
                  {i.name ?? i.id}: <Badge tone={tone(i.status)}>{i.status.replace('_', ' ')}</Badge> {i.login ? `as ${i.login}` : ''} {i.detail ?? ''}
                </li>
              ))}
            </ul>
          </div>
          <div>
            <strong className="small">Agent tools</strong>
            <ul className="small">
              {report.agentTools.map((t) => (
                <li key={t.name}>
                  <span className="mono">{t.name}</span> {t.readOnly ? '' : <Badge tone="warn">acts</Badge>} — {t.description}
                </li>
              ))}
            </ul>
          </div>
          {report.project && (
            <details>
              <summary className="small">Project state</summary>
              <pre className="stream">{JSON.stringify(report.project, null, 2)}</pre>
            </details>
          )}
          <div className="small faint">Known limitations: {report.limitations.join('; ')}.</div>
        </div>
      )}
    </Modal>
  );
}
