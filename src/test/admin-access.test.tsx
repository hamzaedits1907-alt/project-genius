// The owner's Access tab: switching Genius between open / early access / closed, inviting people and
// deciding requests — against a recording stand-in for the database functions (the real rules are tested in
// tests/db/access.test.ts on PostgreSQL).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const calls: { fn: string; args: Record<string, unknown> | undefined }[] = [];
let overview = { access: { mode: 'open', cohort: 'early', message: '', waitlist: true }, users: 12, admins: 1, cohortMembers: 3, allowedNow: 12, pendingRequests: 1, invitesPending: 2, invitesClaimed: 1, cohorts: [{ id: 'early', name: 'Early access', members: 3 }] };
const requests = [{ user_id: 'u-bob', email: 'bob@example.com', note: 'big fan', status: 'pending', created_at: '2026-10-20T10:00:00Z' }];

vi.mock('../web/auth', async (orig) => {
  const real = await orig<typeof import('../web/auth')>();
  const rpc = async (fn: string, args?: Record<string, unknown>) => {
    calls.push({ fn, args });
    const data: Record<string, unknown> = {
      admin_usage_summary: { users: 12, pro: 1, suspended: 0, conversations: 5, messages: 9, activeLast7Days: 4 },
      admin_usage_series: [],
      admin_top_users: [],
      admin_signups_series: [],
      admin_access_overview: overview,
      admin_list_access_requests: requests,
      admin_list_invites: [{ email: 'friend@example.com', cohort: 'early', created_at: '2026-10-19T10:00:00Z', claimed_by: null, claimed_at: null }],
      admin_invite_emails: { invited: 1, addedNow: 1, invalid: [] },
    };
    return { data: data[fn] ?? null, error: null };
  };
  return { ...real, sb: () => ({ rpc, from: () => ({ select: () => Promise.resolve({ data: [], error: null }) }) }) };
});

import AdminPanel from '../web/AdminPanel';
import { useAuth } from '../web/auth';

beforeEach(() => {
  calls.length = 0;
  useAuth.setState({ access: { signedIn: true, status: 'active', plan: 'free', planExpiresAt: null, isAdmin: true, features: {} } });
});
afterEach(() => {
  cleanup();
  useAuth.setState({ access: null });
});

const openAccessTab = async () => {
  render(<AdminPanel />);
  fireEvent.click(await screen.findByRole('tab', { name: 'Access & invites' }));
  await screen.findByText('Who may use Genius');
};

describe('admin Access tab', () => {
  it('shows who is in and lists the waiting request and invitation', async () => {
    await openAccessTab();
    expect(screen.getByText(/Genius is open to everyone/)).toBeInTheDocument();
    expect(screen.getByText('bob@example.com')).toBeInTheDocument();
    expect(screen.getByText('big fan')).toBeInTheDocument();
    expect(screen.getByText('friend@example.com')).toBeInTheDocument();
  });

  it('switching to early access asks for confirmation, then applies it through the database function', async () => {
    await openAccessTab();
    fireEvent.click(screen.getByLabelText(/Early access only/));
    fireEvent.change(screen.getByPlaceholderText(/private beta/i), { target: { value: 'Ask me for an invite' } });
    fireEvent.click(screen.getByRole('button', { name: 'Apply…' }));
    expect(screen.getByRole('alert')).toHaveTextContent(/takes effect immediately/);
    expect(calls.some((c) => c.fn === 'admin_set_access')).toBe(false); // nothing happened yet
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Yes, apply now' })));
    await waitFor(() => expect(calls.find((c) => c.fn === 'admin_set_access')?.args).toEqual({ p_mode: 'early', p_cohort: 'early', p_message: 'Ask me for an invite', p_waitlist: true }));
  });

  it('opening again needs no confirmation', async () => {
    overview = { ...overview, access: { mode: 'closed', cohort: 'early', message: '', waitlist: true } };
    await openAccessTab();
    expect(screen.getByText(/Genius is CLOSED/)).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText(/Open to everyone/));
    await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Apply' })));
    await waitFor(() => expect(calls.find((c) => c.fn === 'admin_set_access')?.args).toMatchObject({ p_mode: 'open' }));
    overview = { ...overview, access: { mode: 'open', cohort: 'early', message: '', waitlist: true } };
  });

  it('invites a pasted list of addresses and approves a request', async () => {
    await openAccessTab();
    fireEvent.change(screen.getByLabelText('E-mail addresses to invite'), { target: { value: 'a@example.com, b@example.com\nc@example.com' } });
    await act(async () => fireEvent.click(screen.getByRole('button', { name: /Invite 3 addresses/ })));
    await waitFor(() => expect(calls.find((c) => c.fn === 'admin_invite_emails')?.args).toEqual({ p_emails: ['a@example.com', 'b@example.com', 'c@example.com'], p_cohort: 'early' }));
    const row = screen.getByText('bob@example.com').closest('tr')!;
    await act(async () => fireEvent.click(within(row).getByRole('button', { name: 'Let in' })));
    await waitFor(() => expect(calls.find((c) => c.fn === 'admin_decide_access_request')?.args).toEqual({ p_user: 'u-bob', p_approve: true }));
  });

  it('a person who is not an administrator never reaches these controls', async () => {
    useAuth.setState({ access: { signedIn: true, status: 'active', plan: 'free', planExpiresAt: null, isAdmin: false, features: {} } });
    render(<AdminPanel />);
    expect(await screen.findByText(/Not authorized/)).toBeInTheDocument();
    expect(calls).toHaveLength(0);
  });
});
