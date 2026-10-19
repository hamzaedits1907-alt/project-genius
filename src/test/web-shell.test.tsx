// Regression: the deployed Genius Web showed a blank page right after sign-in (React error #185,
// "Maximum update depth exceeded"). Cause: the announcements selector returned a NEW empty array on
// every call while the account context had not loaded yet, which makes zustand v5 re-render forever.
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Announcements } from '../web/WebApp';
import { useAuth } from '../web/auth';

afterEach(() => {
  cleanup();
  useAuth.setState({ context: null });
});

describe('account announcements', () => {
  it('renders nothing (and does not loop) while the account context is not loaded', () => {
    const errors = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    useAuth.setState({ context: null });
    const { container } = render(<Announcements />);
    expect(container).toBeEmptyDOMElement();
    expect(errors.mock.calls.flat().join(' ')).not.toMatch(/getSnapshot|Maximum update depth/);
    errors.mockRestore();
  });

  it('shows announcements once they arrive and lets the user dismiss one', () => {
    useAuth.setState({ context: { limits: null, announcements: [{ id: 'a1', title: 'Maintenance tonight', body: 'Short break.', level: 'warning' }] } });
    render(<Announcements />);
    expect(screen.getByText('Maintenance tonight')).toBeInTheDocument();
    act(() => screen.getByLabelText('Dismiss Maintenance tonight').click());
    expect(screen.queryByText('Maintenance tonight')).toBeNull();
  });
});
