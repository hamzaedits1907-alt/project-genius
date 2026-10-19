// Last line of defence: an unexpected error anywhere in the web shell must never leave a blank page.
import { Component, type ReactNode } from 'react';

export class RootBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    if (!this.state.error) return this.props.children;
    return (
      <main className="au-root">
        <div className="au-card" role="alert">
          <h1>Something went wrong</h1>
          <p className="muted">Genius hit an unexpected error and stopped. Your conversations are safe in your account. Reload to continue; if it keeps happening, tell the owner what you were doing.</p>
          <pre className="small" style={{ whiteSpace: 'pre-wrap' }}>{this.state.error.message.slice(0, 300)}</pre>
          <button className="btn" onClick={() => location.reload()}>Reload</button>
        </div>
      </main>
    );
  }
}
