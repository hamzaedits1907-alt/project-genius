import { useCallback, useEffect, useState } from 'react';
import { Database, RefreshCw } from 'lucide-react';
import { Github } from '../../components/icons';
import { api, errorMessage } from '../../lib/api';
import type { AuthStatus, SupabaseProject } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Badge, Button, Callout } from '../../components/ui';

type SbStatus = {
  connected: boolean;
  linked: { ref: string; name?: string; url: string } | null;
  remote: { ok: boolean; status?: string; error?: string } | null;
  envConfigured: boolean;
  clientLibrary: boolean;
  envGitignored: boolean;
  serviceRoleMentions: string[];
  manualSteps: string[];
};

/** Per-project view of connected services. Every status shown comes from a real check. */
export default function IntegrationsPanel({ onPublish }: { onPublish: () => void }) {
  const project = useWorkspace((s) => s.project);
  const refreshProject = useWorkspace((s) => s.refreshProject);
  const openTab = useWorkspace((s) => s.openTab);
  const navigate = useApp((s) => s.navigate);
  const toast = useApp((s) => s.toast);
  const [gh, setGh] = useState<AuthStatus | null>(null);
  const [sb, setSb] = useState<SbStatus | null>(null);
  const [projects, setProjects] = useState<SupabaseProject[] | null>(null);
  const [migrations, setMigrations] = useState<{ path: string; appliedRemote: string | null }[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [migName, setMigName] = useState('');
  const [migSql, setMigSql] = useState('');
  const pid = project?.id;

  const load = useCallback(async () => {
    if (!pid) return;
    const [g, s, m] = await Promise.all([api.githubStatus(), api.supabaseStatus(pid) as Promise<unknown> as Promise<SbStatus>, api.supabaseMigrations(pid)]);
    setGh(g);
    setSb(s);
    setMigrations(m);
  }, [pid]);
  useEffect(() => {
    load().catch((e) => toast('bad', errorMessage(e)));
  }, [load, toast]);
  if (!project || !pid) return null;

  const run = async (name: string, fn: () => Promise<unknown>, ok?: string) => {
    setBusy(name);
    try {
      const r = await fn();
      if (ok) toast('ok', ok);
      await refreshProject();
      await load();
      return r;
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <div className="panel-head">
        <span className="panel-title grow">Integrations</span>
        <button className="icon-btn" aria-label="Refresh integrations" onClick={() => void load()}>
          <RefreshCw size={13} />
        </button>
      </div>
      <div className="scroll grow col" style={{ padding: 10, gap: 14 }}>
        <section className="col" style={{ gap: 6 }}>
          <div className="row" style={{ gap: 6 }}>
            <Github size={15} />
            <strong className="grow">GitHub</strong>
            {gh && <Badge tone={gh.connected ? 'ok' : 'bad'}>{gh.connected ? `connected as ${gh.login}` : 'not connected'}</Badge>}
          </div>
          {gh && !gh.connected && <div className="small faint">{gh.detail}</div>}
          {project.status.publish && (
            <div className="small">
              Published to <span className="mono">{project.status.publish.repo}</span> — {project.status.publish.verified ? 'push verified on GitHub' : `last step: ${project.status.publish.step}`}
            </div>
          )}
          <div className="row">
            {gh?.connected ? (
              <Button size="sm" variant="primary" onClick={onPublish}>
                Publish / push…
              </Button>
            ) : (
              <Button size="sm" variant="primary" onClick={() => navigate({ name: 'settings', tab: 'github' })}>
                Connect GitHub
              </Button>
            )}
          </div>
        </section>

        <section className="col" style={{ gap: 6 }}>
          <div className="row" style={{ gap: 6 }}>
            <Database size={15} />
            <strong className="grow">Supabase</strong>
            {sb && <Badge tone={sb.connected ? (sb.linked ? 'ok' : 'warn') : 'bad'}>{sb.connected ? (sb.linked ? `linked: ${sb.linked.name ?? sb.linked.ref}` : 'connected, not linked') : 'not connected'}</Badge>}
          </div>
          {sb && !sb.connected && (
            <>
              <div className="small faint">Connect your Supabase account first (personal access token, stored in the OS credential store).</div>
              <Button size="sm" onClick={() => navigate({ name: 'settings', tab: 'integrations' })}>
                Connect Supabase
              </Button>
            </>
          )}
          {sb?.connected && !sb.linked && (
            <>
              <Button size="sm" loading={busy === 'list'} onClick={() => void run('list', () => api.supabaseProjects().then(setProjects))}>
                Choose a Supabase project…
              </Button>
              {projects?.length === 0 && <Callout tone="warn">Your account has no Supabase projects. Create one at supabase.com/dashboard, then refresh.</Callout>}
              {projects?.map((p) => (
                <button key={p.reference} className="btn sm" style={{ justifyContent: 'flex-start' }} disabled={!!busy} onClick={() => void run('link', () => api.supabaseLink(pid, p.reference), `Linked ${p.name}`)}>
                  {p.name} <span className="faint mono">{p.reference}</span> <Badge>{p.status}</Badge>
                </button>
              ))}
            </>
          )}
          {sb?.connected && sb.linked && (
            <>
              <ul className="small" style={{ margin: 0, paddingLeft: 18 }}>
                <li>
                  Remote project:{' '}
                  {sb.remote?.ok ? <Badge tone="ok">{sb.remote.status}</Badge> : <span className="bad-text">{sb.remote?.error ?? 'not checked'}</span>}
                </li>
                <li>Client config (.env.local): {sb.envConfigured ? <Badge tone="ok">configured</Badge> : <Badge tone="warn">not configured</Badge>}</li>
                <li>.env files ignored by Git: {sb.envGitignored ? <Badge tone="ok">yes</Badge> : <Badge tone="bad">no</Badge>}</li>
                <li>Client library: {sb.clientLibrary ? <Badge tone="ok">in package.json</Badge> : <Badge tone="warn">not added</Badge>}</li>
                {sb.serviceRoleMentions.length > 0 && <li className="bad-text">“service_role” appears in: {sb.serviceRoleMentions.join(', ')} — never ship the service-role key to the browser.</li>}
              </ul>
              <div className="row" style={{ flexWrap: 'wrap' }}>
                <Button
                  size="sm"
                  loading={busy === 'cfg'}
                  onClick={() =>
                    void run('cfg', async () => {
                      const r = await api.supabaseConfigure(pid);
                      if (r.changesetId) openTab(`review:${r.changesetId}`);
                      return r;
                    }, 'Wrote .env.local (gitignored). Review the proposed client code.')
                  }
                >
                  Configure client
                </Button>
                <Button
                  size="sm"
                  loading={busy === 'types'}
                  onClick={() =>
                    void run('types', async () => {
                      const r = await api.supabaseGenerateTypes(pid);
                      if (r.changesetId) openTab(`review:${r.changesetId}`);
                      return r;
                    })
                  }
                >
                  Generate types
                </Button>
              </div>
              <div className="small faint">Manual steps in the Supabase dashboard: {sb.manualSteps.join(' ')}</div>
              <strong className="small">Migrations</strong>
              {migrations.length === 0 && <div className="faint small">No files in supabase/migrations yet. Ask Genius (“create a profiles table with RLS”) or add one below.</div>}
              {migrations.map((m) => (
                <div key={m.path} className="row small" style={{ gap: 6 }}>
                  <span className="mono grow ellipsis">{m.path.replace('supabase/migrations/', '')}</span>
                  {m.appliedRemote ? (
                    <Badge tone="ok">applied</Badge>
                  ) : (
                    <Button size="sm" variant="danger" loading={busy === m.path} onClick={() => void run(m.path, () => api.supabaseApplyMigration(pid, m.path), 'Migration applied to the remote database')}>
                      Apply to remote…
                    </Button>
                  )}
                </div>
              ))}
              <input className="input sm" placeholder="Migration name (e.g. create profiles)" value={migName} onChange={(e) => setMigName(e.target.value)} aria-label="Migration name" />
              <textarea className="textarea mono" style={{ minHeight: 80, fontSize: 11.5 }} placeholder="SQL" value={migSql} onChange={(e) => setMigSql(e.target.value)} aria-label="Migration SQL" />
              <Button
                size="sm"
                disabled={!migName.trim() || !migSql.trim()}
                onClick={() =>
                  void run('mig', async () => {
                    const r = await api.supabaseProposeMigration(pid, migName, migSql);
                    openTab(`review:${r.changesetId}`);
                    setMigName('');
                    setMigSql('');
                  })
                }
              >
                Propose migration file
              </Button>
              <div className="faint tiny">Applying to the remote database always asks for your approval with the exact SQL, whatever your approval settings.</div>
            </>
          )}
        </section>
      </div>
    </>
  );
}
