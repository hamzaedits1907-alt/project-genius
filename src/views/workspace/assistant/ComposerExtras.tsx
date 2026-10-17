import { Check, Clock, Rocket, X } from 'lucide-react';
import { compactNumber } from '../../../lib/agent';
import type { Effort, SlashCommand } from '../../../lib/types';
import type { QueuedMessage } from '../../../store/workspace';
import { AUTOPILOT_HINT, EFFORTS } from './commands';

export function EffortSelect({ value, onChange }: { value: Effort; onChange: (e: Effort) => void }) {
  const cur = EFFORTS.find((e) => e.value === value) ?? EFFORTS[2];
  return (
    <select className="input sm effort-select" aria-label="Effort" title={`${cur.hint} (/effort to change)`} value={value} onChange={(e) => onChange(e.target.value as Effort)}>
      {EFFORTS.map((e) => (
        <option key={e.value} value={e.value} title={e.hint}>
          {e.label}
        </option>
      ))}
    </select>
  );
}

export function AutopilotToggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button type="button" className={`toggle-chip ${on ? 'on' : ''}`} aria-pressed={on} aria-label="Autopilot" title={AUTOPILOT_HINT} onClick={() => onChange(!on)}>
      <Rocket size={12} /> Autopilot
    </button>
  );
}

/** Approximate conversation size against the context budget, with a Compact action when it fills up. */
export function ContextMeter({ used, budget, onCompact, disabled }: { used: number; budget: number; onCompact: () => void; disabled?: boolean }) {
  const ratio = budget > 0 ? Math.min(1, used / budget) : 0;
  const pct = Math.round(ratio * 100);
  const tone = ratio >= 0.85 ? 'bad' : ratio >= 0.7 ? 'warn' : 'ok';
  const r = 7;
  const c = 2 * Math.PI * r;
  const tip = `Conversation size: about ${pct}% of the context budget (${compactNumber(used)} of ${compactNumber(budget)} characters). ${ratio >= 0.7 ? 'Compact it to keep Genius focused and fast.' : 'Older messages are summarized automatically as it grows.'}`;
  return (
    <span className="ctx-meter" title={tip}>
      <svg width="18" height="18" viewBox="0 0 18 18" role="img" aria-label={`Context ${pct}% used`}>
        <circle cx="9" cy="9" r={r} className="ctx-track" />
        <circle cx="9" cy="9" r={r} className={`ctx-fill ${tone}`} strokeDasharray={`${c * ratio} ${c}`} transform="rotate(-90 9 9)" />
      </svg>
      {ratio >= 0.7 && (
        <button type="button" className="btn ghost sm" onClick={onCompact} disabled={disabled} aria-label="Compact conversation" title="Summarize older messages to free up context (/compact)">
          Compact
        </button>
      )}
    </span>
  );
}

export function HelpPanel({ commands, onClose, onPick }: { commands: SlashCommand[]; onClose: () => void; onPick: (c: SlashCommand) => void }) {
  return (
    <div
      className="cmd-help fade-in"
      role="dialog"
      aria-label="Commands"
      onKeyDown={(e) => {
        if (e.key === 'Escape') {
          e.stopPropagation();
          onClose();
        }
      }}
    >
      <div className="row">
        <strong className="grow small">Commands</strong>
        <button className="icon-btn" style={{ width: 22, height: 22 }} aria-label="Close commands" onClick={onClose}>
          <X size={13} />
        </button>
      </div>
      <ul>
        {commands.map((c) => (
          <li key={c.name}>
            <button type="button" onClick={() => onPick(c)}>
              <span className="mono">/{c.name}</span>
              {c.args && <span className="mono faint"> {c.args}</span>}
            </button>
            <span className="faint">{c.description}</span>
          </li>
        ))}
      </ul>
      <div className="faint tiny">
        Type <span className="mono">/</span> for commands and <span className="mono">@</span> to include a project file.
      </div>
    </div>
  );
}

/** A message typed while Genius was working. */
export function QueuedBubble({ q, onRemove }: { q: QueuedMessage; onRemove: () => void }) {
  const status =
    q.status === 'delivered'
      ? 'Delivered — Genius read this'
      : q.status === 'undelivered'
        ? 'Will be sent as a new message when Genius finishes'
        : q.taskId
          ? 'Queued — Genius will read this after its current step'
          : 'Queued — sends when Genius is free';
  return (
    <div className={`msg-wrap user fade-in`}>
      <div className={`msg user queued ${q.status}`} aria-label="Queued message">
        <div className="msg-queued-text">{q.text}</div>
        <div className="msg-queued-status">
          {q.status === 'delivered' ? <Check size={11} /> : <Clock size={11} />} {status}
          {(q.status === 'undelivered' || (q.status === 'queued' && !q.taskId)) && (
            <button className="x" aria-label="Remove queued message" onClick={onRemove}>
              <X size={10} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
