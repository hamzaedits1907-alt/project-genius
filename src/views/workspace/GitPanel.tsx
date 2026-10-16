import { useCallback, useEffect, useState } from 'react';
import { ArrowDown, ArrowUp, Bot, GitBranch, GitCommit, Minus, Plus, RefreshCw, Sparkles, UploadCloud } from 'lucide-react';
import { api, errorKind, errorMessage } from '../../lib/api';
import { timeAgo } from '../../lib/logic';
import type { Branch, Commit, GitFileStatus, GitView, SecretFinding } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout, DiffText, Empty, IconButton, Modal, Spinner } from '../../components/ui';

function code(f: GitFileStatus, staged: boolean) {
  if (f.conflicted) return { c: 'U', color: 'var(--bad)' };
  if (f.untracked) return { c: 'U', color: 'var(--ok)' };
  const x = staged ? f.index : f.worktree;
  const color = x === 'A' ? 'var(--ok)' : x === 'D' ? 'var(--bad)' : 'var(--warn)';
  return { c: x, color };
}

export default function GitPanel({ onPublish }: { onPublish: () => void }) {
  const projectId = useWorkspace((s) => s.projectId)!;
  const version = useWorkspace((s) => s.gitVersion);
  const openTab = useWorkspace((s) => s.openTab);
  const toast = useApp((s) => s.toast);
  const [view, setView] = useState<GitView | null>(null);
  const [branches, setBranches] = useState<Branch[]>([]);
  const [log, setLog] = useState<Commit[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState('');
  const [diff, setDiff] = useState<{ path: string; text: string } | null>(null);
  const [findings, setFindings] = useState<SecretFinding[] | null>(null);
  const [identity, setIdentity] = useState({ name: '', email: '' });
  const [remoteUrl, setRemoteUrl] = useState('');
  const [newBranch, setNewBranch] = useState<string | null>(null);
  const [diverged, setDiverged] = useState(false);

  const load = useCallback(async () => {
    try {
      const v = await api.gitView(projectId);
      setView(v);
      if (v.status.isRepo) {
        const [b, l] = await Promise.all([api.gitBranches(projectId), api.gitLog(projectId, 50)]);
        setBranches(b);
        setLog(l);
      }
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  }, [projectId, toast]);
  useEffect(() => {
    void load();
  }, [load, version]);

  const act = async (name: string, fn: () => Promise<unknown>, ok?: string) => {
    setBusy(name);
    try {
      await fn();
      if (ok) toast('ok', ok);
    } catch (e) {
      if (name === 'pull' && errorKind(e) === 'conflict') setDiverged(true);
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
      void load();
    }
  };

  if (!view) return <Empty icon={<Spinner />} title="Reading repository…" />;
  if (!view.available) return <Empty icon={<GitBranch size={18} />} title="Git is not available">{view.error}</Empty>;
  if (!view.status.isRepo)
    return (
      <>
        <div className="panel-head">
          <span className="panel-title">Source control</span>
        </div>
        <Empty
          icon={<GitBranch size={18} />}
          title="Not a Git repository"
          action={
            <div className="col">
              <Button variant="primary" loading={busy === 'init'} onClick={() => void act('init', () => api.gitInit(projectId), 'Repository initialized')}>
                Initialize repository
              </Button>
              <Button icon={<UploadCloud size={14} />} onClick={onPublish}>
                Publish to GitHub
              </Button>
            </div>
          }
        >
          Track changes with Git. Genius adds a .gitignore for dependencies, build output and secrets (keeping any rules you already have).
        </Empty>
      </>
    );

  const st = view.status;
  const staged = st.files.filter((f) => f.staged);
  const changes = st.files.filter((f) => (f.unstaged || f.untracked) && !f.conflicted);
  const conflicts = st.files.filter((f) => f.conflicted);
  const origin = st.remotes.find((r) => r.name === 'origin');
  const geniusUnstaged = changes.filter((f) => view.geniusPaths.includes(f.path)).map((f) => f.path);

  const showDiff = async (f: GitFileStatus, isStaged: boolean) => {
    try {
      setDiff({ path: f.path, text: await api.gitDiff(projectId, f.path, isStaged, f.untracked) });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const commit = async (allow: boolean) => {
    setBusy('commit');
    try {
      if (!allow) {
        const f = await api.stagedFindings(projectId);
        if (f.length) {
          setFindings(f);
          return;
        }
      }
      const c = await api.gitCommit(projectId, message, allow);
      toast('ok', `Committed ${c.short}`);
      setMessage('');
      setFindings(null);
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
      void load();
    }
  };
  const suggest = async () => {
    setBusy('suggest');
    try {
      setMessage(await api.suggestCommitMessage(projectId));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  const fileRow = (f: GitFileStatus, isStaged: boolean) => {
    const c = code(f, isStaged);
    const genius = view.geniusPaths.includes(f.path);
    return (
      <div key={`${isStaged}:${f.path}`} className="git-file" onClick={() => void showDiff(f, isStaged)} role="button" tabIndex={0} onKeyDown={(e) => e.key === 'Enter' && void showDiff(f, isStaged)}>
        <span className="st" style={{ color: c.color }}>
          {c.c}
        </span>
        <span className="ellipsis grow mono tiny" title={f.path}>
          {f.path}
        </span>
        {genius ? <Badge tone="accent" title="Changed by Genius since the last commit">Genius</Badge> : <Badge title="Not changed through Genius — review before committing">external</Badge>}
        <span className="acts">
          <IconButton
            label={isStaged ? 'Unstage' : 'Stage'}
            onClick={(e) => {
              e.stopPropagation();
              void act('stage', () => api.gitStage(projectId, [f.path], !isStaged));
            }}
            style={{ width: 22, height: 22 }}
          >
            {isStaged ? <Minus size={13} /> : <Plus size={13} />}
          </IconButton>
        </span>
      </div>
    );
  };

  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">Source control</span>
        <IconButton label="Refresh" onClick={() => void load()}>
          <RefreshCw size={14} />
        </IconButton>
      </div>
      <div className="scroll grow">
        <div className="git-section" style={{ padding: 10 }}>
          <div className="row">
            <GitBranch size={14} />
            <select
              className="select"
              style={{ height: 28, padding: '2px 6px' }}
              value={st.branch ?? ''}
              onChange={(e) => (e.target.value === '__new' ? setNewBranch('') : void act('switch', () => api.gitBranch(projectId, e.target.value, false), `Switched to ${e.target.value}`))}
              aria-label="Branch"
            >
              {!st.branch && <option value="">(detached)</option>}
              {branches.map((b) => (
                <option key={b.name} value={b.name}>
                  {b.name}
                </option>
              ))}
              <option value="__new">+ New branch…</option>
            </select>
          </div>
          <div className="row small muted" style={{ marginTop: 6 }}>
            {origin ? <span className="mono tiny ellipsis grow" title={origin.url}>{origin.url}</span> : <span className="grow">No remote</span>}
            {st.upstream && (
              <span title={`vs ${st.upstream}`}>
                <ArrowUp size={11} />
                {st.ahead} <ArrowDown size={11} />
                {st.behind}
              </span>
            )}
          </div>
          <div className="row wrap" style={{ marginTop: 8, gap: 4 }}>
            {origin ? (
              <>
                <Button size="sm" loading={busy === 'fetch'} onClick={() => void act('fetch', () => api.gitFetch(projectId), 'Fetched')}>
                  Fetch
                </Button>
                <Button size="sm" loading={busy === 'pull'} onClick={() => void act('pull', () => api.gitPull(projectId, false), 'Pulled')}>
                  Pull
                </Button>
                <Button size="sm" variant="primary" loading={busy === 'push'} onClick={() => void act('push', () => api.gitPush(projectId), 'Push verified on remote')}>
                  Push
                </Button>
              </>
            ) : (
              <Button size="sm" variant="primary" icon={<UploadCloud size={13} />} onClick={onPublish}>
                Publish to GitHub
              </Button>
            )}
          </div>
          {diverged && (
            <Callout tone="warn">
              Histories have diverged. Pull with merge combines them; conflicts (if any) will be listed here for you to resolve.
              <div className="row" style={{ marginTop: 6 }}>
                <Button size="sm" loading={busy === 'merge'} onClick={() => void act('merge', () => api.gitPull(projectId, true), 'Merged').then(() => setDiverged(false))}>
                  Pull with merge
                </Button>
              </div>
            </Callout>
          )}
          {!origin && (
            <div className="row" style={{ marginTop: 8 }}>
              <input className="input mono" style={{ height: 28 }} placeholder="https://github.com/you/repo.git" value={remoteUrl} onChange={(e) => setRemoteUrl(e.target.value)} aria-label="Remote URL" />
              <Button size="sm" disabled={!remoteUrl} onClick={() => void act('remote', () => api.gitSetRemote(projectId, 'origin', remoteUrl), 'Remote set')}>
                Set
              </Button>
            </div>
          )}
        </div>

        {!view.identity && (
          <div className="git-section" style={{ padding: 10 }}>
            <Callout tone="warn">Git needs a name and email for commits in this repository.</Callout>
            <div className="col" style={{ marginTop: 6 }}>
              <input className="input" placeholder="Your name" value={identity.name} onChange={(e) => setIdentity({ ...identity, name: e.target.value })} />
              <input className="input" placeholder="you@example.com" value={identity.email} onChange={(e) => setIdentity({ ...identity, email: e.target.value })} />
              <Button size="sm" onClick={() => void act('identity', () => api.gitSetIdentity(projectId, identity.name, identity.email), 'Identity saved for this repository')}>
                Save for this repository
              </Button>
            </div>
          </div>
        )}

        {(conflicts.length > 0 || st.merging) && (
          <div className="git-section">
            <div className="git-head" style={{ color: 'var(--bad)' }}>
              Merge conflicts ({conflicts.length})
            </div>
            {conflicts.map((f) => (
              <div key={f.path} className="col" style={{ padding: '4px 10px', gap: 4 }}>
                <span className="mono tiny">{f.path}</span>
                <div className="row" style={{ gap: 4 }}>
                  <Button size="sm" onClick={() => void act('resolve', () => api.gitResolve(projectId, f.path, 'ours'), 'Kept your version')}>
                    Keep mine
                  </Button>
                  <Button size="sm" onClick={() => void act('resolve', () => api.gitResolve(projectId, f.path, 'theirs'), 'Took incoming version')}>
                    Take theirs
                  </Button>
                  <Button
                    size="sm"
                    icon={<Bot size={12} />}
                    loading={busy === `ai:${f.path}`}
                    onClick={() =>
                      void act(`ai:${f.path}`, async () => {
                        const cs = await api.proposeConflictResolution(projectId, f.path);
                        openTab(`review:${cs.id}`);
                      })
                    }
                  >
                    AI merge
                  </Button>
                </div>
              </div>
            ))}
            {st.merging && (
              <div style={{ padding: '6px 10px 10px' }}>
                <Button size="sm" variant="danger" onClick={() => window.confirm('Abort the merge and return to the state before pulling?') && void act('abort', () => api.gitMergeAbort(projectId), 'Merge aborted')}>
                  Abort merge
                </Button>
              </div>
            )}
          </div>
        )}

        <div className="git-section" style={{ padding: 10 }}>
          <textarea className="textarea" style={{ minHeight: 56 }} placeholder="Commit message" value={message} onChange={(e) => setMessage(e.target.value)} aria-label="Commit message" />
          <div className="row" style={{ marginTop: 6 }}>
            <Button size="sm" variant="ghost" icon={<Sparkles size={12} />} loading={busy === 'suggest'} disabled={staged.length === 0} onClick={() => void suggest()}>
              Suggest
            </Button>
            <span className="spacer" />
            <Button size="sm" variant="primary" icon={<GitCommit size={13} />} loading={busy === 'commit'} disabled={staged.length === 0 || !message.trim() || !view.identity} onClick={() => void commit(false)}>
              Commit {staged.length > 0 && `(${staged.length})`}
            </Button>
          </div>
          {findings && (
            <Callout tone="bad" title="Possible secrets in staged files">
              {findings.map((f) => `${f.path}${f.line ? `:${f.line}` : ''} — ${f.kind}`).join('\n')}
              <div className="row" style={{ marginTop: 6 }}>
                <Button size="sm" onClick={() => setFindings(null)}>
                  Cancel
                </Button>
                <Button size="sm" variant="danger" onClick={() => void commit(true)}>
                  Not secrets — commit anyway
                </Button>
              </div>
            </Callout>
          )}
        </div>

        <div className="git-section">
          <div className="git-head">
            <span className="grow">Staged ({staged.length})</span>
            {staged.length > 0 && (
              <button className="btn ghost sm" onClick={() => void act('stage', () => api.gitStage(projectId, staged.map((f) => f.path), false))}>
                Unstage all
              </button>
            )}
          </div>
          {staged.map((f) => fileRow(f, true))}
        </div>
        <div className="git-section">
          <div className="git-head">
            <span className="grow">Changes ({changes.length})</span>
            {geniusUnstaged.length > 0 && (
              <button className="btn ghost sm" title="Stage only files Genius changed since the last commit" onClick={() => void act('stage', () => api.gitStage(projectId, geniusUnstaged, true))}>
                Stage Genius changes
              </button>
            )}
            {changes.length > 0 && (
              <button className="btn ghost sm" onClick={() => void act('stage', () => api.gitStage(projectId, changes.map((f) => f.path), true))}>
                Stage all
              </button>
            )}
          </div>
          {changes.map((f) => fileRow(f, false))}
          {changes.length === 0 && staged.length === 0 && <div className="faint small" style={{ padding: '4px 10px 10px' }}>Working tree clean.</div>}
        </div>
        <div className="git-section">
          <div className="git-head">History</div>
          {log.length === 0 && <div className="faint small" style={{ padding: '4px 10px 10px' }}>No commits yet.</div>}
          {log.map((c) => (
            <button key={c.sha} className="tree-row" style={{ height: 'auto', padding: '4px 10px', flexDirection: 'column', alignItems: 'flex-start' }} onClick={() => openTab(`commit:${c.sha}`)}>
              <span className="ellipsis small" style={{ width: '100%', color: 'var(--text)' }}>
                {c.subject}
              </span>
              <span className="faint tiny">
                <span className="mono">{c.short}</span> · {c.author} · {timeAgo(c.date)}
              </span>
            </button>
          ))}
        </div>
      </div>
      {diff && (
        <Modal title={diff.path} onClose={() => setDiff(null)} wide>
          <div className="card" style={{ maxHeight: '60vh', overflow: 'auto' }}>
            <DiffText diff={diff.text} />
          </div>
        </Modal>
      )}
      {newBranch !== null && (
        <Modal
          title="Create branch"
          onClose={() => setNewBranch(null)}
          footer={
            <>
              <Button onClick={() => setNewBranch(null)}>Cancel</Button>
              <Button variant="primary" disabled={!newBranch.trim()} onClick={() => void act('branch', () => api.gitBranch(projectId, newBranch.trim(), true), `Created ${newBranch}`).then(() => setNewBranch(null))}>
                Create and switch
              </Button>
            </>
          }
        >
          <input className="input mono" placeholder="feature/my-change" value={newBranch} onChange={(e) => setNewBranch(e.target.value)} aria-label="Branch name" />
        </Modal>
      )}
    </>
  );
}
