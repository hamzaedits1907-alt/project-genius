// Development-only UI harness: mounts the real <App /> against the test IPC fixtures so the
// conversation UI can be screenshotted in representative states (see docs/TESTING.md).
// Not part of the production build (only index.html is built). The conversations here are
// fixtures for looking at layouts; the product itself never ships invented history.
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from '../App';
import '../styles/global.css';
import '../styles/chat.css';
import '../styles/genius.css';
import { installIpc, settings, type Handler } from '../test/ipc';
import { useApp } from '../store/app';
import { useChat } from '../store/chat';
import type { ChatConversation, MessageRow } from '../lib/types';

const params = new URLSearchParams(location.hash.slice(1));
const scenario = params.get('s') ?? 'welcome';
const theme = (params.get('theme') as 'dark' | 'light') ?? 'dark';

const day = 86_400_000;
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();
const conv = (id: string, title: string, age: number, mode: 'chat' | 'research' = 'chat', snippet = ''): ChatConversation => ({ id, projectId: 'gw', title, createdAt: ago(age), updatedAt: ago(age), messageCount: 2, mode, snippet, origin: null });
const msg = (id: string, role: 'user' | 'assistant', content: string, meta: Record<string, unknown> = {}, kind = role === 'user' ? 'chat' : 'answer'): MessageRow => ({ id, projectId: 'gw', taskId: 't', role, content, createdAt: ago(60_000), conversationId: 'c1', kind, meta });

const picture = 'data:image/svg+xml;utf8,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2b5fff"/><stop offset="1" stop-color="#18c5a0"/></linearGradient></defs><rect width="640" height="400" fill="url(#g)"/><rect x="60" y="250" width="520" height="90" rx="14" fill="#0b1020" opacity="0.55"/><circle cx="320" cy="150" r="64" fill="#ffd34d"/><text x="320" y="310" font-size="30" text-anchor="middle" fill="#fff" font-family="sans-serif">Sample screenshot (fixture)</text></svg>');

const history = [conv('c1', 'Explaining the quadratic formula', 3_600_000), conv('c2', 'Weekly meal plan', 5 * 3_600_000), conv('c3', 'Roblox screenshot', day + 3_600_000), conv('r1', 'Vite vs webpack for a small app', 3 * day, 'research'), conv('c4', 'Cover letter draft', 12 * day), conv('c5', 'CSV cleanup help', 60 * day)];

const markdown = `Here is the idea in three steps.

## The quadratic formula

For $ax^2+bx+c=0$ the solutions are

$$x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}$$

### In code

\`\`\`python
import math

def roots(a, b, c):
    d = b * b - 4 * a * c
    return ((-b + math.sqrt(d)) / (2 * a), (-b - math.sqrt(d)) / (2 * a))
\`\`\`

| Discriminant | Real roots |
|---|---|
| $b^2-4ac>0$ | two |
| $b^2-4ac=0$ | one |
| $b^2-4ac<0$ | none (complex) |

- Check the sign of the discriminant first.
- Then plug the numbers in.`;

const research = `## Summary
Vite starts faster in development because it serves source files as native ES modules [1]; webpack bundles first [2].

## Comparison
| | Vite | webpack |
|---|---|---|
| Dev start | fast [1] | slower [2] |

[1] Why Vite — https://vite.dev/guide/why
[2] webpack concepts — https://webpack.js.org/concepts/`;

const base: Record<string, Handler> = {
  key_status: () => ({ stored: true, hint: 'nvapi-…abcd', backend: 'memory', error: null }),
  list_chat_conversations: () => history,
  chat_workspace: () => ({ id: 'gw', name: 'General workspace', kind: 'chat', description: '', path: '', spec: null, notes: '', status: { stage: 'general' }, createdAt: ago(day), updatedAt: ago(day), lastOpenedAt: null }),
  attachment_preview: () => ({ attachment: {}, preview: '', imageUrl: picture }),
  list_attachments: () => [],
  conversation_messages: () => [],
  active_tasks: () => [],
};

