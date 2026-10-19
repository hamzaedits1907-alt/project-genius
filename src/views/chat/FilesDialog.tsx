import { useEffect, useState } from 'react';
import { FileText, ImageIcon, Trash2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { bytes, timeAgo } from '../../lib/logic';
import type { Attachment } from '../../lib/types';
import { useApp } from '../../store/app';
import { useChat } from '../../store/chat';
import { Button, Modal } from '../../components/ui';

/** Everything attached in Chat and Research, with a preview and delete. Reuses the attachment store (no second file system). */
export function FilesDialog({ onClose }: { onClose: () => void }) {
  const toast = useApp((s) => s.toast);
  const [files, setFiles] = useState<Attachment[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [preview, setPreview] = useState<{ text: string; imageUrl: string | null } | null>(null);
  const [ws, setWs] = useState<string | null>(useChat.getState().workspaceId);

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const id = ws ?? (await api.chatWorkspace()).id;
        if (!alive) return;
        setWs(id);
        const list = await api.listAttachments(id);
        if (alive) setFiles(list.filter((a) => a.status === 'ready').sort((a, b) => b.createdAt.localeCompare(a.createdAt)));
      } catch (e) {
        if (alive) {
          setFiles([]);
          toast('bad', errorMessage(e));
        }
      }
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const open = async (a: Attachment) => {
    if (!ws) return;
    setSelected(a.id);
    setPreview(null);
    try {
      const r = await api.attachmentPreview(ws, a.id);
      setPreview({ text: r.preview, imageUrl: r.imageUrl ?? null });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const remove = async (a: Attachment) => {
    if (!ws || !window.confirm(`Delete “${a.name}”? Conversations that used it keep their text, but the file can no longer be opened.`)) return;
    try {
      await api.deleteAttachment(ws, a.id);
      setFiles((cur) => (cur ?? []).filter((x) => x.id !== a.id));
      if (selected === a.id) {
        setSelected(null);
        setPreview(null);
      }
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  return (
    <Modal title="Files" onClose={onClose} wide footer={<Button onClick={onClose}>Close</Button>}>
      {files === null && <div className="faint small">Loading…</div>}
      {files?.length === 0 && <div className="faint small">Nothing attached yet. Attach files from the message box; they are kept here, on this computer.</div>}
      {files && files.length > 0 && (
        <div className="gx-files">
          <ul aria-label="Attached files">
            {files.map((a) => (
              <li key={a.id} className={a.id === selected ? 'on' : ''}>
                <button className="gx-file" onClick={() => void open(a)}>
                  {a.kind === 'image' ? <ImageIcon size={14} /> : <FileText size={14} />}
                  <span className="ellipsis grow">{a.name}</span>
                  <span className="faint tiny">{bytes(a.size)} · {timeAgo(a.createdAt)}</span>
                </button>
                <button className="icon-btn" aria-label={`Delete ${a.name}`} onClick={() => void remove(a)}>
                  <Trash2 size={13} />
                </button>
              </li>
            ))}
          </ul>
          <div className="gx-file-preview" aria-label="Preview">
            {!selected && <div className="faint small">Choose a file to preview it.</div>}
            {selected && !preview && <div className="faint small">Loading…</div>}
            {preview?.imageUrl && <img src={preview.imageUrl} alt={files.find((f) => f.id === selected)?.name ?? 'Attached image'} />}
            {preview && !preview.imageUrl && <pre className="block">{preview.text || '(no text could be extracted)'}</pre>}
          </div>
        </div>
      )}
    </Modal>
  );
}
