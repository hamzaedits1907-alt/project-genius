import { useState } from 'react';
import { Archive, Folder } from 'lucide-react';
import { open as openDialog, save as saveDialog } from '@tauri-apps/plugin-dialog';
import { api, errorMessage } from '../../lib/api';
import { bytes } from '../../lib/logic';
import type { ExportResult } from '../../lib/types';
import { useWorkspace } from '../../store/workspace';
import { Button, Callout, Modal } from '../../components/ui';

export default function ExportDialog({ onClose }: { onClose: () => void }) {
  const project = useWorkspace((s) => s.project)!;
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ExportResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const run = async (zip: boolean) => {
    setErr(null);
    const dest = zip
      ? await saveDialog({ title: 'Export source as .zip', defaultPath: `${project.name.replace(/[^\w-]+/g, '-').toLowerCase()}.zip`, filters: [{ name: 'Zip archive', extensions: ['zip'] }] })
      : await openDialog({ directory: true, title: 'Choose an empty folder for the exported source' });
    if (typeof dest !== 'string') return;
    setBusy(true);
    try {
      setResult(await api.exportProject(project.id, dest, zip));
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title="Export source code" onClose={onClose} footer={<Button onClick={onClose}>Close</Button>}>
      <p className="muted" style={{ margin: 0 }}>
        Exports the real project files and configuration. node_modules, build output, .git and secret files such as .env are excluded. Run <span className="mono">npm install</span> after extracting.
      </p>
      <div className="row">
        <Button icon={<Archive size={15} />} loading={busy} onClick={() => void run(true)}>
          Export as .zip
        </Button>
        <Button icon={<Folder size={15} />} disabled={busy} onClick={() => void run(false)}>
          Copy to folder
        </Button>
      </div>
      {err && <Callout tone="bad">{err}</Callout>}
      {result && (
        <Callout tone="ok" title="Export complete">
          {result.files} files ({bytes(result.bytes)}) written to {result.destination}
          {result.skipped.length > 0 && `\nExcluded for safety: ${result.skipped.join(', ')}`}
        </Callout>
      )}
    </Modal>
  );
}
