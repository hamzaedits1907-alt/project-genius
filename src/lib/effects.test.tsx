// Regression tests for "Something went wrong in this view — destroy is not a function".
import { StrictMode, useEffect } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render } from '@testing-library/react';
import { ESLint } from 'eslint';
import { asCleanup, InvalidCleanupError, useSubscription } from './effects';
import { firstComponent } from './logic';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('the original failure mode', () => {
  it('React 19 calls a non-function effect return value on unmount ("destroy is not a function")', () => {
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    // An expression-bodied effect whose expression evaluates to something other than undefined.
    const notACleanup = () => ({ ok: true });
    function Bad() {
      // @ts-expect-error — deliberately reproduces the bug class the lint rule forbids
      // eslint-disable-next-line no-restricted-syntax
      useEffect(() => notACleanup(), []);
      return null;
    }
    const r = render(<Bad />);
    expect(() => r.unmount()).toThrow(/destroy is not a function/);
  });

  it('the lint rule rejects every way an effect can leak a return value', async () => {
    const eslint = new ESLint();
    const code = `import { useEffect } from 'react';
const load = () => undefined;
export function A() {
  useEffect(() => document.body.scrollIntoView(), []);
  useEffect(load, []);
  useEffect(async () => { await Promise.resolve(); }, []);
  useEffect(() => { load(); }, []);
  return null;
}
`;
    const [res] = await eslint.lintText(code, { filePath: 'src/__lint_probe.tsx' });
    const msgs = res.messages.filter((m) => m.ruleId === 'no-restricted-syntax').map((m) => m.line);
    expect(msgs).toEqual([4, 5, 6]);
  });
});

describe('asCleanup', () => {
  it('accepts functions and undefined, rejects everything else with a precise message', () => {
    const f = () => undefined;
    expect(asCleanup('x', f)).toBe(f);
    expect(asCleanup('x', undefined)()).toBeUndefined();
    expect(() => asCleanup('events', null)).toThrow(new InvalidCleanupError('events', null));
    expect(() => asCleanup('events', Promise.resolve())).toThrow(/returned a Promise/);
    expect(() => asCleanup('events', {})).toThrow(/returned an object/);
  });
});

describe('useSubscription', () => {
  function Sub({ subscribe, onError }: { subscribe: () => unknown; onError?: (e: unknown) => void }) {
    useSubscription('test', subscribe as () => () => void, [], onError);
    return null;
  }

  it('releases an async subscription that resolves after unmount (StrictMode double mount)', async () => {
    const released: number[] = [];
    let n = 0;
    const subscribe = () => {
      const id = ++n;
      return new Promise<() => void>((r) => setTimeout(() => r(() => released.push(id)), 5));
    };
    const r = render(
      <StrictMode>
        <Sub subscribe={subscribe} />
      </StrictMode>,
    );
    await act(() => new Promise((res) => setTimeout(res, 20)));
    // StrictMode mounted twice: the first subscription was released as soon as it resolved.
    expect(n).toBe(2);
    expect(released).toEqual([1]);
    r.unmount();
    expect(released).toEqual([1, 2]);
  });

  it('reports (never crashes on) a subscription that yields a non-function', async () => {
    const errors: unknown[] = [];
    const r = render(<Sub subscribe={() => Promise.resolve(null)} onError={(e) => errors.push(e)} />);
    await act(() => new Promise((res) => setTimeout(res, 5)));
    expect(() => r.unmount()).not.toThrow();
    expect(errors).toHaveLength(1);
    expect(errors[0]).toBeInstanceOf(InvalidCleanupError);
  });

  it('reports a failing cleanup without breaking unmount', () => {
    const errors: unknown[] = [];
    const r = render(
      <Sub
        subscribe={() => () => {
          throw new Error('boom');
        }}
        onError={(e) => errors.push(e)}
      />,
    );
    expect(() => r.unmount()).not.toThrow();
    expect(String(errors[0])).toMatch(/boom/);
  });
});

describe('firstComponent', () => {
  it('extracts the innermost component name from a React component stack', () => {
    expect(firstComponent('\n    at AssistantPanel (http://localhost:1420/src/x.tsx:10:3)\n    at div')).toBe('AssistantPanel');
    expect(firstComponent('\n    in MemoryPanel\n    in div')).toBe(null);
    expect(firstComponent('')).toBe(null);
  });
});
