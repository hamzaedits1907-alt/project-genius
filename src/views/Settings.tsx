import { useEffect, useMemo, useState } from 'react';
import { ArrowDownCircle, CheckCircle2, Copy, Globe, Plug, ExternalLink, FolderCog, KeyRound, Palette, RefreshCw, ShieldCheck, Sparkles, XCircle } from 'lucide-react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Github } from '../components/icons';
import { api, errorMessage } from '../lib/api';
import { timeAgo } from '../lib/logic';
import type { AuthStatus, Effort, ModelList, ModelMode, ModelRole, Policy, ResolvedModels, Settings, ToolStatus } from '../lib/types';
import { useApp, type SettingsTab } from '../store/app';
import { Badge, Button, Callout, Field, Segmented, Spinner, Switch } from '../components/ui';
import { GeniusAvatar } from '../components/GeniusAvatar';
import { DiagnosticsCard, KEY_LINK, KEY_PLACEHOLDER, LimitsCard, ProviderCard, RelayConsentCard } from './ProviderPanels';
import { isWeb } from '../lib/platform';
import { extraViews } from '../lib/extraViews';

const TABS: { id: SettingsTab; label: string; icon: React.ReactNode }[] = [
  { id: 'ai', label: 'AI provider', icon: <Sparkles size={15} /> },
  { id: 'github', label: 'GitHub', icon: <Github size={15} /> },
  { id: 'integrations', label: 'Integrations', icon: <Plug size={15} /> },
  { id: 'approvals', label: 'Approvals', icon: <ShieldCheck size={15} /> },
  { id: 'workspace', label: 'Workspace & tools', icon: <FolderCog size={15} /> },
  { id: 'appearance', label: 'Appearance', icon: <Palette size={15} /> },
  { id: 'updates', label: 'Updates & about', icon: <ArrowDownCircle size={15} /> },
  { id: 'account', label: 'Account', icon: <KeyRound size={15} /> },
];
/** Tabs that need a local computer (GitHub CLI, project folders, updates) are not offered in the browser. */
const WEB_TABS: SettingsTab[] = ['ai', 'appearance', 'account'];
const visibleTabs = () => TABS.filter((t) => (isWeb ? WEB_TABS.includes(t.id) : t.id !== 'account'));

