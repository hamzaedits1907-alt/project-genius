// "Improve my prompt": rewrites a rough idea into a detailed, buildable brief in place, with
// Undo. Reusable by any text box (Home idea box, chat composer): pass the text and a setter.
import { useEffect, useRef, useState } from 'react';
import { Undo2, Wand2 } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { useApp } from '../store/app';

export const ENHANCE_MIN = 3;
export const ENHANCE_MAX = 4000;

export interface EnhanceButtonProps {
  value: string;
  onChange: (next: string) => void;
  /** Disable when the AI is not set up (no key or model). */
  disabled?: boolean;
  /** Explains why the button is disabled. */
  disabledReason?: string;
  /** Called after an improvement was applied (e.g. to focus the text box). */
  onEnhanced?: (brief: string) => void;
  /** Reports the busy state (e.g. to disable the text box or show the avatar working). */
  onBusyChange?: (busy: boolean) => void;
  compact?: boolean;
}

export function EnhanceButton({ value, onChange, disabled, disabledReason, onEnhanced, onBusyChange, compact }: EnhanceButtonProps) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState(false);
  // The text before the last improvement, and the improvement itself (Undo is offered while
  // the box still shows exactly that improvement).
  const [undo, setUndo] = useState<{ before: string; after: string } | null>(null);
  const latest = useRef(value);
  latest.current = value;
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const len = value.trim().length;
  const tooShort = len < ENHANCE_MIN;
  const tooLong = len > ENHANCE_MAX;
  const canUndo = !!undo && undo.after === value && !busy;

  const setBusyBoth = (b: boolean) => {
    setBusy(b);
    onBusyChange?.(b);
  };

  const run = async () => {
    const before = value;
    setBusyBoth(true);
    try {
      const brief = await api.enhancePrompt(before);
      if (!mounted.current) return;
      if (latest.current !== before) {
        toast('info', 'You kept typing, so your text was left as it is. Press Improve again to rewrite it.');
        return;
      }
      onChange(brief);
      setUndo({ before, after: brief });
      onEnhanced?.(brief);
    } catch (e) {
      if (mounted.current) toast('bad', `Could not improve the prompt: ${errorMessage(e)}`);
    } finally {
      if (mounted.current) setBusyBoth(false);
    }
  };

  const title = disabled
    ? (disabledReason ?? 'Set up the AI first')
    : tooShort
      ? 'Type a few words about your idea first'
      : tooLong
        ? `Too long to improve (limit ${ENHANCE_MAX} characters)`
        : 'Rewrite your idea into a detailed brief: goal, audience, pages, features, data and style';

  return (
    <span className="enhance">
      <button
        type="button"
        className={`btn ghost sm enhance-btn ${busy ? 'busy' : ''}`}
        onClick={() => void run()}
        disabled={disabled || busy || tooShort || tooLong}
        aria-busy={busy || undefined}
        title={title}
      >
        {busy ? <span className="spinner" aria-hidden /> : <Wand2 size={13} />}
        {busy ? 'Improving…' : compact ? 'Improve' : 'Improve my prompt'}
      </button>
      {canUndo && (
        <button
          type="button"
          className="btn ghost sm enhance-undo fade-in"
          onClick={() => {
            onChange(undo.before);
            setUndo(null);
          }}
          title="Bring back what you wrote"
        >
          <Undo2 size={13} /> Undo
        </button>
      )}
    </span>
  );
}

export default EnhanceButton;
