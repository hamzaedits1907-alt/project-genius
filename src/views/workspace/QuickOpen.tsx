import { useMemo, useState } from 'react';
import { useWorkspace } from '../../store/workspace';
import { Modal } from '../../components/ui';

export default function QuickOpen({ onClose }: { onClose: () => void }) {
  const files = useWorkspace((s) => s.files);
  const openFile = useWorkspace((s) => s.openFile);
  const [q, setQ] = useState('');
  const [sel, setSel] = useState(0);
  const shown = useMemo(() => {
    const needle = q.toLowerCase();
    return files.filter((f) => f.isText && f.path.toLowerCase().includes(needle)).slice(0, 50);
  }, [files, q]);
  const pick = (path: string) => {
    void openFile(path);
    onClose();
  };
  return (
    <Modal title="Open file" onClose={onClose}>
      <input
        className="input mono"
        placeholder="Type to filter files"
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          setSel(0);
        }}
        onKeyDown={(e) => {
          if (e.key === 'ArrowDown') setSel((s) => Math.min(s + 1, shown.length - 1));
          if (e.key === 'ArrowUp') setSel((s) => Math.max(s - 1, 0));
          if (e.key === 'Enter' && shown[sel]) pick(shown[sel].path);
        }}
        aria-label="Filter files"
      />
      <div className="model-list" role="listbox">
        {shown.map((f, i) => (
          <button key={f.path} role="option" aria-selected={i === sel} className={`model-row ${i === sel ? 'on' : ''}`} style={{ gridTemplateColumns: '1fr' }} onClick={() => pick(f.path)}>
            <span className="mono small">{f.path}</span>
          </button>
        ))}
      </div>
    </Modal>
  );
}
