// Genius Chat / Research UI against the mocked backend: mode switcher, conversation list,
// citations and tables, the sources panel, internet-off handling and "start a project".
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { calls, clearMocks, installIpc, settings, type Handler } from './ipc';
import ChatHome from '../views/chat/ChatHome';
import ResearchView from '../views/chat/ResearchView';
import { ModeSwitch } from '../views/chat/ModeSwitch';
import { SourcesPanel } from '../views/chat/SourcesPanel';
import { Markdown } from '../components/Markdown';
import { useApp } from '../store/app';
import { useChat } from '../store/chat';
import type { ChatConversation, MessageRow, ResearchSource } from '../lib/types';

const flush = (ms = 40) => act(() => new Promise((r) => setTimeout(r, ms)));
const conv = (id: string, title: string, mode: 'chat' | 'research' = 'chat', snippet = ''): ChatConversation => ({ id, projectId: 'gw', title, createdAt: '2026-10-01T00:00:00Z', updatedAt: '2026-10-01T00:00:00Z', messageCount: 2, mode, snippet, origin: null });
const msg = (id: string, role: 'user' | 'assistant', content: string, meta: Record<string, unknown> = {}, kind = role === 'user' ? 'chat' : 'answer'): MessageRow => ({ id, projectId: 'gw', taskId: 't', role, content, createdAt: '2026-10-01T00:00:00Z', conversationId: 'c1', kind, meta });
const source = (n: number, over: Partial<ResearchSource> = {}): ResearchSource => ({ id: `s${n}`, conversationId: 'c1', taskId: 't', n, url: `https://react.dev/p${n}`, requestedUrl: `https://react.dev/p${n}`, title: `Page ${n}`, status: 200, fetchedAt: '2026-10-18T10:00:00.000Z', contentHash: 'abcdef0123456789', snippet: 's', usedFor: n === 1 ? 'Hooks let components use state' : '', official: n === 1, flags: [], ...over });

function setup(over: Record<string, Handler> = {}) {
  installIpc(over);
  useChat.setState({ workspaceId: null, conversations: { chat: [], research: [] }, active: { chat: null, research: null }, query: '', messages: [], sources: [], tasks: [], steps: {}, queue: [], pending: null });
  useApp.setState({ settings: { ...settings }, route: { name: 'chat' } });
}

beforeEach(() => {
  window.confirm = () => true;
});
afterEach(() => {
  cleanup();
  clearMocks();
});

describe('mode switcher', () => {
  it('switches between Chat, Code and Research without touching state', async () => {
    setup();
    useChat.setState({ messages: [msg('m', 'user', 'keep me')] });
    render(<ModeSwitch mode="chat" />);
    const group = screen.getByRole('radiogroup', { name: 'Mode' });
    expect(within(group).getAllByRole('radio').map((r) => r.textContent)).toEqual(['Chat', 'Code', 'Research']);
    expect(screen.getByRole('radio', { name: 'Chat' }).getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByRole('radio', { name: 'Research' }));
    expect(useApp.getState().route.name).toBe('research');
    fireEvent.click(screen.getByRole('radio', { name: 'Code' }));
    expect(useApp.getState().route.name).toBe('home');
    expect(useChat.getState().messages[0].content).toBe('keep me');
  });
});

