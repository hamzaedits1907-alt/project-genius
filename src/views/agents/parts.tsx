// Small shared building blocks for the Agents views.
import { useState, type KeyboardEvent, type ReactNode } from 'react';
import { Plus, X } from 'lucide-react';
import { GeniusAvatar } from '../../components/GeniusAvatar';
import { Badge, Button } from '../../components/ui';
import { AGENT_STATUS_LABEL, agentStatusTone, avatarEmoji, isAnimatedStatus, moodForStatus } from '../../lib/agents';
import type { AgentStatus } from '../../lib/types';

/**
 * An agent's avatar. The "spark" preset is Genius's own character whose mood follows the real
 * status; other avatars are emoji with a ring. Nothing moves unless the agent is working,
 * planning or reviewing.
 */
export function AgentAvatar({ avatar, status = 'idle', size = 36, label }: { avatar: string; status?: AgentStatus; size?: number; label?: string }) {
  const animated = isAnimatedStatus(status);
  return (
    <span className={`ag-avatar tone-${agentStatusTone(status)}`} data-animated={animated ? 'true' : 'false'} data-status={status} style={{ width: size, height: size }} role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      {avatar === 'spark' ? <GeniusAvatar size={size - 4} mood={moodForStatus(status)} className={animated ? '' : 'still'} /> : <span className="ag-emoji" style={{ fontSize: Math.round(size * 0.56) }}>{avatarEmoji(avatar)}</span>}
    </span>
  );
}

export function AgentStatusBadge({ status }: { status: AgentStatus }) {
  const animated = isAnimatedStatus(status);
  return (
    <span data-testid="agent-status" data-status={status} data-animated={animated ? 'true' : 'false'}>
      <Badge tone={agentStatusTone(status)}>
        {animated && <span className="spinner" style={{ width: 9, height: 9, borderWidth: 1.5 }} aria-hidden />}
        {AGENT_STATUS_LABEL[status]}
      </Badge>
    </span>
  );
}

/** A titled group of form fields. */
export function Section({ title, hint, children }: { title: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <section className="ag-section">
      <h3>{title}</h3>
      {hint && <p className="small muted" style={{ margin: '0 0 8px' }}>{hint}</p>}
      <div className="col" style={{ gap: 12 }}>{children}</div>
    </section>
  );
}

/** Add/remove list of short strings (specializations, paths, links). */
export function ChipInput({ label, values, onChange, placeholder, error, hint, id }: { label: string; values: string[]; onChange: (v: string[]) => void; placeholder?: string; error?: string; hint?: ReactNode; id: string }) {
  const [text, setText] = useState('');
  const add = () => {
    const t = text.trim();
    if (!t) return;
    if (!values.includes(t)) onChange([...values, t]);
    setText('');
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      add();
    }
  };
  return (
    <div className="field">
      <label htmlFor={id}>{label}</label>
      {values.length > 0 && (
        <ul className="chips" aria-label={label}>
          {values.map((v) => (
            <li key={v} className="chip">
              <span className="ellipsis">{v}</span>
              <button type="button" className="chip-x" aria-label={`Remove ${v}`} onClick={() => onChange(values.filter((x) => x !== v))}>
                <X size={11} />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="row">
        <input id={id} className="input grow" value={text} placeholder={placeholder} onChange={(e) => setText(e.target.value)} onKeyDown={onKey} aria-invalid={error ? true : undefined} />
        <Button type="button" size="sm" icon={<Plus size={13} />} onClick={add} disabled={!text.trim()} aria-label={`Add to ${label}`}>
          Add
        </Button>
      </div>
      {hint && <div className="hint">{hint}</div>}
      {error && <div className="field-error" role="alert">{error}</div>}
    </div>
  );
}
