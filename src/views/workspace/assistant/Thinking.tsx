import { useEffect, useRef, useState } from 'react';
import { Brain, ChevronDown, ChevronRight } from 'lucide-react';
import { duration } from '../../../lib/agent';
import { useWorkspace } from '../../../store/workspace';

/** Reasoning counts as "still thinking" while deltas keep arriving. */
const QUIET_MS = 2500;

/**
 * The model's streamed reasoning for one task (from `genius://ai-thinking`, kept in memory only).
 * Collapsed it shows the last few lines, faded; expanded it scrolls with the stream.
 */
export function Thinking({ taskId, live }: { taskId: string; live: boolean }) {
  const t = useWorkspace((s) => s.thinking[taskId]);
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(() => Date.now());
  const body = useRef<HTMLPreElement>(null);
  const stick = useRef(true);

  useEffect(() => {
    if (!live) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [live]);

  useEffect(() => {
    const el = body.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [t?.text, open]);

  if (!t || !t.text.trim()) return null;
  const active = live && Math.max(now, t.lastAt) - t.lastAt < QUIET_MS;
  const elapsed = active ? Math.max(now, t.lastAt) - t.startedAt : t.lastAt - t.startedAt;
  const label = active ? `Thinking… ${Math.max(0, Math.round(elapsed / 1000))}s` : `Thought for ${duration(elapsed)}`;
  const preview = t.text
    .split('\n')
    .map((l) => l.trim())
    .filter(Boolean)
    .slice(-3);
  return (
    <div className={`thinking ${active ? 'active' : ''}`}>
      <button className="thinking-head" onClick={() => setOpen(!open)} aria-expanded={open} title={open ? 'Hide reasoning' : 'Show the model’s reasoning'}>
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        <Brain size={12} />
        <span>{label}</span>
      </button>
      {open ? (
        <pre
          className="thinking-body"
          ref={body}
          aria-label="Reasoning"
          onScroll={(e) => {
            const el = e.currentTarget;
            stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
          }}
        >
          {t.text}
        </pre>
      ) : (
        live && (
          <div className="thinking-preview" aria-hidden>
            {preview.map((l, i) => (
              <div key={i}>{l}</div>
            ))}
          </div>
        )
      )}
    </div>
  );
}
