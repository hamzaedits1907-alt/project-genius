// Genius conversation UI against the mocked backend: sidebar navigation and history, the welcome
// and thread views, attachments and pictures, failure and retry, math typesetting, first-run setup.
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { calls, clearMocks, installIpc, settings, type Handler } from './ipc';
import ChatHome from '../views/chat/ChatHome';
import { Sidebar } from '../components/Sidebar';
import { Markdown } from '../components/Markdown';
import { useApp } from '../store/app';
import { useChat } from '../store/chat';
import { answerDetails, groupByRecency, webSourcesOf } from '../lib/chat';
import type { ChatConversation, MessageRow } from '../lib/types';

const flush = (ms = 40) => act(() => new Promise((r) => setTimeout(r, ms)));
const conv = (id: string, title: string, mode: 'chat' | 'research' = 'chat', snippet = '', updatedAt = '2026-10-01T00:00:00Z'): ChatConversation => ({ id, projectId: 'gw', title, createdAt: updatedAt, updatedAt, messageCount: 2, mode, snippet, origin: null });
const msg = (id: string, role: 'user' | 'assistant', content: string, meta: Record<string, unknown> = {}, kind = role === 'user' ? 'chat' : 'answer'): MessageRow => ({ id, projectId: 'gw', taskId: 't', role, content, createdAt: '2026-10-01T00:00:00Z', conversationId: 'c1', kind, meta });
const KEY = { stored: true, hint: 'nvapi-…abcd', backend: 'memory', error: null };

function setup(over: Record<string, Handler> = {}) {
  installIpc({ key_status: () => KEY, ...over });
  useChat.setState({ workspaceId: null, conversations: { chat: [], research: [] }, all: [], active: { chat: null, research: null }, query: '', messages: [], sources: [], tasks: [], steps: {}, queue: [], pending: null });
  useApp.setState({ settings: { ...settings }, keyStatus: KEY, route: { name: 'chat' }, sidebarCollapsed: false, approvals: [], toasts: [] });
}

