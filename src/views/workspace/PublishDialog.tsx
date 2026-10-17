import { useCallback, useEffect, useState } from 'react';
import { CheckCircle2, ExternalLink, Globe, UploadCloud } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Github } from '../../components/icons';
import { api, errorMessage } from '../../lib/api';
import type { DeployStatus, PublishPlan, PublishRequest } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout, Field, Modal, Segmented, Spinner } from '../../components/ui';

/**
 * One-click publish: shows exactly what will be published and where, then initialises Git,
 * commits the reviewed files, creates (or reuses) the GitHub repository, pushes and verifies
 * the remote branch. Each step is persisted, so a retry continues where it stopped.
 */
export default function PublishDialog({ onClose }: { onClose: () => void }) {
  const project = useWorkspace((s) => s.project)!;
  const navigate = useApp((s) => s.navigate);
  const [plan, setPlan] = useState<PublishPlan | null>(null);
  const [orgs, setOrgs] = useState<string[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<{ url: string; repo: string; branch: string } | null>(null);
  const [req, setReq] = useState<PublishRequest>({ owner: '', name: '', visibility: 'private', description: project.spec?.purpose ?? project.description, commitMessage: 'Initial commit', acknowledgeFindings: false });
  const [visibilityChosen, setVisibilityChosen] = useState(false);
  const [showFiles, setShowFiles] = useState(false);

  useEffect(() => {
    api
      .publishPlan(project.id)
      .then((p) => {
        setPlan(p);
        const prevRepo = p.previous?.repo?.split('/');
        setReq((r) => ({ ...r, owner: prevRepo?.[0] ?? p.gh.login ?? '', name: prevRepo?.[1] ?? p.suggestedName, visibility: (p.previous?.visibility as PublishRequest['visibility']) ?? r.visibility }));
        if (p.previous) setVisibilityChosen(true);
        if (p.gh.connected) api.githubOrgs().then(setOrgs).catch(() => undefined);
      })
      .catch((e) => setErr(errorMessage(e)));
  }, [project.id]);

  const publish = async () => {
    setBusy(true);
    setErr(null);
    try {
      setDone(await api.publish(project.id, req));
    } catch (e) {
      setErr(errorMessage(e));
      api.publishPlan(project.id).then(setPlan).catch(() => undefined);
    } finally {
      setBusy(false);
    }
  };

  if (done) {
    return (
      <Modal title="Published to GitHub" icon={<CheckCircle2 size={18} color="var(--ok)" />} onClose={onClose} footer={<Button onClick={onClose}>Close</Button>}>
        <Callout tone="ok" title="Push verified">
          Branch {done.branch} on {done.repo} now points at your latest local commit (checked with git ls-remote).
        </Callout>
        <Button icon={<ExternalLink size={14} />} onClick={() => void openUrl(done.url)}>
          Open {done.url}
        </Button>
        <DeploySection projectId={project.id} published />
      </Modal>
    );
  }

  const blocked = !plan || !plan.gitAvailable || !plan.gh.connected;
  const hasFindings = (plan?.findings.length ?? 0) > 0;
  return (
    <Modal
      title="Publish to GitHub"
      icon={<Github size={18} />}
      onClose={onClose}
      wide
      footer={
        <>
          <span className="faint small spacer">You will be asked to approve the push.</span>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" icon={<UploadCloud size={14} />} loading={busy} disabled={blocked || !req.owner || !req.name || !visibilityChosen || (hasFindings && !req.acknowledgeFindings)} onClick={() => void publish()}>
            {plan?.previous && plan.previous.step !== 'pushed' ? 'Retry publish' : 'Publish'}
          </Button>
        </>
      }
    >
      {!plan && !err && (
        <div className="row muted">
          <Spinner /> Inspecting the project and Git status…
        </div>
      )}
      {err && <Callout tone="bad" title="Publishing stopped">{err}</Callout>}
      {plan && !plan.gitAvailable && <Callout tone="bad">Git is not installed. Install it from git-scm.com and restart Genius.</Callout>}
      {plan && plan.gitAvailable && !plan.gh.connected && (
        <Callout tone="warn" title="GitHub is not connected">
          {plan.gh.detail}{' '}
          <a href="#" onClick={(e) => (e.preventDefault(), navigate({ name: 'settings', tab: 'github' }))}>
            Connect in Settings → GitHub
          </a>
        </Callout>
      )}
      {plan?.previous && plan.previous.step !== 'pushed' && (
        <Callout tone="info" title="A previous publish did not finish">
          Last completed step: {plan.previous.step.replace('_', ' ')} for {plan.previous.repo}. Retrying reuses the existing repository instead of creating a duplicate.
        </Callout>
      )}
      {plan && (
        <>
          <div className="row" style={{ alignItems: 'flex-start', gap: 14 }}>
            <Field label="Owner">
              <select className="select" value={req.owner} onChange={(e) => setReq({ ...req, owner: e.target.value })}>
                {plan.gh.login && <option value={plan.gh.login}>{plan.gh.login} (you)</option>}
                {orgs.map((o) => (
                  <option key={o} value={o}>
                    {o}
                  </option>
                ))}
              </select>
            </Field>
            <div className="grow">
              <Field label="Repository name">
                <input className="input mono" value={req.name} onChange={(e) => setReq({ ...req, name: e.target.value })} />
              </Field>
            </div>
          </div>
          <Field label="Visibility" hint={!visibilityChosen ? 'Choose explicitly — Genius never assumes a repository should be public.' : req.visibility === 'public' ? 'Anyone on the internet will be able to see this code.' : 'Only you and people you grant access can see it.'}>
            <Segmented
              label="Visibility"
              value={visibilityChosen ? req.visibility : ('' as PublishRequest['visibility'])}
              onChange={(v) => {
                setReq({ ...req, visibility: v });
                setVisibilityChosen(true);
              }}
              options={[
                { value: 'private', label: 'Private' },
                { value: 'public', label: 'Public' },
              ]}
            />
          </Field>
          <Field label="Description">
            <input className="input" value={req.description} onChange={(e) => setReq({ ...req, description: e.target.value })} />
          </Field>
          <Field label="Commit message" hint={plan.isRepo ? `${plan.uncommitted} uncommitted file(s) will be committed on branch ${plan.branch}.` : 'A new Git repository (branch main) will be initialised.'}>
            <input className="input" value={req.commitMessage} onChange={(e) => setReq({ ...req, commitMessage: e.target.value })} />
          </Field>
          <div className="card pad col" style={{ gap: 8 }}>
            <div className="row">
              <strong className="grow">
                Target: <span className="mono">{req.owner || '?'}/{req.name || '?'}</span>
              </strong>
              {visibilityChosen && <Badge tone={req.visibility === 'public' ? 'warn' : 'ok'}>{req.visibility}</Badge>}
            </div>
            <div className="row small muted">
              {plan.files.length} files will be published · {plan.excludedByGitignore.length} excluded by .gitignore
              <button className="btn ghost sm" onClick={() => setShowFiles(!showFiles)}>
                {showFiles ? 'Hide files' : 'Review files'}
              </button>
            </div>
            {plan.gitignoreMissing.length > 0 && <div className="small muted">.gitignore will be extended with: {plan.gitignoreMissing.join(', ')} (existing rules are kept).</div>}
            {showFiles && (
              <pre className="block" style={{ maxHeight: 180 }}>
                {plan.files.join('\n')}
              </pre>
            )}
          </div>
          <DeploySection projectId={project.id} published={!!plan.remoteUrl?.includes('github.com')} />
          {hasFindings && (
            <Callout tone="bad" title="Possible secrets detected">
              {plan.findings.map((f) => `${f.path}${f.line ? `:${f.line}` : ''} — ${f.kind}`).join('\n')}
              <label className="checkbox" style={{ marginTop: 8 }}>
                <input type="checkbox" checked={req.acknowledgeFindings} onChange={(e) => setReq({ ...req, acknowledgeFindings: e.target.checked })} />I reviewed these and they are not real secrets
              </label>
            </Callout>
          )}
        </>
      )}
    </Modal>
  );
}

const DEPLOY_STEPS: Record<string, string> = {
  building: 'Building your app for the web…',
  uploading: 'Uploading the website to GitHub (approve the push when asked)…',
  enabling: 'Turning on GitHub Pages…',
};

/**
 * One-click website: builds the app and serves it with GitHub Pages from the repository's
 * gh-pages branch. Progress and the live link come from the persisted deploy status.
 */
export function DeploySection({ projectId, published }: { projectId: string; published: boolean }) {
  const [st, setSt] = useState<DeployStatus | null>(null);
  const [starting, setStarting] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const refresh = useCallback(() => api.deployStatus(projectId).then(setSt).catch(() => undefined), [projectId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);
  const running = starting || !!st?.running;
  useEffect(() => {
    if (!running) return;
    const t = setInterval(() => void refresh(), 1500);
    return () => clearInterval(t);
  }, [running, refresh]);

  const deploy = async () => {
    setStarting(true);
    setErr(null);
    try {
      await api.deployPages(projectId);
      await refresh();
    } catch (e) {
      setErr(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const live = st?.url;
  return (
    <div className="card pad col" style={{ gap: 8 }}>
      <div className="row">
        <Globe size={16} />
        <strong className="grow">Website</strong>
        {st?.step === 'deployed' && !running && <Badge tone="ok">live</Badge>}
        <Button size="sm" variant={live ? 'secondary' : 'primary'} icon={<UploadCloud size={13} />} loading={running} disabled={!published || running} onClick={() => void deploy()}>
          {live ? 'Redeploy' : 'Deploy website'}
        </Button>
      </div>
      <div className="small muted">
        {published
          ? 'Builds the app and publishes it with GitHub Pages (branch gh-pages of your repository). Your code and branches are not changed.'
          : 'Publish the project to GitHub first; the website is served from that repository with GitHub Pages.'}
      </div>
      {running && (
        <div className="row small">
          <Spinner size={12} /> {DEPLOY_STEPS[st?.step ?? 'building'] ?? 'Starting…'}
        </div>
      )}
      {!running && st?.step === 'failed' && st.error && <Callout tone="bad" title="The website was not deployed">{st.error}</Callout>}
      {err && <Callout tone="bad">{err}</Callout>}
      {live && (
        <div className="row small">
          <a
            href={live}
            className="mono"
            onClick={(e) => {
              e.preventDefault();
              void openUrl(live);
            }}
          >
            {live}
          </a>
          {st?.step === 'deployed' && !running && <span className="faint">GitHub can take a minute to show a new version.</span>}
        </div>
      )}
    </div>
  );
}
