// Small inline-SVG illustrations for blueprint cards: a stylised glimpse of each app. Colours
// come from CSS (theme surfaces + the blueprint's two accents set as --bp-a / --bp-b).
import { useId, type ReactNode } from 'react';

type Draw = (g: string) => ReactNode;

// Keyed by geometry so bars can be produced from arrays.
const bar = (x: number, y: number, w: number, h = 4, cls = 'l') => <rect key={`${x}:${y}:${w}`} x={x} y={y} width={w} height={h} rx={h / 2} className={cls} />;

const ART: Record<string, Draw> = {
  'saas-landing': (g) => (
    <>
      {bar(14, 12, 18, 5, 'a')}
      {bar(108, 12, 12)}
      {bar(124, 12, 22, 5, 'a')}
      {bar(34, 30, 92, 8, 'h')}
      {bar(48, 43, 64)}
      <rect x="56" y="54" width="22" height="9" rx="4.5" fill={`url(#${g})`} />
      <rect x="82" y="54" width="22" height="9" rx="4.5" className="o" />
      <rect x="30" y="70" width="100" height="26" rx="5" className="s" />
      {bar(38, 77, 30, 3)}
      {bar(38, 84, 50, 3)}
    </>
  ),
  portfolio: (g) => (
    <>
      {bar(14, 12, 50, 8, 'h')}
      {bar(14, 24, 34)}
      <rect x="14" y="36" width="42" height="26" rx="4" fill={`url(#${g})`} />
      <rect x="60" y="36" width="42" height="26" rx="4" className="s" />
      <rect x="106" y="36" width="40" height="26" rx="4" className="b" opacity="0.7" />
      <rect x="14" y="66" width="42" height="24" rx="4" className="s" />
      <rect x="60" y="66" width="42" height="24" rx="4" className="a" opacity="0.6" />
      <rect x="106" y="66" width="40" height="24" rx="4" className="s" />
    </>
  ),
  'online-store': (g) => (
    <>
      {bar(14, 12, 24, 5, 'a')}
      <circle cx="140" cy="14" r="6" className="o" />
      <circle cx="145" cy="9" r="3.5" className="b" />
      {[14, 62, 110].map((x, i) => (
        <g key={x}>
          <rect x={x} y="28" width="38" height="60" rx="5" className="s" />
          <rect x={x + 4} y="32" width="30" height="28" rx="3" fill={i === 1 ? `url(#${g})` : undefined} className={i === 1 ? '' : i ? 'b' : 'a'} opacity={i === 1 ? 1 : 0.55} />
          {bar(x + 4, 65, 22, 3)}
          {bar(x + 4, 72, 12, 4, 'h')}
          <rect x={x + 24} y="77" width="10" height="7" rx="3.5" className="a" />
        </g>
      ))}
    </>
  ),
  'blog-magazine': (g) => (
    <>
      <rect x="14" y="12" width="70" height="44" rx="4" fill={`url(#${g})`} />
      {bar(14, 62, 64, 6, 'h')}
      {bar(14, 72, 54, 3)}
      {bar(14, 79, 60, 3)}
      {[12, 40, 68].map((y) => (
        <g key={y}>
          <rect x="94" y={y} width="20" height="20" rx="3" className="s" />
          {bar(118, y + 3, 28, 4, 'h')}
          {bar(118, y + 11, 22, 3)}
        </g>
      ))}
    </>
  ),
  restaurant: (g) => (
    <>
      <circle cx="44" cy="50" r="30" className="s" />
      <circle cx="44" cy="50" r="21" fill={`url(#${g})`} />
      <circle cx="38" cy="44" r="5" className="o" opacity="0.6" />
      <rect x="8" y="26" width="3" height="48" rx="1.5" className="l" />
      <rect x="77" y="26" width="3" height="48" rx="1.5" className="l" />
      {[18, 36, 54, 72].map((y, i) => (
        <g key={y}>
          {bar(94, y, 34 - (i % 2) * 8, 4, 'h')}
          {bar(136, y, 10, 4, 'a')}
          {bar(94, y + 7, 46, 3)}
        </g>
      ))}
    </>
  ),
  'event-conference': (g) => (
    <>
      <rect x="14" y="12" width="64" height="72" rx="6" fill={`url(#${g})`} />
      {bar(22, 22, 30, 6, 'o')}
      {bar(22, 32, 44, 3, 'o')}
      <circle cx="46" cy="62" r="13" className="o" opacity="0.3" />
      {[16, 36, 56, 76].map((y, i) => (
        <g key={y}>
          {bar(88, y, 14, 4, i === 1 ? 'a' : 'b')}
          <rect x="106" y={y - 3} width="40" height="11" rx="3" className="s" />
          {bar(110, y, 24, 3)}
        </g>
      ))}
    </>
  ),
  'productivity-dashboard': (g) => (
    <>
      <rect x="12" y="10" width="20" height="78" rx="4" className="s" />
      {[18, 28, 38, 48].map((y) => bar(16, y, 12, 3))}
      <rect x="38" y="10" width="62" height="40" rx="4" className="s" />
      <polyline points="44,42 54,34 64,38 74,24 84,28 94,18" className="line" />
      <rect x="104" y="10" width="44" height="40" rx="4" className="s" />
      <circle cx="126" cy="30" r="12" className="ring" />
      <circle cx="126" cy="30" r="12" className="ring-a" strokeDasharray="50 100" transform="rotate(-90 126 30)" />
      <rect x="38" y="54" width="110" height="34" rx="4" className="s" />
      {[62, 70, 78].map((y, i) => (
        <g key={y}>
          <rect x="44" y={y - 2} width="5" height="5" rx="1.5" className={i === 0 ? 'a' : 'l'} />
          {bar(53, y - 1, 50 - i * 10, 3)}
        </g>
      ))}
      <rect x="120" y="60" width="22" height="22" rx="3" fill={`url(#${g})`} opacity="0.85" />
    </>
  ),
  kanban: () => (
    <>
      {[12, 60, 108].map((x, col) => (
        <g key={x}>
          <rect x={x} y="10" width="40" height="78" rx="5" className="s" />
          {bar(x + 5, 16, 18, 4, 'h')}
          {Array.from({ length: 3 - col + (col === 2 ? 1 : 0) }).map((_, i) => (
            <g key={i}>
              <rect x={x + 4} y={26 + i * 19} width="32" height="15" rx="3" className="o" />
              {bar(x + 8, 30 + i * 19, 10, 3, (i + col) % 2 ? 'b' : 'a')}
              {bar(x + 8, 36 + i * 19, 20, 2.5)}
            </g>
          ))}
        </g>
      ))}
    </>
  ),
  'finance-tracker': (g) => (
    <>
      <circle cx="40" cy="48" r="24" className="ring thick" />
      <circle cx="40" cy="48" r="24" className="ring-a thick" strokeDasharray="70 151" transform="rotate(-90 40 48)" />
      <circle cx="40" cy="48" r="24" className="ring-b thick" strokeDasharray="40 151" strokeDashoffset="-74" transform="rotate(-90 40 48)" />
      {bar(30, 46, 20, 5, 'h')}
      {[0, 1, 2, 3, 4, 5].map((i) => {
        const h = [24, 36, 30, 48, 40, 56][i];
        return <rect key={i} x={80 + i * 11} y={84 - h} width="7" height={h} rx="2" fill={i === 5 ? `url(#${g})` : undefined} className={i === 5 ? '' : 'l'} />;
      })}
      {bar(80, 12, 40, 6, 'h')}
    </>
  ),
  'habit-fitness': () => (
    <>
      {[26, 19, 12].map((r, i) => (
        <g key={r}>
          <circle cx="40" cy="48" r={r} className="ring" />
          <circle cx="40" cy="48" r={r} className={i === 1 ? 'ring-b' : 'ring-a'} strokeDasharray={`${r * 6.28 * [0.8, 0.6, 0.9][i]} 200`} transform="rotate(-90 40 48)" />
        </g>
      ))}
      {Array.from({ length: 28 }).map((_, i) => (
        <rect key={i} x={82 + (i % 7) * 9.5} y={20 + Math.floor(i / 7) * 16} width="7" height="12" rx="2" className={[1, 2, 4, 5, 8, 9, 10, 12, 15, 16, 17, 18, 22, 23, 24, 25, 26].includes(i) ? (i % 3 ? 'a' : 'b') : 'l'} opacity={i % 4 === 0 ? 0.6 : 1} />
      ))}
    </>
  ),
  'ai-chat-ui': (g) => (
    <>
      <rect x="10" y="10" width="34" height="78" rx="4" className="s" />
      {[18, 27, 36, 45].map((y, i) => bar(15, y, i === 0 ? 24 : 20, 4, i === 0 ? 'a' : 'l'))}
      <rect x="86" y="14" width="58" height="14" rx="7" fill={`url(#${g})`} />
      <circle cx="56" cy="40" r="5" className="b" />
      <rect x="64" y="34" width="70" height="24" rx="6" className="s" />
      {bar(70, 40, 52, 3)}
      {bar(70, 47, 40, 3)}
      <rect x="52" y="70" width="94" height="16" rx="8" className="o" />
      <circle cx="138" cy="78" r="5" className="a" />
    </>
  ),
  booking: (g) => (
    <>
      <rect x="12" y="10" width="72" height="78" rx="5" className="s" />
      {bar(18, 16, 30, 5, 'h')}
      {Array.from({ length: 20 }).map((_, i) => (
        <rect key={i} x={18 + (i % 5) * 13} y={28 + Math.floor(i / 5) * 14} width="9" height="9" rx="2.5" fill={i === 12 ? `url(#${g})` : undefined} className={i === 12 ? '' : [3, 7, 16].includes(i) ? 'b' : 'l'} opacity={i === 12 ? 1 : [3, 7, 16].includes(i) ? 0.5 : 1} />
      ))}
      {[16, 32, 48, 64].map((y, i) => (
        <rect key={y} x="92" y={y} width="54" height="11" rx="5.5" className={i === 1 ? 'a' : 'o'} />
      ))}
    </>
  ),
  'game-2048': () => (
    <>
      <rect x="44" y="6" width="72" height="84" rx="7" className="s" />
      {Array.from({ length: 16 }).map((_, i) => {
        const v = [0, 2, 0, 4, 0, 8, 2, 0, 16, 64, 4, 0, 128, 32, 2048, 8][i];
        const cls = v === 0 ? 'l' : v >= 128 ? 'b' : v >= 16 ? 'a' : 'o';
        return <rect key={i} x={50 + (i % 4) * 16.5} y={12 + Math.floor(i / 4) * 19.5} width="13.5" height="16" rx="2.5" className={cls} opacity={v === 0 ? 0.6 : v >= 16 && v < 128 ? 0.8 : 1} />;
      })}
    </>
  ),
  'game-snake': (g) => (
    <>
      <rect x="14" y="8" width="132" height="80" rx="6" className="s" />
      <polyline points="34,72 34,52 70,52 70,30 104,30 104,46" className="snake" stroke={`url(#${g})`} />
      <circle cx="104" cy="48" r="4.5" className="b" />
      <circle cx="126" cy="64" r="4" className="a" />
      {bar(20, 14, 20, 4, 'h')}
    </>
  ),
  'game-memory': (g) => (
    <>
      {Array.from({ length: 8 }).map((_, i) => {
        const x = 22 + (i % 4) * 30;
        const y = 12 + Math.floor(i / 4) * 40;
        const open = i === 1 || i === 6;
        return (
          <g key={i}>
            <rect x={x} y={y} width="24" height="34" rx="4" fill={open ? undefined : `url(#${g})`} className={open ? 'o' : ''} opacity={open ? 1 : 0.9} />
            {open ? <circle cx={x + 12} cy={y + 17} r="6" className="b" /> : <rect x={x + 8} y={y + 13} width="8" height="8" rx="2" className="o" opacity="0.35" />}
          </g>
        );
      })}
    </>
  ),
  'docs-knowledge-base': () => (
    <>
      <rect x="10" y="10" width="34" height="78" rx="4" className="s" />
      {[18, 26, 34, 42, 50, 58].map((y, i) => bar(i % 3 ? 20 : 15, y, i % 3 ? 18 : 22, 3, i === 2 ? 'a' : 'l'))}
      <rect x="54" y="10" width="92" height="12" rx="6" className="o" />
      <circle cx="62" cy="16" r="2.5" className="a" />
      {bar(54, 30, 56, 6, 'h')}
      {bar(54, 42, 88, 3)}
      {bar(54, 49, 80, 3)}
      <rect x="54" y="58" width="92" height="22" rx="4" className="s" />
      {bar(60, 64, 40, 3, 'b')}
      {bar(60, 71, 60, 3)}
    </>
  ),
  'invoice-generator': (g) => (
    <>
      <rect x="40" y="6" width="80" height="88" rx="4" className="o" />
      <rect x="48" y="14" width="18" height="8" rx="2" fill={`url(#${g})`} />
      {bar(88, 15, 24, 4, 'h')}
      {[34, 44, 54].map((y) => (
        <g key={y}>
          {bar(48, y, 36, 3)}
          {bar(98, y, 14, 3)}
        </g>
      ))}
      <rect x="48" y="64" width="64" height="1" className="l" />
      {bar(80, 72, 32, 6, 'a')}
    </>
  ),
  pomodoro: (g) => (
    <>
      <circle cx="80" cy="48" r="34" className="ring thick" />
      <circle cx="80" cy="48" r="34" className="ring-g thick" stroke={`url(#${g})`} strokeDasharray="150 214" transform="rotate(-90 80 48)" />
      {bar(62, 42, 36, 10, 'h')}
      {bar(70, 58, 20, 3)}
      <circle cx="132" cy="22" r="5" className="b" opacity="0.7" />
      <circle cx="28" cy="74" r="4" className="a" opacity="0.6" />
    </>
  ),
};

export function BlueprintArt({ id, colors }: { id: string; colors: [string, string] }) {
  const g = `bpg-${useId().replace(/[^a-zA-Z0-9-]/g, '')}`;
  const draw = ART[id] ?? ART['saas-landing'];
  return (
    <svg className="bp-art" viewBox="0 0 160 96" aria-hidden focusable="false" style={{ ['--bp-a' as string]: colors[0], ['--bp-b' as string]: colors[1] }}>
      <defs>
        <linearGradient id={g} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" className="stop-a" />
          <stop offset="1" className="stop-b" />
        </linearGradient>
      </defs>
      {draw(g)}
    </svg>
  );
}

export function hasBlueprintArt(id: string): boolean {
  return id in ART;
}
