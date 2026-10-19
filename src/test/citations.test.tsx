import { describe, expect, it, vi } from 'vitest';
import { fireEvent, render, screen } from '@testing-library/react';
import { Markdown } from '../components/Markdown';
import { isPublicHttpUrl } from '../lib/platform';

const opened = vi.hoisted(() => ({ calls: [] as string[] }));
vi.mock('@tauri-apps/plugin-opener', () => ({
  openUrl: (u: string) => {
    opened.calls.push(u);
    return Promise.resolve();
  },
}));

describe('public links only', () => {
  it('accepts ordinary web addresses and rejects the app itself, the machine and private networks', () => {
    for (const ok of ['https://react.dev/learn', 'http://example.com/a?b=1#c', 'https://lufthansa-connect-ptfs.netlify.app/']) expect(isPublicHttpUrl(ok), ok).toBe(true);
    for (const bad of [
      'http://tauri.localhost/#src-1',
      'https://ipc.localhost/x',
      'http://localhost:3000',
      'http://127.0.0.1:8080/x',
      'http://192.168.1.2/',
      'http://10.0.0.5/',
      'http://172.20.0.1/',
      'http://169.254.169.254/latest',
      'http://[::1]/',
      'file:///etc/passwd',
      'javascript:alert(1)',
      '#src-1',
      '',
    ])
      expect(isPublicHttpUrl(bad), bad).toBe(false);
  });
});

describe('citations in answers', () => {
  it('without a source record a citation is plain text, never an in-page anchor', () => {
    const { container } = render(<Markdown text="Hooks batch updates [1]." />);
    expect(container.querySelector('a')).toBeNull();
    expect(container.querySelector('sup.cite .cite-plain')?.textContent).toBe('1');
    expect(container.innerHTML).not.toContain('#src-');
  });

  it('a citation with a fetched source links to the real page and opens it when no Sources panel is showing', () => {
    opened.calls.length = 0;
    const { container } = render(<Markdown text="Hooks batch updates [1]." sources={[{ n: 1, url: 'https://react.dev/learn', title: 'Learn React' }]} />);
    const a = container.querySelector('sup.cite a') as HTMLAnchorElement;
    expect(a.getAttribute('href')).toBe('https://react.dev/learn');
    fireEvent.click(a);
    expect(opened.calls).toEqual(['https://react.dev/learn']);
  });

  it('a mounted Sources panel handles the click instead (highlight, no navigation)', () => {
    opened.calls.length = 0;
    const seen: number[] = [];
    const on = (e: Event) => {
      const d = (e as CustomEvent<{ n: number; handled: boolean }>).detail;
      d.handled = true;
      seen.push(d.n);
    };
    window.addEventListener('genius-cite', on);
    const { container } = render(<Markdown text="Claim [2]." sources={[{ n: 2, url: 'https://vite.dev/guide/' }]} />);
    fireEvent.click(container.querySelector('sup.cite a')!);
    window.removeEventListener('genius-cite', on);
    expect(seen).toEqual([2]);
    expect(opened.calls).toEqual([]);
  });

  it('a source whose address is local is never linked', () => {
    const { container } = render(<Markdown text="Claim [1]." sources={[{ n: 1, url: 'http://tauri.localhost/#src-1' }]} />);
    expect(container.querySelector('a')).toBeNull();
  });

  it('Markdown links to local addresses lose their link; public ones keep it', () => {
    render(<Markdown text="See [the docs](https://react.dev/learn) and [source one](http://tauri.localhost/#src-1) and [dev](http://localhost:3000)." />);
    expect(screen.getByText('the docs').closest('a')?.getAttribute('href')).toBe('https://react.dev/learn');
    expect(screen.getByText('source one').closest('a')).toBeNull();
    expect(screen.getByText('dev').closest('a')).toBeNull();
  });
});