const scenarios: Record<string, { handlers?: Record<string, Handler>; messages?: MessageRow[]; active?: string; route?: 'chat' | 'research'; noKey?: boolean; tasks?: unknown[]; steps?: unknown[] }> = {
  welcome: {},
  firstrun: { noKey: true },
  thread: { active: 'c1', messages: [msg('u1', 'user', 'Explain the quadratic formula and show me code for it.'), msg('a1', 'assistant', markdown, { report: { steps: 0, model: 'openai/gpt-oss-20b', usage: { totalTokens: 1290, promptTokens: 1255, completionTokens: 35, calls: 1 }, timing: { totalMs: 1377 }, modelCalls: [{ firstTokenMs: 261 }] } })] },
  image: {
    active: 'c3',
    messages: [
      msg('u1', 'user', 'what is this?', { attachments: [{ id: 'a1', name: 'roblox.png', kind: 'image' }] }),
      msg('a1', 'assistant', 'This looks like a **screenshot of a Roblox game**: a bright 3D scene with a character in the centre and a dark panel with white text along the bottom.', { report: { steps: 0, model: 'nvidia/nemotron-3.5-super-vl-120b-a12b', reportedModel: 'private/nvidia/nemotron-3.5-super-120b-a12b', vision: { route: 'native', answeredBy: 'nvidia/nemotron-3.5-super-vl-120b-a12b', images: 1, reason: 'the regular chat model cannot read images' }, usage: { totalTokens: 1573, promptTokens: 1498, completionTokens: 75, calls: 1 }, timing: { totalMs: 9094 }, modelCalls: [{ firstTokenMs: 8172 }] } }),
    ],
  },
  web: { active: 'c1', messages: [msg('u1', 'user', 'What changed in the latest TypeScript release?'), msg('a1', 'assistant', 'The most recent release adds **faster project loading** and several editor improvements. I read the release notes below.', { report: { steps: 3, webSearches: 1, webSources: ['https://devblogs.microsoft.com/typescript/announcing-typescript-5-9/', 'https://github.com/microsoft/TypeScript/releases'], usage: { totalTokens: 4100, calls: 2 }, model: 'qwen/qwen3-next-80b-a3b-instruct', timing: { totalMs: 14200 } } })] },
  research: {
    route: 'research',
    active: 'r1',
    messages: [msg('u1', 'user', 'Compare Vite and webpack for a small app.'), msg('a1', 'assistant', research, { validation: { ok: true, facts: 2, assumptions: 0, uncertain: 0, uncited: [], unknownCitations: [], unfetchedUrls: [], cited: [1, 2], repaired: false } })],
    handlers: {
      research_sources: () => [
        { id: 's1', conversationId: 'r1', taskId: 't', n: 1, url: 'https://vite.dev/guide/why', requestedUrl: 'https://vite.dev/guide/why', title: 'Why Vite', status: 200, fetchedAt: ago(90_000), contentHash: 'abcdef0123456789', snippet: 's', usedFor: 'Dev server starts fast', official: true, flags: [] },
        { id: 's2', conversationId: 'r1', taskId: 't', n: 2, url: 'https://webpack.js.org/concepts/', requestedUrl: 'https://webpack.js.org/concepts/', title: 'Concepts | webpack', status: 200, fetchedAt: ago(80_000), contentHash: '0123456789abcdef', snippet: 's', usedFor: 'Bundles first', official: true, flags: [] },
      ],
    },
  },
  running: {
    active: 'c1',
    messages: [msg('u1', 'user', 'Find the current LTS version of Node.js and summarise what is new.')],
    handlers: { active_tasks: () => [{ id: 'tc1', projectId: 'gw', kind: 'chat', title: 'Chat: Find the current LTS version of Node.js', startedAt: ago(14_000) }] },
    steps: [
      { id: 's1', taskId: 'tc1', projectId: 'gw', seq: 1, tool: 'web_search', args: { query: 'Node.js LTS' }, ok: true, summary: 'Searched the web for "Node.js LTS"', result: {} },
      { id: 's2', taskId: 'tc1', projectId: 'gw', seq: 2, tool: 'fetch_url', args: { url: 'https://nodejs.org' }, ok: true, summary: 'Read nodejs.org', result: {} },
    ],
  },
  error: { active: 'c1', messages: [msg('u1', 'user', 'Summarise my notes please.'), msg('e1', 'assistant', 'I stopped because of an error: the model provider is rate limiting requests right now (HTTP 429). Wait a moment and try again, or choose another model in Settings.', {}, 'error')] },
};

const sc = scenarios[scenario] ?? scenarios.welcome;
const none = { stored: false, hint: null, backend: 'memory', error: null };
installIpc({ ...base, get_settings: () => ({ ...settings, theme }), ...(sc.noKey ? { key_status: () => none } : {}), conversation_messages: () => sc.messages ?? [], ...(sc.handlers ?? {}) });
useApp.setState({
  route: { name: sc.route ?? 'chat' },
  settings: { ...settings, theme },
  ...(sc.noKey ? { keyStatus: none } : {}),
});
document.documentElement.dataset.theme = theme;
useChat.setState({ active: { chat: sc.route === 'research' ? null : (sc.active ?? null), research: sc.route === 'research' ? (sc.active ?? null) : null }, workspaceId: 'gw', steps: sc.steps ? { tc1: sc.steps as never } : {} });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
