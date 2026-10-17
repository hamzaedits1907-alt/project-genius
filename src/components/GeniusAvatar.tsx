import { useId } from 'react';

/** What Genius is doing right now; drives the character's expression and motion. */
export type Mood = 'idle' | 'listening' | 'thinking' | 'working' | 'happy' | 'sad';

/**
 * Genius's character: a small glowing spark-spirit with a floating star.
 * Pure SVG + CSS (no assets), so it scales from a 20px chat avatar to a hero illustration.
 * Animations respect `prefers-reduced-motion` (see global.css).
 */
export function GeniusAvatar({ size = 32, mood = 'idle', className = '', title }: { size?: number; mood?: Mood; className?: string; title?: string }) {
  const id = useId().replace(/:/g, '');
  const body = `gb-${id}`;
  const glow = `gg-${id}`;
  const shine = `gs-${id}`;
  return (
    <span className={`genius-avatar mood-${mood} ${className}`} style={{ width: size, height: size }} role={title ? 'img' : undefined} aria-label={title} aria-hidden={title ? undefined : true}>
      <svg viewBox="0 0 64 64" width={size} height={size}>
        <defs>
          <radialGradient id={body} cx="38%" cy="30%" r="75%">
            <stop offset="0%" stopColor="#c4b5ff" />
            <stop offset="45%" stopColor="#8b6cff" />
            <stop offset="100%" stopColor="#5b3df0" />
          </radialGradient>
          <radialGradient id={glow} cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#a78bfa" stopOpacity="0.55" />
            <stop offset="100%" stopColor="#a78bfa" stopOpacity="0" />
          </radialGradient>
          <linearGradient id={shine} x1="0" y1="0" x2="1" y2="1">
            <stop offset="0%" stopColor="#fff6c8" />
            <stop offset="100%" stopColor="#ffb547" />
          </linearGradient>
        </defs>
        <circle className="ga-halo" cx="32" cy="36" r="30" fill={`url(#${glow})`} />
        <g className="ga-float">
          {/* the spark above the head: Genius's "idea" */}
          <g className="ga-spark">
            <path d="M44 4 L46.2 9.8 L52 12 L46.2 14.2 L44 20 L41.8 14.2 L36 12 L41.8 9.8 Z" fill={`url(#${shine})`} />
          </g>
          <g className="ga-body">
            <path d="M32 14c11.6 0 20 8.6 20 20.5S43.6 56 32 56 12 46.4 12 34.5 20.4 14 32 14Z" fill={`url(#${body})`} />
            <ellipse cx="25" cy="23" rx="6" ry="3.2" fill="#fff" opacity="0.28" transform="rotate(-20 25 23)" />
            <ellipse className="ga-cheek" cx="21" cy="40" rx="3.2" ry="2" fill="#ff8fc7" opacity="0.55" />
            <ellipse className="ga-cheek" cx="43" cy="40" rx="3.2" ry="2" fill="#ff8fc7" opacity="0.55" />
            <g className="ga-eyes">
              <g className="ga-eyes-open">
                <ellipse cx="25.5" cy="34" rx="3.1" ry="4.3" fill="#1b1340" />
                <ellipse cx="38.5" cy="34" rx="3.1" ry="4.3" fill="#1b1340" />
                <circle cx="26.6" cy="32.4" r="1.1" fill="#fff" />
                <circle cx="39.6" cy="32.4" r="1.1" fill="#fff" />
              </g>
              <g className="ga-eyes-happy" fill="none" stroke="#1b1340" strokeWidth="2.4" strokeLinecap="round">
                <path d="M22 35 Q25.5 30.5 29 35" />
                <path d="M35 35 Q38.5 30.5 42 35" />
              </g>
            </g>
            <path className="ga-mouth" d="M28.5 43.5 Q32 46.5 35.5 43.5" fill="none" stroke="#1b1340" strokeWidth="2" strokeLinecap="round" />
            <path className="ga-mouth-sad" d="M28.5 45.5 Q32 42.5 35.5 45.5" fill="none" stroke="#1b1340" strokeWidth="2" strokeLinecap="round" />
          </g>
          <g className="ga-dots" fill="#c4b5ff">
            <circle cx="52" cy="30" r="2" />
            <circle cx="57" cy="24" r="1.5" />
            <circle cx="60" cy="17" r="1.1" />
          </g>
        </g>
      </svg>
    </span>
  );
}

/** Map live workspace state to a mood. */
export function moodFor({ busy, failed, typing, justFinished }: { busy?: 'thinking' | 'working' | null; failed?: boolean; typing?: boolean; justFinished?: boolean }): Mood {
  if (busy) return busy;
  if (failed) return 'sad';
  if (justFinished) return 'happy';
  if (typing) return 'listening';
  return 'idle';
}
