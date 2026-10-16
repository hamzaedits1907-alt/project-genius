import { useState } from 'react';
import { ShieldAlert } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { ApprovalRequest } from '../lib/types';
import { useApp } from '../store/app';
import { Button, Callout, Modal } from '../components/ui';

const CATEGORY_TEXT: Record<string, string> = {
  dependency_install: 'Installing npm packages downloads code from the npm registry and runs their install scripts on your computer.',
  untrusted_script: 'This npm script is different from the one Genius created. It will run as a program on your computer with your user permissions.',
  git_push: 'This uploads commits to a remote repository. Anyone with access to that repository will be able to see them.',
  git_commit: 'This records the staged changes in the repository history.',
};

/** Shows exactly what a sensitive operation will do; nothing runs until the user decides. */
export default function ApprovalDialog({ request, remaining }: { request: ApprovalRequest; remaining: number }) {
  const [busy, setBusy] = useState(false);
  const refresh = useApp((s) => s.refreshApprovals);
  const toast = useApp((s) => s.toast);
  const respond = async (approved: boolean) => {
    setBusy(true);
    try {
      await api.respondApproval(request.id, approved);
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
      void refresh();
    }
  };
  const d = request.detail as Record<string, unknown>;
  return (
    <Modal
      title="Approval required"
      icon={<ShieldAlert size={18} color="var(--warn)" />}
      onClose={() => void respond(false)}
      footer={
        <>
          {remaining > 0 && <span className="faint small spacer">{remaining} more waiting</span>}
          <Button onClick={() => void respond(false)} disabled={busy}>
            Deny
          </Button>
          <Button variant="primary" onClick={() => void respond(true)} loading={busy}>
            Approve
          </Button>
        </>
      }
    >
      <div style={{ fontWeight: 650, fontSize: 14 }}>{request.title}</div>
      {CATEGORY_TEXT[request.category] && <Callout tone="warn">{CATEGORY_TEXT[request.category]}</Callout>}
      {typeof d.command === 'string' && (
        <div className="field">
          <label>Command</label>
          <pre className="block">{d.command}</pre>
        </div>
      )}
      {typeof d.cwd === 'string' && (
        <div className="field">
          <label>Working directory</label>
          <pre className="block">{d.cwd}</pre>
        </div>
      )}
      {typeof d.remote === 'string' && (
        <div className="field">
          <label>Remote</label>
          <pre className="block">
            {d.remote}
            {typeof d.branch === 'string' ? `  (branch ${d.branch})` : ''}
            {typeof d.visibility === 'string' ? `  · ${d.visibility}` : ''}
          </pre>
        </div>
      )}
      {d.packages != null && (
        <div className="field">
          <label>Packages from package.json</label>
          <pre className="block">{JSON.stringify(d.packages, null, 2)}</pre>
        </div>
      )}
      {typeof d.note === 'string' && <div className="muted small">{d.note}</div>}
      <div className="faint tiny">Category: {request.category}. You can change which actions ask first in Settings → Approvals.</div>
    </Modal>
  );
}