export default function SettingsView({ initialTab }: { initialTab?: SettingsTab }) {
  const [tab, setTab] = useState<SettingsTab>(initialTab ?? 'ai');
  const settings = useApp((s) => s.settings);
  return (
    <div className="page">
      <div className="page-inner">
        <h1>Settings</h1>
        <div className="settings">
          <nav aria-label="Settings sections">
            {visibleTabs().map((t) => (
              <button key={t.id} className={tab === t.id ? 'on' : ''} onClick={() => setTab(t.id)}>
                {t.icon}
                {t.label}
              </button>
            ))}
          </nav>
          {!settings ? (
            <Spinner />
          ) : (
            <div>
              {tab === 'ai' && <AiSettings key={settings.provider} />}
              {tab === 'github' && <GithubSettings />}
              {tab === 'integrations' && <IntegrationSettings />}
              {tab === 'approvals' && <ApprovalSettings />}
              {tab === 'workspace' && <WorkspaceSettings />}
              {tab === 'appearance' && <AppearanceSettings />}
              {tab === 'updates' && <UpdateSettings />}
              {tab === 'account' && extraViews.account && (() => { const A = extraViews.account; return <A />; })()}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function useSave() {
  const setSettings = useApp((s) => s.setSettings);
  const toast = useApp((s) => s.toast);
  return async (next: Settings, quiet = false) => {
    try {
      const saved = await api.saveSettings(next);
      setSettings(saved);
      if (!quiet) toast('ok', 'Settings saved');
      return true;
    } catch (e) {
      toast('bad', errorMessage(e));
      return false;
    }
  };
}

// ------------------------------------------------------------------ AI provider

function AiSettings() {
  const settings = useApp((s) => s.settings)!;
  const keyStatus = useApp((s) => s.keyStatus);
  const setKeyStatus = useApp((s) => s.setKeyStatus);
  const setSettings = useApp((s) => s.setSettings);
  const toast = useApp((s) => s.toast);
  const save = useSave();
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [models, setModels] = useState<ModelList | null>(null);
  const [modelErr, setModelErr] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [showAll, setShowAll] = useState(false);
  const [baseUrl, setBaseUrl] = useState(settings.baseUrl);
  const [params, setParams] = useState({ temperature: settings.temperature, maxTokens: settings.maxTokens, contextBudgetChars: settings.contextBudgetChars });

  const loadModels = async (refresh: boolean) => {
    setBusy(refresh ? 'models' : null);
    setModelErr(null);
    try {
      setModels(await api.listModels(refresh));
    } catch (e) {
      setModelErr(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  useEffect(() => {
    void loadModels(false);
  }, []);

  const saveKey = async () => {
    setBusy('key');
    try {
      const st = await api.saveApiKey(key);
      setKeyStatus(st);
      setKey('');
      toast('ok', `Key saved to ${st.backend}`);
      void loadModels(true);
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const removeKey = async () => {
    setBusy('remove');
    try {
      setKeyStatus(await api.removeApiKey());
      toast('ok', 'Key removed');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const select = async (id: string) => {
    try {
      setSettings(await api.selectModel(id));
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const verify = async (id: string) => {
    setBusy(`probe:${id}`);
    try {
      const p = await api.probeModel(id);
      toast('ok', `${id} responded in ${p.latencyMs} ms${p.streaming ? ' (streaming)' : ''}`);
      void loadModels(false);
    } catch (e) {
      toast('bad', `${id}: ${errorMessage(e)}`);
    } finally {
      setBusy(null);
    }
  };

  const auto = settings.modelMode !== 'manual' && settings.provider === 'nvidia';
  const setMode = (modelMode: ModelMode) => void save({ ...settings, modelMode }, true);

  const shown = useMemo(() => {
    const list = models?.models ?? [];
    return list.filter((m) => (showAll || m.inferredCategory === 'chat' || m.inferredCategory === 'code') && m.id.toLowerCase().includes(filter.toLowerCase()));
  }, [models, filter, showAll]);

  return (
    <section>
      <div>
        <h2>AI provider</h2>
        <p className="muted small" style={{ margin: '4px 0 0' }}>
          Genius works with your own account at the provider you choose.
          {KEY_LINK[settings.provider] && (
            <>
              {' '}Get a key at{' '}
              <a href="#" onClick={(e) => (e.preventDefault(), void openUrl(KEY_LINK[settings.provider]!))}>
                {KEY_LINK[settings.provider]!.replace('https://', '').replace(/\/$/, '')}
              </a>
              .
            </>
          )}
        </p>
      </div>

      <ProviderCard settings={settings} save={save} />

      {isWeb && <RelayConsentCard settings={settings} save={save} />}

      <InternetCard />

      <div className="card pad col" style={{ gap: 12 }}>
        <div className="row">
          <KeyRound size={16} />
          <strong className="grow">API key</strong>
          {keyStatus?.stored ? <Badge tone="ok">Saved {keyStatus.hint}</Badge> : <Badge tone="warn">Not set</Badge>}
        </div>
        {keyStatus?.error && <Callout tone="bad">{keyStatus.error}</Callout>}
        <div className="row">
          <input
            className="input mono grow"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={keyStatus?.stored ? 'Enter a new key to replace the saved one' : KEY_PLACEHOLDER[settings.provider]}
            value={key}
            onChange={(e) => setKey(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && key && void saveKey()}
            aria-label="API key"
          />
          <Button variant="primary" disabled={!key.trim()} loading={busy === 'key'} onClick={() => void saveKey()}>
            Save key
          </Button>
          {keyStatus?.stored && (
            <Button variant="danger" loading={busy === 'remove'} onClick={() => void removeKey()}>
              Remove
            </Button>
          )}
        </div>
        <div className="faint small">
          Stored in: {keyStatus?.backend ?? '…'}. The key never returns to this window, is never written to project files or logs, and is never exposed to previewed apps.
        </div>
      </div>

      <DiagnosticsCard keyStored={!!keyStatus?.stored} onDone={() => void loadModels(false)} />

      <div className="card pad col" style={{ gap: 12 }}>
        <div className="row">
          <Sparkles size={16} />
          <strong className="grow">Model</strong>
          {auto ? <Badge tone="accent">Auto</Badge> : settings.model ? <Badge tone="accent">Selected: {settings.model}</Badge> : <Badge tone="warn">None selected</Badge>}
        </div>
        <div className="col" style={{ gap: 6 }} role="radiogroup" aria-label="How Genius chooses the model">
          {settings.provider === 'nvidia' && (
          <label className={`choice ${auto ? 'on' : ''}`}>
            <input type="radio" name="model-mode" checked={auto} onChange={() => setMode('auto')} />
            <span>
              <strong>Auto (recommended)</strong> — Genius picks the best available model for each job
              <span className="faint small" style={{ display: 'block' }}>Only models your key can use are considered. If one is unavailable, busy or failing, Genius switches to the next best and tells you.</span>
            </span>
          </label>
          )}
          <label className={`choice ${!auto ? 'on' : ''}`}>
            <input type="radio" name="model-mode" checked={!auto} onChange={() => setMode('manual')} />
            <span>
              <strong>Choose a model</strong>
              <span className="faint small" style={{ display: 'block' }}>Always use the model you pick below.</span>
            </span>
          </label>
        </div>
        {auto ? (
          <AutoModels version={`${models?.fetchedAt ?? ''}|${settings.effort}|${settings.model ?? ''}`} onRefresh={() => void loadModels(true)} refreshing={busy === 'models'} count={models?.models.length} />
        ) : (
        <>
        <div className="row">
          <input className="input grow" placeholder="Search models" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Search models" />
          <label className="checkbox small">
            <input type="checkbox" checked={showAll} onChange={(e) => setShowAll(e.target.checked)} />
            Show non-chat models
          </label>
          <Button icon={<RefreshCw size={14} />} loading={busy === 'models'} onClick={() => void loadModels(true)}>
            Refresh models
          </Button>
        </div>
        {modelErr && <Callout tone="bad">{modelErr}</Callout>}
        {models && (
          <div className="faint small">
            {models.source === 'live' ? 'Fetched live' : 'From cache'} from {models.baseUrl}/models {timeAgo(models.fetchedAt)}
            {models.stale && <Badge tone="warn">may be out of date — refresh</Badge>} · {models.models.length} models. Categories are inferred from model names; use Verify to confirm a model answers chat requests with your key.
          </div>
        )}
        {!models && !modelErr && <div className="faint small">No model list yet. Save a key and press Refresh models.</div>}
        {models && (
          <div className="model-list" role="listbox" aria-label="Available models">
            {shown.map((m) => {
              const probe = models.probes[m.id];
              const on = settings.model === m.id;
              return (
                <div key={m.id} className={`model-row ${on ? 'on' : ''}`} role="option" aria-selected={on} tabIndex={0} onClick={() => void select(m.id)} onKeyDown={(e) => e.key === 'Enter' && void select(m.id)}>
                  <div className="col" style={{ gap: 0, minWidth: 0 }}>
                    <span className="mono ellipsis">{m.id}</span>
                    <span className="faint tiny">
                      {m.owner ?? 'unknown provider'} · {m.inferredCategory} (inferred)
                    </span>
                  </div>
                  {probe ? (
                    <Badge tone="ok" title={`Verified ${probe.latencyMs} ms`}>
                      verified{probe.streaming ? ' · streams' : ''}
                    </Badge>
                  ) : (
                    <Button
                      size="sm"
                      variant="ghost"
                      loading={busy === `probe:${m.id}`}
                      disabled={!keyStatus?.stored}
                      onClick={(e) => {
                        e.stopPropagation();
                        void verify(m.id);
                      }}
                    >
                      Verify
                    </Button>
                  )}
                  {on ? <CheckCircle2 size={16} color="var(--accent-2)" /> : <span style={{ width: 16 }} />}
                </div>
              );
            })}
            {shown.length === 0 && <div className="faint small" style={{ padding: 12 }}>No models match.</div>}
          </div>
        )}
        </>
        )}
        {auto && modelErr && <Callout tone="bad">{modelErr}</Callout>}
      </div>

      <EffortCard />

      {!auto && <VisionCheck model={settings.model} />}

      <LimitsCard settings={settings} save={save} />

      <div className="card pad col" style={{ gap: 14 }}>
        <strong>Generation</strong>
        <div className="row" style={{ alignItems: 'flex-start', gap: 14 }}>
          <Field label="Temperature" hint="Lower is more deterministic.">
            <input className="input" type="number" step={0.1} min={0} max={2} value={params.temperature} onChange={(e) => setParams({ ...params, temperature: Number(e.target.value) })} />
          </Field>
          <Field label="Max output tokens" hint="Long generations continue automatically.">
            <input className="input" type="number" min={256} max={131072} value={params.maxTokens} onChange={(e) => setParams({ ...params, maxTokens: Number(e.target.value) })} />
          </Field>
          <Field label="Context budget (characters)" hint="Max project source sent per request.">
            <input className="input" type="number" min={4000} max={400000} step={1000} value={params.contextBudgetChars} onChange={(e) => setParams({ ...params, contextBudgetChars: Number(e.target.value) })} />
          </Field>
        </div>
        <Field label="API base URL" hint={settings.provider === 'custom' ? 'The address of your OpenAI-compatible server (…/v1).' : 'Normally leave this alone. Use http://localhost:… only for a self-hosted server.'}>
          <input className="input mono" value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} />
        </Field>
        <div>
          <Button variant="primary" onClick={() => void save({ ...settings, ...params, baseUrl }).then((ok) => ok && baseUrl !== settings.baseUrl && setModels(null))}>
            Save generation settings
          </Button>
        </div>
      </div>
    </section>
  );
}

const ROLE_LABELS: [ModelRole, string][] = [
  ['agent', 'Conversations, code changes & Build Doctor'],
  ['code', 'Generating whole apps'],
  ['plan', 'Planning'],
  ['subagent', 'Helper analysts (review, design, tests)'],
  ['quick', 'Quick tasks (commit messages, prompt help)'],
  ['vision', 'Reading images (screenshots)'],
];

/** Read-only view of what auto mode resolves to for each job, best model first. */
function AutoModels({ version, onRefresh, refreshing, count }: { version: string; onRefresh: () => void; refreshing: boolean; count?: number }) {
  const [resolved, setResolved] = useState<ResolvedModels | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    api
      .resolvedModels()
      .then((r) => live && (setResolved(r), setErr(null)))
      .catch((e) => live && setErr(errorMessage(e)));
    return () => {
      live = false;
    };
  }, [version]);
  return (
    <div className="col" style={{ gap: 8 }}>
      <div className="row">
        <strong className="small grow">Genius will use</strong>
        <span className="faint tiny">{count != null ? `${count} models available to your key` : ''}</span>
        <Button size="sm" icon={<RefreshCw size={13} />} loading={refreshing} onClick={onRefresh}>
          Refresh models
        </Button>
      </div>
      {err && <Callout tone="warn">{err}</Callout>}
      {!resolved && !err && <Spinner />}
      {resolved && (
        <table className="auto-models" aria-label="Models Genius will use">
          <tbody>
            {ROLE_LABELS.map(([role, label]) => {
              const chain = resolved[role] ?? [];
              return (
                <tr key={role}>
                  <td className="small">{label}</td>
                  <td>
                    {chain.length ? (
                      <>
                        <span className="mono small">{chain[0]}</span>
                        {chain.length > 1 && <span className="faint tiny" style={{ display: 'block' }}>then {chain.slice(1).join(', ')}</span>}
                      </>
                    ) : (
                      <span className="faint small">{role === 'vision' ? 'No vision model available — images cannot be read' : 'No model available yet — refresh the model list'}</span>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
      <div className="faint tiny">When the chosen model cannot see images, a vision model describes them in detail first.</div>
    </div>
  );
}

const EFFORTS: { value: Effort; label: string; hint: string }[] = [
  { value: 'fast', label: 'Fast', hint: 'Quickest answers: faster models, shorter replies, no thinking.' },
  { value: 'balanced', label: 'Balanced', hint: 'Strong models with a moderate output budget; quick jobs use fast models.' },
  { value: 'max', label: 'Max', hint: 'The best available model for every job, long replies and full reasoning.' },
];

function EffortCard() {
  const settings = useApp((s) => s.settings)!;
  const save = useSave();
  const current = EFFORTS.find((e) => e.value === settings.effort) ?? EFFORTS[2];
  return (
    <div className="card pad col" style={{ gap: 10 }}>
      <div className="row">
        <strong className="grow">Effort</strong>
        <Segmented label="Effort" value={settings.effort} options={EFFORTS.map((e) => ({ value: e.value, label: e.label }))} onChange={(effort) => void save({ ...settings, effort }, true)} />
      </div>
      <div className="small">{current.hint}</div>
      <ul className="faint tiny" style={{ margin: 0, paddingLeft: 18 }}>
        {EFFORTS.map((e) => (
          <li key={e.value}>
            <strong>{e.label}</strong> — {e.hint}
          </li>
        ))}
      </ul>
      <div className="row">
        <Switch label="Let the model think" checked={settings.thinking} disabled={settings.effort === 'fast'} onChange={(thinking) => void save({ ...settings, thinking }, true)} />
        <span className="grow small">
          Let the model think (shows its reasoning live)
          <span className="faint tiny" style={{ display: 'block' }}>{settings.effort === 'fast' ? 'Fast effort always answers without thinking.' : 'Reasoning models think before answering: slower, but better on hard problems.'}</span>
        </span>
      </div>
    </div>
  );
}

/** Image input is never assumed: it is enabled only after a real request with an image succeeds. */
function VisionCheck({ model }: { model: string | null }) {
  const toast = useApp((s) => s.toast);
  const [res, setRes] = useState<{ ok: boolean; answer: string; error: string | null } | null>(null);
  const [busy, setBusy] = useState(false);
  if (!model) return null;
  const run = async () => {
    setBusy(true);
    try {
      setRes(await api.probeVision(model));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="card pad col" style={{ gap: 8 }}>
      <strong>Image input (screenshots)</strong>
      <div className="small faint">Sends one small red test image to {model}. Screenshots can be attached only if this succeeds.</div>
      <div className="row">
        <Button size="sm" loading={busy} onClick={() => void run()}>
          Test image input
        </Button>
        {res && (res.ok ? <Badge tone="ok">accepted — model answered “{res.answer}”</Badge> : <Badge tone="bad">not supported{res.error ? `: ${res.error}` : ''}</Badge>)}
      </div>
    </div>
  );
}

// ------------------------------------------------------------------ Integrations

function IntegrationSettings() {
  const toast = useApp((s) => s.toast);
  const [items, setItems] = useState<Awaited<ReturnType<typeof api.integrationStatuses>> | null>(null);
  const [token, setToken] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const load = () => {
    api.integrationStatuses().then(setItems).catch((e) => toast('bad', errorMessage(e)));
  };
  useEffect(() => {
    load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const sb = items?.find((i) => i.id === 'supabase');
  const connect = async () => {
    setBusy('connect');
    try {
      const r = await api.supabaseConnect(token);
      setToken('');
      toast('ok', `Supabase connected — ${r.projects.length} project(s) visible`);
      load();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const disconnect = async () => {
    setBusy('disconnect');
    try {
      const r = await api.supabaseDisconnect();
      toast('info', r.note);
      load();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <section>
      <h2>Integrations</h2>
      <p className="small faint">Genius only uses services listed here as connected, and only for the operations each one declares. Tokens are kept in your OS credential store, never in projects, prompts or logs.</p>
      {!items && <Spinner />}
      {items?.map((i) => {
        const spec = i.spec as { authMethod: string; permissions: string[]; capabilities: string[]; credentialStorage: string; disconnect: string; docs: string };
        return (
          <div key={i.id} className="card pad col" style={{ gap: 8, marginBottom: 12 }}>
            <div className="row">
              <strong className="grow">{i.name}</strong>
              <Badge tone={i.status === 'connected' ? 'ok' : i.status === 'not_connected' ? 'bad' : 'warn'}>{i.status.replace('_', ' ')}</Badge>
            </div>
            <ul className="small" style={{ margin: 0, paddingLeft: 18 }}>
              <li>Sign-in: {spec.authMethod}</li>
              <li>Permissions: {spec.permissions.join('; ')}</li>
              <li>Can: {spec.capabilities.join('; ')}</li>
              <li>Credentials: {spec.credentialStorage}</li>
              <li>Disconnect: {spec.disconnect}</li>
            </ul>
            {i.id === 'github' && <div className="small faint">Manage the GitHub connection in the GitHub tab.</div>}
            {i.id === 'supabase' && sb && (
              sb.status === 'not_connected' ? (
                <div className="col" style={{ gap: 6 }}>
                  <div className="small">
                    Create a personal access token at{' '}
                    <button className="link" onClick={() => void openUrl('https://supabase.com/dashboard/account/tokens')}>
                      supabase.com/dashboard/account/tokens
                    </button>
                    , then paste it here. Genius checks it with a real API call before saving.
                  </div>
                  <div className="row">
                    <input className="input mono grow" type="password" value={token} onChange={(e) => setToken(e.target.value)} placeholder="sbp_…" aria-label="Supabase access token" autoComplete="off" />
                    <Button variant="primary" loading={busy === 'connect'} disabled={!token.trim()} onClick={() => void connect()}>
                      Connect
                    </Button>
                  </div>
                </div>
              ) : (
                <div className="row">
                  <span className="small grow">Connected{sb.meta && typeof sb.meta.projects === 'number' ? ` · ${sb.meta.projects} project(s) at last check` : ''}. Link a project from a workspace’s Integrations panel.</span>
                  <Button variant="danger" size="sm" loading={busy === 'disconnect'} onClick={() => void disconnect()}>
                    Disconnect
                  </Button>
                </div>
              )
            )}
          </div>
        );
      })}
    </section>
  );
}

// ------------------------------------------------------------------ GitHub

function GithubSettings() {
  const toast = useApp((s) => s.toast);
  const deviceCode = useApp((s) => s.deviceCode);
  const setDeviceCode = useApp((s) => s.setDeviceCode);
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [token, setToken] = useState('');
  const refresh = () => api.githubStatus().then(setStatus).catch((e) => toast('bad', errorMessage(e)));
  useEffect(() => {
    void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deviceCode]);

  const loginWeb = async () => {
    setBusy('web');
    try {
      await api.githubLoginWeb();
      toast('info', 'Waiting for GitHub… a one-time code will appear below.');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const loginToken = async () => {
    setBusy('token');
    try {
      setStatus(await api.githubLoginToken(token));
      setToken('');
      toast('ok', 'Connected to GitHub');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const logout = async () => {
    setBusy('logout');
    try {
      setStatus(await api.githubLogout());
      toast('ok', 'Disconnected from GitHub');
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section>
      <div>
        <h2>GitHub</h2>
        <p className="muted small" style={{ margin: '4px 0 0' }}>
          Genius connects through the official GitHub CLI (gh). gh performs GitHub's sign-in and keeps the token in your system credential store; Genius never stores it.
        </p>
      </div>
      {!status ? (
        <Spinner />
      ) : !status.ghInstalled ? (
        <Callout tone="warn" title="GitHub CLI not found">
          {status.detail}{' '}
          <a href="#" onClick={(e) => (e.preventDefault(), void openUrl('https://cli.github.com/'))}>
            cli.github.com
          </a>
        </Callout>
      ) : (
        <div className="card pad col" style={{ gap: 12 }}>
          <div className="row">
            <Github size={16} />
            <strong className="grow">Account</strong>
            {status.connected ? <Badge tone="ok">Connected{status.login ? ` as ${status.login}` : ''}</Badge> : <Badge tone="warn">Not connected</Badge>}
          </div>
          <div className="faint small">{status.ghVersion}</div>
          {status.connected ? (
            <>
              {status.scopes.length > 0 && <div className="small muted">Token scopes: {status.scopes.join(', ')}</div>}
              <div>
                <Button variant="danger" loading={busy === 'logout'} onClick={() => void logout()}>
                  Disconnect
                </Button>
              </div>
            </>
          ) : (
            <>
              <div className="row">
                <Button variant="primary" loading={busy === 'web'} onClick={() => void loginWeb()} icon={<Github size={15} />}>
                  Connect with browser
                </Button>
                <span className="faint small">Uses GitHub's device sign-in. Default gh scopes: repo, read:org, gist.</span>
              </div>
              {deviceCode && (
                <Callout tone="info" title="Enter this code on GitHub">
                  <div className="row" style={{ marginTop: 6 }}>
                    <span className="mono" style={{ fontSize: 20, letterSpacing: 2 }}>
                      {deviceCode.code}
                    </span>
                    <Button size="sm" icon={<Copy size={13} />} onClick={() => void navigator.clipboard.writeText(deviceCode.code)}>
                      Copy
                    </Button>
                    <Button size="sm" icon={<ExternalLink size={13} />} onClick={() => void openUrl(deviceCode.url)}>
                      Open {deviceCode.url}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setDeviceCode(null)}>
                      Hide
                    </Button>
                  </div>
                </Callout>
              )}
              <details>
                <summary className="small muted" style={{ cursor: 'pointer' }}>
                  Connect with a personal access token instead
                </summary>
                <div className="col" style={{ marginTop: 10 }}>
                  <div className="row">
                    <input className="input mono grow" type="password" autoComplete="off" placeholder="github_pat_… or ghp_…" value={token} onChange={(e) => setToken(e.target.value)} aria-label="GitHub token" />
                    <Button disabled={!token.trim()} loading={busy === 'token'} onClick={() => void loginToken()}>
                      Connect
                    </Button>
                  </div>
                  <span className="faint small">The token is passed directly to gh, which validates it and stores it in your credential store. Needs the “repo” scope to create repositories.</span>
                </div>
              </details>
              {!status.connected && status.detail && <div className="faint tiny mono">{status.detail}</div>}
            </>
          )}
        </div>
      )}
    </section>
  );
}

// ------------------------------------------------------------------ approvals

function ApprovalSettings() {
  const settings = useApp((s) => s.settings)!;
  const save = useSave();
  const setPolicy = (k: keyof Settings['approvals'], v: Policy) => void save({ ...settings, approvals: { ...settings.approvals, [k]: v } }, true);
  const rows: { k: keyof Settings['approvals']; title: string; desc: string; warn?: string }[] = [
    { k: 'applyAiEdits', title: 'Apply AI code changes', desc: 'Changes and Build Doctor fixes are shown as a reviewable diff first. The very first generation into a fresh template is always applied (every file stays revertible in History).' },
    { k: 'dependencyInstall', title: 'Install npm dependencies', desc: 'npm install downloads packages and runs their install scripts.' },
    { k: 'gitCommit', title: 'Create Git commits', desc: 'Commits are always created from files you staged and are secret-scanned first.' },
    { k: 'gitPush', title: 'Push to remotes', desc: 'Uploads commits to GitHub or another remote. Genius never force-pushes.', warn: 'Automatic pushing publishes code without asking. Only enable this if you are sure.' },
  ];
  return (
    <section>
      <div>
        <h2>Approvals & autonomy</h2>
        <p className="muted small" style={{ margin: '4px 0 0' }}>
          Choose what Genius may do on its own. Modified npm scripts, operations outside the project folder, and deletions of projects always ask.
        </p>
      </div>
      <div className="card">
        {rows.map((r) => (
          <div key={r.k} className="row" style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)', alignItems: 'flex-start' }}>
            <div className="grow">
              <div style={{ fontWeight: 600 }}>{r.title}</div>
              <div className="muted small">{r.desc}</div>
              {r.warn && settings.approvals[r.k] === 'auto' && <div className="small" style={{ color: 'var(--warn)', marginTop: 4 }}>{r.warn}</div>}
            </div>
            <Segmented label={r.title} value={settings.approvals[r.k]} onChange={(v) => setPolicy(r.k, v)} options={[{ value: 'ask', label: 'Ask first' }, { value: 'auto', label: 'Automatic' }]} />
          </div>
        ))}
        <div className="row" style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)' }}>
          <div className="grow">
            <div style={{ fontWeight: 600 }}>Verify after applying AI changes</div>
            <div className="muted small">Run a real production build after AI changes are applied, and offer Build Doctor repairs if it fails.</div>
          </div>
          <Switch label="Verify after applying" checked={settings.verifyAfterApply} onChange={(v) => void save({ ...settings, verifyAfterApply: v }, true)} />
        </div>
        <div className="row" style={{ padding: '14px 16px' }}>
          <div className="grow">
            <div style={{ fontWeight: 600 }}>Automatic repair attempts</div>
            <div className="muted small">The Build Doctor stops after this many attempts for the same failure, so it never loops blindly.</div>
          </div>
          <Segmented label="Repair attempts" value={String(settings.maxRepairAttempts)} onChange={(v) => void save({ ...settings, maxRepairAttempts: Number(v) }, true)} options={['0', '1', '2', '3', '5'].map((v) => ({ value: v, label: v }))} />
        </div>
      </div>
    </section>
  );
}

// ------------------------------------------------------------------ workspace & tools

function WorkspaceSettings() {
  const settings = useApp((s) => s.settings)!;
  const appInfo = useApp((s) => s.appInfo);
  const save = useSave();
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const load = () => {
    setTools(null);
    void api.detectTools().then(setTools);
  };
  useEffect(() => {
    load();
  }, []);
  const choose = async () => {
    const dir = await openDialog({ directory: true, title: 'Choose where new projects are created' });
    if (typeof dir === 'string') await save({ ...settings, workspaceDir: dir });
  };
  return (
    <section>
      <h2>Workspace & tools</h2>
      <div className="card pad col" style={{ gap: 10 }}>
        <Field label="New projects folder" hint="Generated projects are kept separate from Genius's own files.">
          <div className="row">
            <input className="input mono grow" readOnly value={settings.workspaceDir ?? appInfo?.workspaceDir ?? ''} aria-label="Projects folder" />
            <Button onClick={() => void choose()}>Change…</Button>
            {settings.workspaceDir && <Button variant="ghost" onClick={() => void save({ ...settings, workspaceDir: null })}>Use default</Button>}
          </div>
        </Field>
        <div className="faint small">Genius data (database, caches): <span className="mono">{appInfo?.dataDir}</span></div>
      </div>
      <div className="card">
        <div className="row" style={{ padding: '12px 16px', borderBottom: '1px solid var(--border)' }}>
          <strong className="grow">Detected tools</strong>
          <Button size="sm" icon={<RefreshCw size={13} />} onClick={load}>
            Re-check
          </Button>
        </div>
        {!tools ? (
          <div style={{ padding: 16 }}>
            <Spinner />
          </div>
        ) : (
          tools.map((t) => (
            <div key={t.name} className="row" style={{ padding: '10px 16px', borderBottom: '1px solid var(--border)', alignItems: 'flex-start' }}>
              {t.found ? <CheckCircle2 size={16} color="var(--ok)" /> : <XCircle size={16} color="var(--bad)" />}
              <div className="grow">
                <div>
                  <strong className="mono">{t.name}</strong> {t.version && <span className="muted small">{t.version}</span>}
                </div>
                <div className="faint small">{t.found ? t.path : t.hint}</div>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

function AppearanceSettings() {
  const settings = useApp((s) => s.settings)!;
  const save = useSave();
  return (
    <section>
      <h2>Appearance</h2>
      <div className="card pad row">
        <div className="grow">
          <div style={{ fontWeight: 600 }}>Theme</div>
          <div className="muted small">Dark is tuned for long sessions.</div>
        </div>
        <Segmented label="Theme" value={settings.theme} onChange={(v) => void save({ ...settings, theme: v }, true)} options={[{ value: 'dark', label: 'Dark' }, { value: 'light', label: 'Light' }, { value: 'system', label: 'System' }]} />
      </div>
      <div className="card pad row">
        <div className="grow">
          <div style={{ fontWeight: 600 }}>Interface</div>
          <div className="muted small">Simple shows just the chat and your app. Advanced adds files, terminal, Git, history and every tool.</div>
        </div>
        <Segmented label="Interface" value={settings.uiMode} onChange={(v) => void save({ ...settings, uiMode: v }, true)} options={[{ value: 'simple', label: 'Simple' }, { value: 'advanced', label: 'Advanced' }]} />
      </div>
      <div className="card pad row">
        <div className="grow">
          <div style={{ fontWeight: 600 }}>Assistant name</div>
          <div className="muted small">What the assistant is called in chats.</div>
        </div>
        <AssistantName />
      </div>
      <div className="card pad col small muted" style={{ gap: 4 }}>
        <strong style={{ color: 'var(--text)' }}>Keyboard shortcuts</strong>
        <span>Ctrl/⌘ + S — save file · Enter — send message (Shift + Enter for a new line) · Ctrl/⌘ + B — toggle sidebar · Ctrl/⌘ + J — toggle bottom panel</span>
        <span>Ctrl/⌘ + , — settings · Ctrl/⌘ + Shift + N — new project · Ctrl/⌘ + P — quick open file</span>
      </div>
    </section>
  );
}

function InternetCard() {
  const settings = useApp((s) => s.settings)!;
  const save = useSave();
  return (
    <div className="card pad row">
      <Globe size={16} />
      <div className="grow">
        <div style={{ fontWeight: 600 }}>Internet access</div>
        <div className="muted small">Lets Genius search the web and read public pages (docs, examples, design inspiration). Private and local network addresses are always blocked; web content is treated as data, never as instructions.</div>
      </div>
      <Switch label="Internet access" checked={settings.internetAccess} onChange={(v) => void save({ ...settings, internetAccess: v }, true)} />
    </div>
  );
}

function AssistantName() {
  const settings = useApp((s) => s.settings)!;
  const save = useSave();
  const [name, setName] = useState(settings.assistantName || 'Genius');
  return (
    <input
      className="input"
      style={{ width: 180 }}
      maxLength={40}
      value={name}
      aria-label="Assistant name"
      onChange={(e) => setName(e.target.value)}
      onBlur={() => name.trim() && name.trim() !== settings.assistantName && void save({ ...settings, assistantName: name.trim() }, true)}
    />
  );
}

// ------------------------------------------------------------------ updates

function UpdateSettings() {
  const settings = useApp((s) => s.settings)!;
  const appInfo = useApp((s) => s.appInfo);
  const update = useApp((s) => s.update);
  const checkUpdate = useApp((s) => s.checkUpdate);
  const setShowUpdate = useApp((s) => s.setShowUpdate);
  const skipped = useApp((s) => s.skippedVersion);
  const skipVersion = useApp((s) => s.skipVersion);
  const save = useSave();
  const [checking, setChecking] = useState(false);
  const check = async () => {
    setChecking(true);
    const u = await checkUpdate(true);
    setChecking(false);
    if (u?.available) setShowUpdate(true);
  };
  return (
    <section>
      <div className="card pad row" style={{ gap: 16 }}>
        <GeniusAvatar size={64} mood="happy" />
        <div className="grow">
          <h2>Project Genius</h2>
          <div className="muted small">Version {update?.currentVersion ?? appInfo?.version ?? '—'} · your AI software studio</div>
        </div>
      </div>
      <div className="card pad col" style={{ gap: 12 }}>
        <div className="row">
          <ArrowDownCircle size={16} />
          <strong className="grow">Updates</strong>
          {update?.available ? <Badge tone="accent">{update.latestVersion} available</Badge> : update ? <Badge tone="ok">Up to date</Badge> : null}
        </div>
        <div className="muted small">New versions are published as GitHub releases. Genius downloads the installer for your system, verifies it, and restarts into the new version. Your projects and settings are kept.</div>
        {update?.detail && !update.available && <div className="faint small">{update.detail}</div>}
        {skipped && (
          <div className="faint small">
            You skipped version {skipped}.{' '}
            <button className="link" onClick={() => skipVersion(null)}>
              Undo
            </button>
          </div>
        )}
        <div className="row">
          <Button icon={<RefreshCw size={14} />} loading={checking} onClick={() => void check()}>
            Check for updates
          </Button>
          {update?.available && (
            <Button variant="primary" onClick={() => setShowUpdate(true)}>
              Install {update.latestVersion}
            </Button>
          )}
        </div>
        <div className="row" style={{ gap: 10 }}>
          <Switch label="Check for updates on startup" checked={settings.autoCheckUpdates} onChange={(v) => void save({ ...settings, autoCheckUpdates: v }, true)} />
          <span className="small">Check for updates when Genius starts</span>
        </div>
      </div>
    </section>
  );
}