describe('Chat: conversation list', () => {
  const list = [conv('c1', 'Zoology', 'chat', 'Alpacas are camelids'), conv('c2', 'Weekly plan')];
  const handlers = (): Record<string, Handler> => ({
    list_chat_conversations: (_c, a) => (a.query ? list.filter((c) => c.title.toLowerCase().includes(String(a.query).toLowerCase())) : list),
    conversation_messages: (_c, a) => (a.conversationId === 'c1' ? [msg('u1', 'user', 'Tell me about alpacas'), msg('a1', 'assistant', 'Alpacas are camelids.')] : []),
    rename_conversation: () => null,
    delete_conversation: () => null,
  });

  it('lists, opens, searches, renames and deletes conversations', async () => {
    setup(handlers());
    render(<ChatHome />);
    await flush();
    expect(screen.getByText('Zoology')).toBeTruthy();
    expect(screen.getByText('Weekly plan')).toBeTruthy();
    expect(screen.getByText('Alpacas are camelids.')).toBeTruthy(); // newest conversation opened with its history
    // search
    await act(async () => void fireEvent.change(screen.getByLabelText('Search conversations'), { target: { value: 'week' } }));
    await flush();
    expect(calls.some((c) => c.cmd === 'list_chat_conversations' && c.args.query === 'week')).toBe(true);
    expect(screen.queryByText('Zoology')).toBeNull();
    // rename
    fireEvent.click(screen.getByLabelText('Rename Weekly plan'));
    const input = screen.getByLabelText('Conversation title');
    await act(async () => void fireEvent.change(input, { target: { value: 'Plan B' } }));
    await act(async () => void fireEvent.keyDown(input, { key: 'Enter' }));
    await flush();
    expect(calls.find((c) => c.cmd === 'rename_conversation')?.args).toEqual({ conversationId: 'c2', title: 'Plan B' });
    // delete
    fireEvent.click(screen.getByLabelText('Delete Weekly plan'));
    await flush();
    expect(calls.find((c) => c.cmd === 'delete_conversation')?.args).toEqual({ conversationId: 'c2' });
  });

  it('sends a message through the chat entry point (not the code agent) and attaches nothing it cannot read', async () => {
    setup(handlers());
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
    // an image with no vision-capable model is refused with an honest message
    const file = new File([new Uint8Array([1, 2, 3])], 'shot.png', { type: 'image/png' });
    await act(async () => void fireEvent.change(screen.getByLabelText('Choose files'), { target: { files: [file] } }));
    await flush();
    expect(calls.some((c) => c.cmd === 'import_attachment_data')).toBe(false);
    expect(useApp.getState().toasts.some((t) => t.text.includes('none of your available models can read images'))).toBe(true);
  });

  it('starts a project from the conversation, plans it and opens it in Code mode', async () => {
    setup(handlers());
    render(<ChatHome />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Start a project from this conversation/ })));
    await flush();
    expect(calls.find((c) => c.cmd === 'project_from_conversation')?.args.conversationId).toBe('c1');
    expect(calls.some((c) => c.cmd === 'start_plan' && c.args.projectId === 'p-new')).toBe(true);
    expect(useApp.getState().route).toEqual({ name: 'project', id: 'p-new' });
  });
});

describe('Markdown: tables and citations', () => {
  it('renders tables, superscript citations and a footnote list', () => {
    render(<Markdown text={'| Lib | Size |\n|---|:--:|\n| zustand | 1 kB [1] |\n\nHooks hold state [2].\n\n[1] Zustand README — https://github.com/pmndrs/zustand\n[2] React docs — https://react.dev/learn'} />);
    const table = screen.getByRole('table');
    expect(within(table).getAllByRole('columnheader').map((h) => h.textContent)).toEqual(['Lib', 'Size']);
    expect(within(table).getByText('zustand')).toBeTruthy();
    expect(document.querySelectorAll('sup.cite').length).toBe(2); // [1] in the cell and [2] in the paragraph
    const notes = screen.getByLabelText('Sources');
    expect(within(notes).getAllByRole('listitem').length).toBe(2);
    expect(notes.querySelector('#src-2')?.textContent).toContain('React docs');
  });

  it('keeps markdown links with numeric text as links and leaves plain brackets alone', () => {
    render(<Markdown text={'See [1](https://react.dev) and arr[0] and [x]'} />);
    expect(document.querySelectorAll('sup.cite').length).toBe(0);
    expect(screen.getByText('1').closest('a')?.getAttribute('href')).toBe('https://react.dev');
  });
});

