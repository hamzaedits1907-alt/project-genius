import { useEffect, useRef, type ReactNode } from 'react';

export interface SuggestItem {
  key: string;
  label: ReactNode;
  detail?: ReactNode;
  icon?: ReactNode;
}

/**
 * Keyboard-driven suggestion list shown above the composer (slash commands, @-mentions).
 * The textarea keeps focus; it forwards ↑/↓/Enter/Tab/Esc and `active` follows along.
 */
export function SuggestPopup({ id, title, items, active, onPick, onHover, empty }: { id: string; title: string; items: SuggestItem[]; active: number; onPick: (i: number) => void; onHover: (i: number) => void; empty?: string }) {
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-i="${active}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  return (
    <div className="suggest fade-in" role="listbox" id={id} aria-label={title} ref={list}>
      <div className="suggest-title">{title}</div>
      {items.length === 0 && <div className="suggest-empty">{empty ?? 'No matches'}</div>}
      {items.map((it, i) => (
        <div
          key={it.key}
          id={`${id}-${i}`}
          data-i={i}
          role="option"
          aria-selected={i === active}
          className={`suggest-item ${i === active ? 'on' : ''}`}
          // mousedown, not click: keep focus in the textarea
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(i);
          }}
          onMouseEnter={() => onHover(i)}
        >
          {it.icon && <span className="suggest-icon">{it.icon}</span>}
          <span className="suggest-label">{it.label}</span>
          {it.detail && <span className="suggest-detail">{it.detail}</span>}
        </div>
      ))}
      <div className="suggest-foot">
        <kbd className="kbd">↑</kbd>
        <kbd className="kbd">↓</kbd> to choose · <kbd className="kbd">Enter</kbd> or <kbd className="kbd">Tab</kbd> to pick · <kbd className="kbd">Esc</kbd> to close
      </div>
    </div>
  );
}
