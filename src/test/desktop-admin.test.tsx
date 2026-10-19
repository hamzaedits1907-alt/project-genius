// Regression: the owner admin panel was never wired into the desktop app (the account block always said
// isAdmin: false and no "admin" screen was registered), so it could not be opened there at all.
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { AccountBlock } from '../components/AccountBlock';
import { extraViews } from '../lib/extraViews';
import { useApp } from '../store/app';
import { initDesktopAccount } from '../web/desktopAccount';
import { useAuth, type Access } from '../web/auth';

const access = (isAdmin: boolean): Access => ({ signedIn: true, status: 'active', plan: 'free', planExpiresAt: null, isAdmin, features: {} });
const session = { user: { email: 'owner@example.com' } } as never;

afterEach(() => {
  cleanup();
  useAuth.setState({ session: null, access: null });
});

describe('desktop account and admin', () => {
  it('registers the admin screen', () => {
    initDesktopAccount();
    expect(extraViews.admin).toBeTypeOf('function');
  });

  it('an administrator opens the admin panel from the account block', () => {
    initDesktopAccount();
    useAuth.setState({ session, access: access(true) });
    render(<AccountBlock />);
    expect(screen.getByText('Free · owner')).toBeInTheDocument();
    act(() => screen.getByLabelText('Signed in as owner@example.com').click());
    expect(useApp.getState().route.name).toBe('admin');
  });

  it('an ordinary user gets Settings → Account, never the admin screen, from the same button', () => {
    initDesktopAccount();
    useApp.getState().navigate({ name: 'chat' });
    useAuth.setState({ session, access: access(false) });
    render(<AccountBlock />);
    act(() => screen.getByLabelText('Signed in as owner@example.com').click());
    expect(useApp.getState().route.name).toBe('settings');
  });
});
