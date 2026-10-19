import { Kbd, Modal } from './ui';
import { shortcutLabel } from './CommandPalette';

const ROWS: [string, string[]][] = [
  ['New chat', ['Ctrl+Shift+O']],
  ['Command palette', ['Ctrl+K']],
  ['Collapse or expand the sidebar', ['Ctrl+B']],
  ['Settings', ['Ctrl+,']],
  ['Send message', ['Enter']],
  ['New line in the message', ['Shift+Enter']],
  ['Start an Agent Mode mission', ['Ctrl+Enter']],
  ['Close a dialog or menu', ['Esc']],
  ['Show this list', ['Ctrl+/']],
];

export function ShortcutsDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="Keyboard shortcuts" onClose={onClose}>
      <dl className="kb-list">
        {ROWS.map(([what, keys]) => (
          <div key={what} style={{ display: 'contents' }}>
            <dt>{what}</dt>
            <dd>{keys.map((k) => <Kbd key={k}>{shortcutLabel(k)}</Kbd>)}</dd>
          </div>
        ))}
      </dl>
    </Modal>
  );
}