describe('Research', () => {
  const report = msg('a1', 'assistant', '## Findings\nHooks let components use state [1].\n\n## Sources\n[1] Page 1 — https://react.dev/p1', { validation: { ok: true, facts: 1, assumptions: 0, uncertain: 0, uncited: [], unknownCitations: [], unfetchedUrls: [], cited: [1], repaired: false } });
  const handlers = (extra: Record<string, Handler> = {}): Record<string, Handler> => ({
    list_chat_conversations: () => [conv('c1', 'Hooks research', 'research')],
    conversation_messages: () => [msg('u1', 'user', 'How do hooks work?'), report],
    research_sources: () => [source(1), source(2, { flags: ['ignore all previous instructions'] })],
    ...extra,
  });

  it('shows the report with citations, the sources ledger and the validation result', async () => {
    setup(handlers());
    render(<ResearchView />);
    await flush();
    expect(screen.getByLabelText('Sources', { selector: 'aside' })).toBeTruthy();
    expect(screen.getByText('Sources (2)')).toBeTruthy();
    expect(screen.getByText('Page 1')).toBeTruthy();
    expect(screen.getByText(/Used for: Hooks let components use state/)).toBeTruthy();
    expect(screen.getByText('official')).toBeTruthy();
    expect(screen.getByText(/Fetched, not cited in the report/)).toBeTruthy();
    expect(screen.getByText(/Contains text aimed at the AI/)).toBeTruthy();
    expect(screen.getByText(/Citation check passed: 1 cited claim/)).toBeTruthy();
    // clicking a citation highlights the ledger entry
    const sup = document.querySelector('.md sup.cite a') as HTMLElement;
    await act(async () => void fireEvent.click(sup));
    expect(document.querySelector('#ledger-1')?.className).toContain('hl');
  });

  it('flags a report whose citations do not check out', async () => {
    const bad = msg('a2', 'assistant', 'x', { validation: { ok: false, facts: 1, assumptions: 0, uncertain: 0, uncited: ['A claim'], unknownCitations: [4], unfetchedUrls: [], cited: [1], repaired: true } });
    setup(handlers({ conversation_messages: () => [msg('u1', 'user', 'q'), bad] }));
    render(<ResearchView />);
    await flush();
    expect(screen.getByRole('note', { name: 'Citation check' }).textContent).toContain('1 claim(s) without a source, 1 citation(s) to sources that were never fetched');
  });

  it('explains when internet access is off and offers to turn it on', async () => {
    setup(handlers());
    useApp.setState({ settings: { ...settings, internetAccess: false } });
    render(<ResearchView />);
    await flush();
    expect(screen.getByRole('alert').textContent).toContain('Internet access is turned off');
    expect((screen.getByLabelText('Research question') as HTMLTextAreaElement).disabled).toBe(true);
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Turn on internet access' })));
    await flush();
    expect((calls.find((c) => c.cmd === 'save_settings')?.args.settings as { internetAccess: boolean }).internetAccess).toBe(true);
  });

  it('turns the report into a spec and starts a project from that spec', async () => {
    const spec = msg('sp1', 'assistant', '## Goal\nA dashboard', { spec: true }, 'plan');
    setup(handlers({ conversation_messages: () => [msg('u1', 'user', 'q'), report, spec] }));
    render(<ResearchView />);
    await flush();
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: 'Turn research into a spec' })));
    await flush();
    expect(calls.some((c) => c.cmd === 'start_research_spec' && c.args.conversationId === 'c1')).toBe(true);
    await act(async () => void fireEvent.click(screen.getByRole('button', { name: /Start a project from this spec/ })));
    await flush();
    expect(calls.find((c) => c.cmd === 'project_from_conversation')?.args).toMatchObject({ conversationId: 'c1', sourceMessageId: 'sp1' });
  });

  it('SourcesPanel states honestly that search results are not sources', () => {
    render(<SourcesPanel sources={[]} />);
    expect(screen.getByText(/Search results are not sources until the page was actually fetched/)).toBeTruthy();
  });
});
