import { useCallback, useEffect, useState } from 'react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { FileText, ImageIcon, Trash2, Upload } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { fileToBase64 } from '../../lib/agent';
import { bytes } from '../../lib/logic';
import type { Attachment } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout, Modal } from '../../components/ui';

/** Imported files: project reference material the agent can search, plus files sent with messages. */
export default function ReferencesPanel() {
  const projectId = useWorkspace((s) => s.projectId);
  const toast = useApp((s) => s.toast);
  const [items, setItems] = useState<Attachment[]>([]);
  const [preview, setPreview] = useState<{ attachment: Attachment; preview: string } | null>(null);
  const [dragging, setDragging] = useState(false);
  const load = useCallback(() => {
    if (projectId) api.listAttachments(projectId).then(setItems).catch((e) => toast('bad', errorMessage(e)));
  }, [projectId, toast]);
  useEffect(() => {
    load();
  }, [load]);
  if (!projectId) return null;

  const report = (a: Attachment) => (a.status === 'failed' ? toast('bad', `${a.name}: ${a.error}`) : toast('ok', `Imported ${a.name}`));
  const importPaths = async () => {
    const types = await api.supportedImportTypes();
    const chosen = await openDialog({ multiple: true, title: 'Import reference files', filters: [{ name: 'Supported files', extensions: types }] });
    const paths = Array.isArray(chosen) ? chosen : typeof chosen === 'string' ? [chosen] : [];
    for (const p of paths) {
      try {
        report(await api.importAttachment(projectId, p, 'reference'));
      } catch (e) {
        toast('bad', errorMessage(e));
      }
    }
    load();
  };
  const importDropped = async (files: File[]) => {
    for (const f of files) {
      try {
        report(await api.importAttachmentData(projectId, f.name, await fileToBase64(f), 'reference'));
      } catch (e) {
        toast('bad', `${f.name}: ${errorMessage(e)}`);
      }
    }
    load();
  };
  const act = async (fn: () => Promise<unknown>) => {
    try {
      await fn();
      load();
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">Imported files</span>
        <Button size="sm" icon={<Upload size={12} />} onClick={() => void importPaths()}>
          Import
        </Button>
      </div>
      <div
        className={`scroll grow ${dragging ? 'dropping' : ''}`}
        style={{ padding: 8 }}
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault();
          setDragging(false);
          void importDropped(Array.from(e.dataTransfer.files));
        }}
      >
        <div className="faint tiny" style={{ marginBottom: 8 }}>
          Drop specs, docs, data, code or screenshots here. <strong>Reference</strong> files are searchable by the agent in every conversation; <strong>message</strong> files were sent with one message only. Their text is sent to your NVIDIA model when used. Contents are treated as data, never as instructions.
        </div>
        {items.length === 0 && <div className="faint small">No files imported yet.</div>}
        {items.map((a) => (
          <div key={a.id} className="mem-row">
            <div className="row" style={{ gap: 6 }}>
              {a.kind === 'image' ? <ImageIcon size={13} /> : <FileText size={13} />}
              <button className="link grow ellipsis small" style={{ textAlign: 'left' }} onClick={() => void api.attachmentPreview(projectId, a.id).then(setPreview)}>
                {a.name}
              </button>
              <button className="icon-btn" aria-label={`Remove ${a.name}`} onClick={() => void act(() => api.deleteAttachment(projectId, a.id))}>
                <Trash2 size={12} />
              </button>
            </div>
            <div className="row" style={{ gap: 6 }}>
              <Badge tone={a.status === 'ready' ? 'ok' : 'bad'}>{a.status}</Badge>
              <Badge>{a.kind}</Badge>
              <span className="faint tiny grow">
                {bytes(a.size)}
                {a.chars ? ` · ${a.chars.toLocaleString()} chars` : ''}
              </span>
              {a.status === 'ready' && (
                <select className="input sm" aria-label={`Availability of ${a.name}`} value={a.scope} onChange={(e) => void act(() => api.setAttachmentScope(projectId, a.id, e.target.value as Attachment['scope']))}>
                  <option value="reference">reference</option>
                  <option value="message">message only</option>
                </select>
              )}
            </div>
            {a.error && <div className="bad-text tiny">{a.error}</div>}
          </div>
        ))}
      </div>
      {preview && (
        <Modal title={preview.attachment.name} onClose={() => setPreview(null)} wide footer={<Button onClick={() => setPreview(null)}>Close</Button>}>
          {preview.attachment.kind === 'image' ? (
            <Callout>Image stored for models with verified image input. It is not shown here.</Callout>
          ) : (
            <pre className="stream" style={{ maxHeight: 420 }}>
              {preview.preview || '(no text extracted)'}
              {preview.attachment.chars > preview.preview.length ? '\n…' : ''}
            </pre>
          )}
        </Modal>
      )}
    </>
  );
}