beforeEach(() => {
  window.confirm = () => true;
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('sidebar navigation', () => {
  it('switches between Chat, Research, Code and Agents, keeps state, and collapses to an icon rail', async () => {
    setup();
    render(<Sidebar />);
    await flush();
    useChat.setState({ messages: [msg('m', 'user', 'keep me')] });
    const nav = screen.getByRole('navigation', { name: 'Workspaces' });
    expect(within(nav).getAllByRole('button').map((b) => b.textContent)).toEqual(['Chat', 'Research', 'Code', 'Agents', 'Projects']);
    expect(within(nav).getByRole('button', { name: 'Chat' }).getAttribute('aria-current')).toBe('page');
    fireEvent.click(within(nav).getByRole('button', { name: 'Research' }));
    expect(useApp.getState().route.name).toBe('research');
    fireEvent.click(within(nav).getByRole('button', { name: 'Code' }));
    expect(useApp.getState().route.name).toBe('home');
    fireEvent.click(within(nav).getByRole('button', { name: 'Agents' }));
    expect(useApp.getState().route.name).toBe('agents');
    expect(useChat.getState().messages[0].content).toBe('keep me');
    fireEvent.click(screen.getByRole('button', { name: 'Collapse sidebar' }));
    expect(useApp.getState().sidebarCollapsed).toBe(true);
    expect(screen.queryByRole('navigation', { name: 'Workspaces' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Chat' }));
    expect(useApp.getState().route.name).toBe('chat');
    fireEvent.click(screen.getAllByRole('button', { name: 'Expand sidebar' })[0]);
    expect(useApp.getState().sidebarCollapsed).toBe(false);
  });

  it('New chat starts a blank conversation without creating anything on the backend', async () => {
    setup({ list_chat_conversations: () => [conv('c1', 'Zoology')] });
    render(<Sidebar />);
    await flush();
    useChat.setState({ active: { chat: 'c1', research: null }, messages: [msg('u1', 'user', 'Tell me about alpacas')] });
    fireEvent.click(screen.getByRole('button', { name: 'New chat' }));
    expect(useChat.getState().active.chat).toBeNull();
    expect(useChat.getState().messages).toEqual([]);
    expect(calls.some((c) => c.cmd === 'create_chat_conversation')).toBe(false);
  });
});

describe('history', () => {
  it('groups conversations by recency', () => {
    const now = new Date(2026, 9, 18, 15, 0, 0);
    const at = (d: number) => new Date(now.getFullYear(), now.getMonth(), now.getDate() - d, 10).toISOString();
    const groups = groupByRecency([conv('a', 'A', 'chat', '', at(0)), conv('b', 'B', 'chat', '', at(1)), conv('c', 'C', 'chat', '', at(4)), conv('d', 'D', 'chat', '', at(20)), conv('e', 'E', 'chat', '', at(90))], now);
    expect(groups.map((g) => [g.label, g.items.map((i) => i.id)])).toEqual([
      ['Today', ['a']],
      ['Yesterday', ['b']],
      ['Previous 7 days', ['c']],
      ['Previous 30 days', ['d']],
      ['Older', ['e']],
    ]);
  });

  const list = [conv('c1', 'Zoology', 'chat', 'Alpacas are camelids'), conv('c2', 'Weekly plan'), conv('r1', 'Hooks research', 'research')];
  const handlers = (): Record<string, Handler> => ({
    list_chat_conversations: (_c, a) => (a.query ? list.filter((c) => c.title.toLowerCase().includes(String(a.query).toLowerCase())) : list),
    conversation_messages: (_c, a) => (a.conversationId === 'c1' ? [msg('u1', 'user', 'Tell me about alpacas'), msg('a1', 'assistant', 'Alpacas are camelids.')] : []),
    rename_conversation: () => null,
    delete_conversation: () => null,
  });

  it('lists, opens, searches, renames and deletes conversations from the sidebar', async () => {
    setup(handlers());
    render(
      <>
        <Sidebar />
        <ChatHome />
      </>,
    );
    await flush();
    expect(screen.getByText('Zoology')).toBeTruthy();
    expect(screen.getByText('Weekly plan')).toBeTruthy();
    expect(screen.getByText('Hooks research')).toBeTruthy(); // Research conversations share the history
    expect(screen.getByRole('heading', { name: 'How can I help you today?' })).toBeTruthy(); // starts blank, not inside the newest conversation
    await act(async () => void fireEvent.click(screen.getByText('Zoology')));
    await flush();
    expect(screen.getByText('Alpacas are camelids.')).toBeTruthy();
    await act(async () => void fireEvent.change(screen.getByLabelText('Search conversations'), { target: { value: 'week' } }));
    await flush();
    expect(calls.some((c) => c.cmd === 'list_chat_conversations' && c.args.query === 'week')).toBe(true);
    expect(screen.queryByText('Zoology')).toBeNull();
    fireEvent.click(screen.getByLabelText('Rename Weekly plan'));
    const input = screen.getByLabelText('Conversation title');
    await act(async () => void fireEvent.change(input, { target: { value: 'Plan B' } }));
    await act(async () => void fireEvent.keyDown(input, { key: 'Enter' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'rename_conversation')?.args).toEqual({ conversationId: 'c2', title: 'Plan B' });
    fireEvent.click(screen.getByLabelText('Delete Weekly plan'));
    await flush();
    expect(calls.find((c) => c.cmd === 'delete_conversation')?.args).toEqual({ conversationId: 'c2' });
  });

  it('opens a Research conversation in the Research view', async () => {
    setup(handlers());
    render(<Sidebar />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByText('Hooks research')));
    expect(useApp.getState().route.name).toBe('research');
    expect(useChat.getState().active.research).toBe('r1');
  });
});

describe('Chat view', () => {
  const handlers = (): Record<string, Handler> => ({
    list_chat_conversations: () => [conv('c1', 'Zoology', 'chat', 'Alpacas are camelids')],
    conversation_messages: () => [msg('u1', 'user', 'Tell me about alpacas'), msg('a1', 'assistant', 'Alpacas are camelids.')],
  });

  it('shows the welcome screen with real actions and no invented history', async () => {
    setup();
    render(<ChatHome />);
    await flush();
    expect(screen.getByRole('heading', { name: 'How can I help you today?' })).toBeTruthy();
    expect(screen.getByLabelText('Message')).toBeTruthy();
    expect(screen.getByRole('button', { name: /Analyse an image or file/ })).toBeTruthy();
    expect(screen.queryByText(/Alpacas/)).toBeNull();
    expect(screen.queryByText('Connect Genius to its AI brain')).toBeNull();
  });

  it('first run: asks for the API key and disables sending until it is saved', async () => {
    const none = { stored: false, hint: null, backend: 'memory', error: null };
    setup({ key_status: () => none });
    useApp.setState({ keyStatus: none });
    render(<ChatHome />);
    await flush();
    expect(screen.getByText('Connect Genius to its AI brain')).toBeTruthy();
    await act(async () => void fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'Hello' } }));
    expect((screen.getByRole('button', { name: 'Send' }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('sends through the chat entry point (not the code agent) and refuses an image it cannot read', async () => {
    setup(handlers());
    useChat.setState({ active: { chat: 'c1', research: null } });
    render(<ChatHome />);
    await flush();
    const box = screen.getByLabelText('Message') as HTMLTextAreaElement;
    await act(async () => void fireEvent.change(box, { target: { value: 'What is an alpaca?' } }));
    await act(async () => void fireEvent.keyDown(box, { key: 'Enter' }));
    await flush();
    const call = calls.find((c) => c.cmd === 'start_chat');
    expect(call?.args.conversationId).toBe('c1');
    expect((call?.args.request as { message: string }).message).toBe('What is an alpaca?');
    expect(calls.some((c) => c.cmd === 'start_agent')).toBe(false);
    const file = new File([new Uint8Array([1, 2, 3])], 'shot.png', { type: 'image/png' });
    await act(async () => void fireEvent.change(screen.getByLabelText('Choose files'), { target: { files: [file] } }));
    await flush();
    expect(calls.some((c) => c.cmd === 'import_attachment_data')).toBe(false);
    expect(useApp.getState().toasts.some((t) => t.text.includes('none of your available models can read images'))).toBe(true);
  });

  it('attaches a picture, shows its thumbnail and sends the attachment id', async () => {
    const img = 'data:image/png;base64,iVBORw0KGgo=';
    setup({
      ...handlers(),
      resolved_models: () => ({ agent: [], code: [], plan: [], subagent: [], quick: [], vision: ['meta/llama-3.2-90b-vision-instruct'] }),
      import_attachment_data: (_c, a) => ({ id: 'att1', projectId: 'gw', conversationId: 'c1', name: String(a.name), kind: 'image', mime: 'image/png', size: 3, scope: 'reference', status: 'ready', error: null, chars: 0, meta: {}, createdAt: '2026-10-01T00:00:00Z' }),
      attachment_preview: () => ({ attachment: {}, preview: '', imageUrl: img }),
    });
    useChat.setState({ active: { chat: 'c1', research: null }, workspaceId: 'gw' });
    render(<ChatHome />);
    await flush();
    const file = new File([new Uint8Array([1, 2, 3])], 'shot.png', { type: 'image/png' });
    await act(async () => void fireEvent.change(screen.getByLabelText('Choose files'), { target: { files: [file] } }));
    await flush(80);
    expect(screen.getByAltText('shot.png').getAttribute('src')).toBe(img);
    const box = screen.getByLabelText('Message') as HTMLTextAreaElement;
    await act(async () => void fireEvent.change(box, { target: { value: 'what is this?' } }));
    await act(async () => void fireEvent.keyDown(box, { key: 'Enter' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'start_chat')?.args.request).toMatchObject({ message: 'what is this?', attachmentIds: ['att1'] });
  });

  it('reopens a picture from history; model details stay behind "Details"', async () => {
    const img = 'data:image/png;base64,iVBORw0KGgo=';
    setup({
      list_chat_conversations: () => [conv('c1', 'Screenshot')],
      conversation_messages: () => [
        msg('u1', 'user', 'what is this?', { attachments: [{ id: 'att1', name: 'roblox.png', kind: 'image' }] }),
        msg('a1', 'assistant', 'A Roblox game.', {
          report: {
            steps: 0,
            model: 'nvidia/nemotron-3.5-super-vl-120b-a12b',
            reportedModel: 'private/nvidia/nemotron-3.5-super-120b-a12b',
            vision: { route: 'native', answeredBy: 'nvidia/nemotron-3.5-super-vl-120b-a12b', images: 1 },
            usage: { totalTokens: 1573, promptTokens: 1498, completionTokens: 75, calls: 1 },
            timing: { totalMs: 9094 },
          },
        }),
      ],
      attachment_preview: () => ({ attachment: {}, preview: '', imageUrl: img }),
    });
    useChat.setState({ active: { chat: 'c1', research: null }, workspaceId: 'gw' });
    render(<ChatHome />);
    await flush(80);
    expect(screen.getByAltText('roblox.png').getAttribute('src')).toBe(img);
    fireEvent.click(screen.getByRole('button', { name: 'Open image roblox.png' }));
    expect(screen.getAllByAltText('roblox.png').length).toBe(2); // thumbnail + enlarged
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByText(/nemotron/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: /Details/ }));
    const details = screen.getByLabelText('Answer details');
    expect(within(details).getByText('nvidia/nemotron-3.5-super-vl-120b-a12b')).toBeTruthy();
    expect(within(details).getByText('private/nvidia/nemotron-3.5-super-120b-a12b')).toBeTruthy(); // the provider's own label, shown as a label
    expect(details.textContent).toContain('9.1 s');
  });

  it('explains a provider failure and offers a retry that resends the question', async () => {
    setup({
      list_chat_conversations: () => [conv('c1', 'Broken')],
      conversation_messages: () => [msg('u1', 'user', 'hello'), msg('e1', 'assistant', 'I stopped because of an error: the provider is rate limited (429).', {}, 'error')],
      rewind_conversation: () => ({ removedMessages: 2, revertedChangesets: 0 }),
    });
    useChat.setState({ active: { chat: 'c1', research: null } });
    render(<ChatHome />);
    await flush();
    expect(screen.getByRole('alert').textContent).toContain('rate limited');
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Retry' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'rewind_conversation')).toBe(true);
    expect((calls.find((c) => c.cmd === 'start_chat')?.args.request as { message: string }).message).toBe('hello');
  });

  it('shows what a running answer is doing and lets the user stop it', async () => {
    const running = { id: 'tc1', projectId: 'gw', kind: 'chat', title: 'Chat: What is an alpaca?', startedAt: new Date().toISOString() };
    setup({ ...handlers(), active_tasks: () => [running] });
    useChat.setState({ active: { chat: 'c1', research: null } });
    render(<ChatHome />);
    await flush();
    expect(screen.getByRole('status', { name: /Running/ })).toBeTruthy();
    await act(async () => void fireEvent.click(screen.getAllByRole('button', { name: /Stop/ })[0]));
    expect(calls.find((c) => c.cmd === 'cancel_task')?.args).toEqual({ taskId: 'tc1' });
  });

  it('web access can be switched from the composer and Research carries the typed text over', async () => {
    setup(handlers());
    useChat.setState({ active: { chat: 'c1', research: null } });
    render(<ChatHome />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Web on/ })));
    await flush();
    expect((calls.find((c) => c.cmd === 'save_settings')?.args.settings as { internetAccess: boolean }).internetAccess).toBe(false);
    await act(async () => void fireEvent.change(screen.getByLabelText('Message'), { target: { value: 'compare vite and webpack' } }));
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Research' })));
    expect(useApp.getState().route.name).toBe('research');
    expect(useChat.getState().pending).toEqual({ mode: 'research', text: 'compare vite and webpack' });
  });

  it('starts a project from the conversation, plans it and opens it in Code mode', async () => {
    setup(handlers());
    useChat.setState({ active: { chat: 'c1', research: null } });
    render(<ChatHome />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Conversation actions' })));
    await act(async () => void fireEvent.click(screen.getByRole('menuitem', { name: /Start a project from this conversation/ })));
    await flush();
    expect(calls.find((c) => c.cmd === 'project_from_conversation')?.args.conversationId).toBe('c1');
    expect(calls.some((c) => c.cmd === 'start_plan' && c.args.projectId === 'p-new')).toBe(true);
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p-new' });
  });

  it('lists the web pages an answer really read, and nothing else', () => {
    expect(webSourcesOf({ webSources: ['https://www.react.dev/a', 'https://www.react.dev/a', 'javascript:alert(1)', 42] })).toEqual([{ url: 'https://www.react.dev/a', host: 'react.dev' }]);
    expect(webSourcesOf(undefined)).toEqual([]);
    expect(answerDetails(undefined)).toEqual([]);
  });
});

describe('Markdown: mathematics', () => {
  it('typesets inline and display formulas with KaTeX and leaves money and code alone', () => {
    const { container } = render(<Markdown text={'The root is $x = \\frac{-b \\pm \\sqrt{b^2-4ac}}{2a}$ here.\n\n$$\\int_0^1 x^2\\,dx = \\frac13$$\n\nIt costs $5 and $6, and `$not math$` stays code. Also \\(a^2+b^2=c^2\\).'} />);
    expect(container.querySelectorAll('.md-math-inline .katex').length).toBe(2);
    expect(container.querySelectorAll('.md-math .katex-display').length).toBe(1);
    expect(container.textContent).toContain('It costs $5 and $6');
    expect(container.querySelector('code')?.textContent).toBe('$not math$');
  });

  it('typesets a multi-line display formula and shows a broken formula instead of crashing', () => {
    const { container } = render(<Markdown text={'$$\n\\begin{aligned}a&=1\\\\b&=2\\end{aligned}\n$$\n\nBroken: $\\frac{1}{$'} />);
    expect(container.querySelectorAll('.md-math .katex-display').length).toBe(1);
    expect(container.textContent).toContain('Broken');
  });

  it('never lets a formula inject markup', () => {
    const { container } = render(<Markdown text={'$\\href{javascript:alert(1)}{x}$ and $\\includegraphics{http://evil/x.png}$ <img src=x onerror=alert(1)>'} />);
    expect(container.querySelector('a[href^="javascript"]')).toBeNull();
    expect(container.querySelector('img')).toBeNull();
  });
});
