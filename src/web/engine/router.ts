// Automatic model choice ("Auto") for Genius Web: pick the best known model family among the models the
// user's key actually lists. Mirrors crates/genius-core/src/router.rs (scores are calibrated priors; the
// listing decides what exists, nothing is hard-coded as available).

const VISION = 1;
const CODER = 2;
const REASONING = 4;

interface Family {
  all: string[];
  none?: string[];
  quality: number;
  speed: number;
  tags: number;
}

/** First match wins: specific entries before generic ones. */
const FAMILIES: Family[] = [
  { all: ['claude', 'fable'], quality: 98, speed: 40, tags: VISION | REASONING | CODER },
  { all: ['claude', 'opus'], quality: 95, speed: 45, tags: VISION | REASONING | CODER },
  { all: ['claude', 'sonnet'], quality: 90, speed: 70, tags: VISION | REASONING | CODER },
  { all: ['claude', 'haiku'], quality: 76, speed: 92, tags: VISION | CODER },
  { all: ['gemini', 'flash-lite'], quality: 70, speed: 96, tags: VISION },
  { all: ['gemini', 'flash'], quality: 82, speed: 85, tags: VISION | REASONING },
  { all: ['gemini', 'pro'], quality: 93, speed: 45, tags: VISION | REASONING | CODER },
  { all: ['gpt-5', 'nano'], quality: 70, speed: 96, tags: VISION },
  { all: ['gpt-5', 'mini'], quality: 84, speed: 82, tags: VISION | REASONING },
  { all: ['gpt-5'], none: ['chat'], quality: 95, speed: 50, tags: VISION | REASONING | CODER },
  { all: ['gpt-4.1', 'nano'], quality: 62, speed: 96, tags: VISION },
  { all: ['gpt-4.1', 'mini'], quality: 76, speed: 88, tags: VISION },
  { all: ['gpt-4.1'], quality: 88, speed: 70, tags: VISION | CODER },
  { all: ['gpt-4o', 'mini'], quality: 66, speed: 92, tags: VISION },
  { all: ['gpt-4o'], quality: 80, speed: 75, tags: VISION },
  { all: ['o4-mini'], quality: 82, speed: 60, tags: VISION | REASONING },
  { all: ['o3'], none: ['mini'], quality: 90, speed: 30, tags: VISION | REASONING },
  { all: ['nemotron', 'vl'], quality: 78, speed: 85, tags: VISION | REASONING },
  { all: ['qwen', 'vl'], quality: 82, speed: 55, tags: VISION },
  { all: ['90b-vision'], quality: 76, speed: 45, tags: VISION },
  { all: ['vision'], quality: 64, speed: 70, tags: VISION },
  { all: ['llama-4-maverick'], quality: 80, speed: 70, tags: VISION },
  { all: ['llama-4-scout'], quality: 72, speed: 80, tags: VISION },
  { all: ['nemotron', 'super'], none: ['llama'], quality: 97, speed: 55, tags: REASONING | CODER },
  { all: ['nemotron', 'ultra'], none: ['llama'], quality: 97, speed: 35, tags: REASONING | CODER },
  { all: ['nemotron', 'nano'], none: ['llama'], quality: 78, speed: 92, tags: REASONING },
  { all: ['qwen', 'coder'], quality: 94, speed: 55, tags: CODER },
  { all: ['qwen', '235b'], quality: 89, speed: 50, tags: REASONING },
  { all: ['qwen', 'next'], quality: 85, speed: 85, tags: 0 },
  { all: ['qwen'], quality: 80, speed: 70, tags: 0 },
  { all: ['deepseek-v'], quality: 92, speed: 40, tags: CODER | REASONING },
  { all: ['kimi-k'], quality: 93, speed: 50, tags: CODER },
  { all: ['glm'], quality: 91, speed: 50, tags: CODER | REASONING },
  { all: ['gpt-oss', '120b'], quality: 87, speed: 75, tags: REASONING },
  { all: ['gpt-oss', '20b'], quality: 73, speed: 95, tags: REASONING },
  { all: ['llama-3.3-70b'], quality: 74, speed: 80, tags: 0 },
  { all: ['llama-3.1-8b'], quality: 50, speed: 98, tags: 0 },
];

const NOT_CHAT = ['embed', 'rerank', 'guard', 'safety', 'whisper', 'tts', 'asr', 'reward', '-base', 'dall-e', 'gpt-image', 'moderation', 'realtime', 'audio', 'transcribe', 'imagen', 'veo-', '-live', 'computer-use', 'search-preview', 'robotics', 'aqa', 'clip', 'ocr'];

const family = (id: string) => FAMILIES.find((f) => f.all.every((a) => id.includes(a)) && !(f.none ?? []).some((n) => id.includes(n)));
const routable = (id: string) => !NOT_CHAT.some((x) => id.toLowerCase().includes(x));

export type Role = 'chat' | 'agent' | 'vision' | 'quick';
export type Effort = 'fast' | 'balanced' | 'max';

export function isVisionModel(id: string): boolean {
  const l = id.toLowerCase().replace(/^models\//, '');
  return ((family(l)?.tags ?? 0) & VISION) !== 0 || l.startsWith('claude-') || l.startsWith('gemini-') || ['gpt-4o', 'gpt-4.1', 'gpt-5', 'gpt-4-turbo'].some((p) => l.startsWith(p));
}

function score(f: Family, role: Role, effort: Effort): number | null {
  const q = f.quality;
  const s = f.speed;
  if (role === 'vision') return (f.tags & VISION) === 0 ? null : effort === 'max' ? q : q * 0.7 + s * 0.3;
  if (role === 'agent') return effort === 'max' ? q : effort === 'balanced' ? q * 0.9 + s * 0.1 : q * 0.6 + s * 0.4;
  if (role === 'quick') return effort === 'max' ? q : effort === 'balanced' ? q * 0.5 + s * 0.5 : q * 0.35 + s * 0.65;
  return effort === 'max' ? q * 0.9 + s * 0.1 : effort === 'balanced' ? q * 0.55 + s * 0.45 : q * 0.4 + s * 0.6;
}

/** Best-first candidates among `ids`. */
export function rank(ids: string[], role: Role, effort: Effort, failed: Set<string> = new Set()): string[] {
  const scored: [number, string][] = [];
  for (const id of ids) {
    if (!routable(id) || failed.has(id)) continue;
    const f = family(id.toLowerCase().replace(/^models\//, ''));
    const sc = f ? score(f, role, effort) : null;
    if (sc !== null) scored.push([sc, id]);
  }
  scored.sort((a, b) => b[0] - a[0] || b[1].localeCompare(a[1]));
  const out = scored.map(([, id]) => id);
  if (!out.length && role !== 'vision') return ids.filter((id) => routable(id) && !failed.has(id)).sort().slice(0, 4);
  return out.slice(0, 6);
}
