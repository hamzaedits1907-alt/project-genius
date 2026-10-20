import { useKeyVault } from '../web/keySync';
import { useEffect, useRef, useState } from 'react';
import { ArrowUp, Check, FolderOpen, KeyRound, Plus, SlidersHorizontal, Sparkles, Trash2, Wrench, X, FileArchive } from 'lucide-react';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Github } from '../components/icons';
import { api, errorMessage } from '../lib/api';
import { timeAgo } from '../lib/logic';
import type { ModelInfo, Project, RepoInfo, ToolStatus } from '../lib/types';
import { useApp } from '../store/app';
import { Badge, Button, Callout, Empty, Modal, Spinner, StatusBadge } from '../components/ui';
import { GeniusAvatar } from '../components/GeniusAvatar';
import { markAutoBuild } from '../store/workspace';
import { KEY_LINK, KEY_PLACEHOLDER } from './ProviderPanels';
import { isWeb } from '../lib/platform';
import type { ProviderId } from '../lib/types';
import { BlueprintGallery } from '../components/BlueprintGallery';
import { EnhanceButton } from '../components/EnhanceButton';
import { blueprintById, type Blueprint } from '../lib/blueprints';

const IDEAS = [
  { label: 'Portfolio website', text: 'A personal portfolio website with a bold hero, project gallery with filters, about section, testimonials and a contact form. Modern, elegant, responsive.' },
  { label: 'Landing page', text: 'A landing page for a specialty coffee shop: hero with call to action, menu highlights, story section, opening hours, location and newsletter signup. Warm, inviting design.' },
  { label: 'Task manager', text: 'A modern task manager with projects, a kanban board with drag and drop, search, due dates and a clean responsive interface. Data saved in the browser.' },
  { label: 'Finance tracker', text: 'A personal finance tracker: add income and expenses, categories, monthly charts, and data saved in the browser.' },
  { label: 'Recipe book', text: 'A recipe book where I can add recipes with ingredients and steps, tag them, and filter by tag. Beautiful food-magazine style.' },
];

/** A short project name from a description: "A landing page for a coffee shop" → "Coffee Shop Landing". */
export function nameFromIdea(text: string): string {
  const stop = new Set(['a', 'an', 'the', 'for', 'with', 'and', 'my', 'me', 'build', 'make', 'create', 'i', 'want', 'to', 'of', 'that', 'where', 'can', 'app', 'application', 'website', 'site', 'page', 'please', 'in', 'on', 'which', 'is', 'it']);
  const words = text
    .replace(/[^\p{L}\p{N}\s-]/gu, ' ')
    .split(/\s+/)
    .filter((w) => w && !stop.has(w.toLowerCase()))
    .slice(0, 3);
  const name = words.map((w) => w[0].toUpperCase() + w.slice(1).toLowerCase()).join(' ');
  return name || 'My App';
}

function greeting(): string {
  const h = new Date().getHours();
  return h < 5 ? 'Up late?' : h < 12 ? 'Good morning!' : h < 18 ? 'Good afternoon!' : 'Good evening!';
}

function stageLabel(p: Project): { text: string; tone: 'ok' | 'warn' | 'accent' | 'idle' } {
  switch (p.status.stage) {
    case 'planned':
      return { text: 'Plan ready', tone: 'accent' };
    case 'generated':
      return { text: 'Code generated', tone: 'ok' };
    case 'imported':
      return { text: 'Imported', tone: 'idle' };
    default:
      return { text: 'New', tone: 'idle' };
  }
}

export default function Home() {
  const navigate = useApp((s) => s.navigate);
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const toast = useApp((s) => s.toast);
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [tools, setTools] = useState<ToolStatus[] | null>(null);
  const [deleting, setDeleting] = useState<Project | null>(null);
  const [cloning, setCloning] = useState(false);
  const [idea, setIdea] = useState('');
  const [starting, setStarting] = useState(false);
  /** Blueprint whose brief is in the idea box (shows the "customise" hint). */
  const [blueprint, setBlueprint] = useState<string | null>(null);
  /** Blueprint being built straight from its card. */
  const [buildingBp, setBuildingBp] = useState<string | null>(null);
  const [enhancing, setEnhancing] = useState(false);
  const ideaRef = useRef<HTMLTextAreaElement>(null);

  const load = () => api.listProjects().then(setProjects).catch((e) => toast('bad', errorMessage(e)));
  useEffect(() => {
    void load();
    void api.detectTools().then(setTools);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const importFolder = async () => {
    const dir = await openDialog({ directory: true, title: 'Open an existing project folder' });
    if (typeof dir !== 'string') return;
    try {
      const p = await api.importProject(dir);
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const importZip = async () => {
    const file = await openDialog({ title: 'Import a project from a ZIP archive', filters: [{ name: 'ZIP archive', extensions: ['zip'] }] });
    if (typeof file !== 'string') return;
    try {
      const p = await api.importZipProject(file);
      toast('ok', 'Extracted safely — no scripts were run. Review package.json scripts before installing.');
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };

  const ready = !!keyStatus?.stored && !!settings?.model;
  const startBuild = async (raw: string, name?: string) => {
    const text = raw.trim();
    if (text.length < 10 || !ready) return;
    setStarting(true);
    try {
      const p = await api.createProject(name ?? nameFromIdea(text), text);
      markAutoBuild(p.id);
      await api.startPlan(p.id, text, '');
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setStarting(false);
    }
  };
  const loaded = blueprint ? blueprintById(blueprint) : undefined;
  // Building an unedited blueprint keeps its friendly project name.
  const buildIdea = () => startBuild(idea, loaded && idea.trim() === loaded.prompt.trim() ? loaded.projectName : undefined);

  const pickBlueprint = (b: Blueprint) => {
    setIdea(b.prompt);
    setBlueprint(b.id);
    const el = ideaRef.current;
    if (el) {
      el.focus();
      el.setSelectionRange(0, 0);
      el.scrollTop = 0;
      el.scrollIntoView?.({ behavior: 'smooth', block: 'center' });
    }
  };
  const buildBlueprint = async (b: Blueprint) => {
    setBuildingBp(b.id);
    try {
      await startBuild(b.prompt, b.projectName);
    } finally {
      setBuildingBp(null);
    }
  };

  const missingTools = tools?.filter((t) => !t.found) ?? [];
  const setup = [
    { done: !!keyStatus?.stored, label: keyStatus?.stored ? 'NVIDIA API key saved' : 'Save your NVIDIA API key', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <KeyRound size={14} /> },
    { done: !!settings?.model, label: settings?.model ? `Model: ${settings.model}` : 'Choose a model', action: () => navigate({ name: 'settings', tab: 'ai' }), icon: <Sparkles size={14} /> },
    {
      done: tools ? missingTools.filter((t) => t.name === 'node' || t.name === 'npm').length === 0 : false,
      label: tools ? (missingTools.some((t) => t.name === 'node' || t.name === 'npm') ? 'Node.js / npm not found' : 'Node.js and npm available') : 'Checking tools…',
      action: () => navigate({ name: 'settings', tab: 'workspace' }),
      icon: <Wrench size={14} />,
    },
  ];
  const setupDone = setup.every((s) => s.done);
  // New users see blueprints first; returning users see their projects first.
  const gallery = <BlueprintGallery onUse={pickBlueprint} onBuild={(b) => void buildBlueprint(b)} canBuild={ready && !starting} building={buildingBp} selected={blueprint} />;

  return (
    <div className="page">
      <div className="page-inner">
        <div className="home-hero">
          <div className="gx-ws-label">Code workspace</div>
          <GeniusAvatar size={96} mood={starting || enhancing ? 'working' : idea.trim() ? 'listening' : 'idle'} />
          <div className="home-greet">{greeting()} I'm Genius.</div>
          <h1>What should we build today?</h1>
          <div className={`prompt-box ${enhancing ? 'enhancing' : ''}`}>
            {loaded && (
              <div className="prompt-hint fade-in">
                <span className="badge accent">Blueprint</span>
                <span className="ellipsis">
                  <b>{loaded.title}</b> · Customize, then press Enter
                </span>
                <span className="spacer" />
                <button
                  className="icon-btn"
                  style={{ width: 22, height: 22 }}
                  aria-label="Clear blueprint"
                  title="Clear"
                  onClick={() => {
                    setIdea('');
                    setBlueprint(null);
                    ideaRef.current?.focus();
                  }}
                >
                  <X size={13} />
                </button>
              </div>
            )}
            <textarea
              ref={ideaRef}
              aria-label="Describe your app"
              placeholder="Describe your app or website… e.g. “A portfolio site for a photographer with a dark, elegant look”"
              value={idea}
              rows={idea.length > 240 || idea.split('\n').length > 4 ? 10 : 3}
              readOnly={enhancing}
              onChange={(e) => {
                setIdea(e.target.value);
                if (!e.target.value.trim()) setBlueprint(null);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && !e.shiftKey) {
                  e.preventDefault();
                  void buildIdea();
                }
              }}
              autoFocus
            />
            <div className="row prompt-tools">
              <button className="btn ghost sm" onClick={() => navigate({ name: 'create' })} title="Name, tech and design preferences, and review the plan before code is written">
                <SlidersHorizontal size={13} /> Detailed setup
              </button>
              <EnhanceButton value={idea} onChange={setIdea} disabled={!ready || starting} disabledReason="Finish setup below first" onBusyChange={setEnhancing} onEnhanced={() => ideaRef.current?.focus()} />
              <span className="spacer" />
              <span className="faint tiny">{ready ? 'Genius plans it, then writes the code' : 'Finish setup below first'}</span>
              <button className="send-btn lg" aria-label="Build it" title="Build it (Enter)" disabled={idea.trim().length < 10 || !ready || starting} onClick={() => void buildIdea()}>
                {starting ? <span className="spinner" /> : <ArrowUp size={18} />}
              </button>
            </div>
          </div>
          <div className="suggestions center-row">
            {IDEAS.map((x) => (
              <button
                key={x.label}
                onClick={() => {
                  setIdea(x.text);
                  setBlueprint(null);
                }}
              >
                {x.label}
              </button>
            ))}
          </div>
          <div className="row home-actions">
            <Button variant="ghost" icon={<Plus size={15} />} onClick={() => navigate({ name: 'create' })}>
              New application
            </Button>
            <Button variant="ghost" icon={<FolderOpen size={15} />} onClick={() => void importFolder()}>
              Open folder
            </Button>
            <Button variant="ghost" icon={<FileArchive size={15} />} onClick={() => void importZip()}>
              Import ZIP
            </Button>
            <Button variant="ghost" icon={<Github size={15} />} onClick={() => setCloning(true)}>
              Clone from GitHub
            </Button>
          </div>
        </div>

        {!ready && <QuickSetup />}
        {ready && !setupDone && (
          <div className="card pad" style={{ marginTop: 16 }}>
            <div style={{ fontWeight: 650, marginBottom: 8 }}>Finish setup</div>
            <div className="col" style={{ gap: 6 }}>
              {setup.map((s) => (
                <button key={s.label} className="row btn ghost" style={{ justifyContent: 'flex-start', height: 30 }} onClick={s.action}>
                  {s.done ? <Check size={15} color="var(--ok)" /> : <X size={15} color="var(--warn)" />}
                  {s.icon}
                  <span>{s.label}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        {!projects?.length && gallery}

        <div className="section-title">
          <h2>Recent projects</h2>
          {projects && <Badge>{projects.length}</Badge>}
        </div>
        {projects === null ? (
          <div className="row muted" style={{ padding: 20 }}>
            <Spinner /> Loading projects…
          </div>
        ) : projects.length === 0 ? (
          <div className="card" style={{ marginTop: 14 }}>
            <Empty icon={<Sparkles size={20} />} title="No projects yet" action={<Button variant="primary" onClick={() => navigate({ name: 'create' })}>Create your first app</Button>}>
              Projects you create or open appear here, with their real build status.
            </Empty>
          </div>
        ) : (
          <div className="project-grid">
            {projects.map((p) => {
              const st = stageLabel(p);
              return (
                <div key={p.id} className="project-card" role="button" tabIndex={0} onClick={() => navigate({ name: 'project', id: p.id })} onKeyDown={(e) => e.key === 'Enter' && navigate({ name: 'project', id: p.id })}>
                  <div className="row">
                    <h3 className="grow ellipsis">{p.name}</h3>
                    <button
                      className="icon-btn"
                      aria-label={`Delete ${p.name}`}
                      title="Delete project"
                      onClick={(e) => {
                        e.stopPropagation();
                        setDeleting(p);
                      }}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                  <p>{p.description || p.spec?.purpose || p.path}</p>
                  <div className="row wrap">
                    <Badge tone={st.tone}>{st.text}</Badge>
                    {p.status.lastBuild ? (
                      <StatusBadge status={p.status.lastBuild.status} label={`build ${p.status.lastBuild.status}`} />
                    ) : (
                      <Badge>not built</Badge>
                    )}
                    {p.status.publish?.step === 'pushed' && <Badge tone="ok">on GitHub</Badge>}
                    <span className="spacer" />
                    <span className="faint tiny">{timeAgo(p.lastOpenedAt ?? p.updatedAt)}</span>
                  </div>
                </div>
              );
            })}
          </div>
        )}
        {!!projects?.length && gallery}
      </div>
      {deleting && (
        <DeleteDialog
          project={deleting}
          onClose={() => setDeleting(null)}
          onDone={() => {
            setDeleting(null);
            void load();
          }}
        />
      )}
      {cloning && <CloneDialog onClose={() => setCloning(false)} />}
    </div>
  );
}

function DeleteDialog({ project, onClose, onDone }: { project: Project; onClose: () => void; onDone: () => void }) {
  const toast = useApp((s) => s.toast);
  const [deleteFiles, setDeleteFiles] = useState(false);
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      const r = await api.deleteProject(project.id, deleteFiles);
      toast('ok', r.filesDeleted ? `Deleted ${project.name} and its files` : `Removed ${project.name} from Genius (files kept)`);
      onDone();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal
      title={`Delete “${project.name}”?`}
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="danger" loading={busy} disabled={deleteFiles && confirm !== project.name} onClick={() => void run()}>
            {deleteFiles ? 'Delete project and files' : 'Remove from Genius'}
          </Button>
        </>
      }
    >
      <p className="muted" style={{ margin: 0 }}>
        This removes the project, its history and activity from Genius. Running processes are stopped.
      </p>
      <label className="checkbox">
        <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
        Also permanently delete the project folder
      </label>
      <div className="mono small faint">{project.path}</div>
      {deleteFiles && (
        <>
          <Callout tone="bad">The folder and all its files will be deleted. This cannot be undone. Folders outside the Genius workspace are never deleted.</Callout>
          <input className="input" placeholder={`Type “${project.name}” to confirm`} value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        </>
      )}
    </Modal>
  );
}

function CloneDialog({ onClose }: { onClose: () => void }) {
  const navigate = useApp((s) => s.navigate);
  const toast = useApp((s) => s.toast);
  const [repos, setRepos] = useState<RepoInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    api
      .githubStatus()
      .then((s) => {
        if (!s.connected) throw new Error(s.detail || 'Not connected to GitHub. Connect in Settings → GitHub.');
        return api.githubRepos();
      })
      .then(setRepos)
      .catch((e) => setError(errorMessage(e)));
  }, []);
  const clone = async (r: RepoInfo) => {
    setBusy(r.nameWithOwner);
    try {
      const p = await api.githubClone(r.nameWithOwner);
      toast('ok', `Cloned ${r.nameWithOwner}`);
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const shown = (repos ?? []).filter((r) => r.nameWithOwner.toLowerCase().includes(filter.toLowerCase()));
  return (
    <Modal title="Clone a GitHub repository" onClose={onClose} wide>
      {error ? (
        <>
          <Callout tone="warn">{error}</Callout>
          <div>
            <Button onClick={() => navigate({ name: 'settings', tab: 'github' })}>Open GitHub settings</Button>
          </div>
        </>
      ) : repos === null ? (
        <div className="row muted">
          <Spinner /> Loading repositories you can access…
        </div>
      ) : (
        <>
          <input className="input" placeholder="Filter repositories" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter repositories" />
          <div className="model-list">
            {shown.map((r) => (
              <div key={r.nameWithOwner} className="model-row" style={{ cursor: 'default' }}>
                <div className="col" style={{ gap: 0 }}>
                  <span>{r.nameWithOwner}</span>
                  {r.description && <span className="faint tiny ellipsis">{r.description}</span>}
                </div>
                <Badge tone={r.visibility === 'public' ? 'warn' : 'idle'}>{r.visibility}</Badge>
                <Button size="sm" loading={busy === r.nameWithOwner} disabled={!!busy} onClick={() => void clone(r)}>
                  Clone
                </Button>
              </div>
            ))}
            {shown.length === 0 && <div className="faint small" style={{ padding: 12 }}>No repositories match.</div>}
          </div>
        </>
      )}
    </Modal>
  );
}

/** Models that work well for planning and writing code, best first (matched by substring). */
const PREFERRED_MODELS = ['qwen3-coder', 'qwen2.5-coder-32b', 'nemotron-3-super', 'nemotron-3-ultra', 'deepseek-v', 'glm-5', 'kimi-k', 'gpt-oss-120b', 'llama-3.3-70b', 'llama-3.1-405b', 'nemotron-super', 'nemotron-3.5-lightning', 'gpt-oss-20b', 'llama-3.1-70b', 'mixtral-8x22b'];

/** Candidate models in the order they should be tried: preferred families first, then everything else that chats. */
export function recommendModels(models: ModelInfo[]): string[] {
  const usable = models.filter((m) => m.inferredCategory === 'chat' || m.inferredCategory === 'code');
  const out: string[] = [];
  for (const want of PREFERRED_MODELS) for (const m of usable) if (m.id.toLowerCase().includes(want) && !out.includes(m.id)) out.push(m.id);
  for (const m of usable) if (!out.includes(m.id)) out.push(m.id);
  return out;
}

export function recommendModel(models: ModelInfo[]): string | null {
  return recommendModels(models)[0] ?? null;
}

/** First-run setup right on Home: paste a key, Genius picks a good model. */
export function QuickSetup() {
  const navigate = useApp((s) => s.navigate);
  const keyStatus = useApp((s) => s.keyStatus);
  const setKeyStatus = useApp((s) => s.setKeyStatus);
  const setSettings = useApp((s) => s.setSettings);
  const toast = useApp((s) => s.toast);
  const [key, setKey] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [allowRelay, setAllowRelay] = useState(false);
  const vaultState = useKeyVault((v) => v.state);
  const vaultProviders = useKeyVault((v) => v.providers);

  const settings = useApp((s) => s.settings);
  const provider: ProviderId = settings?.provider ?? 'nvidia';
  // Genius Web only: NVIDIA (and custom endpoints) refuse calls from a web page, so requests must go through the relay, with the user's consent.
  const needsRelay = isWeb && (provider === 'nvidia' || provider === 'custom');
  const providerLabel = { nvidia: 'NVIDIA', openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Google Gemini', custom: 'your endpoint' }[provider];

  const pickModel = async () => {
    setBusy('Finding the best model for you…');
    const list = await api.listModels(true);
    if (provider !== 'custom' && (isWeb || provider !== 'nvidia')) {
      // Auto picks among the models this key lists; the key is proven by the listing above
      setSettings(await api.saveSettings({ ...(settings as NonNullable<typeof settings>), modelMode: 'auto', model: null }));
      toast('ok', `Connected: ${list.models.length} models are available. Genius picks one for each request; you can choose one in Settings.`);
      return;
    }
    // A provider's model list can include models that no longer answer (NVIDIA lists some that return 404), so a model is only chosen once it really replied.
    const candidates = recommendModels(list.models).slice(0, 8);
    if (!candidates.length) throw new Error('No chat models are available for this key. Choose one in Settings → AI provider.');
    let best: string | null = null;
    for (const c of candidates) {
      setBusy(`Testing ${c}…`);
      try {
        await api.probeModel(c);
        best = c;
        break;
      } catch {
        /* try the next candidate */
      }
    }
    if (!best) throw new Error('None of the first models answered a test message. Choose a model in Settings → AI provider, or try again later.');
    setSettings(await api.selectModel(best));
    toast('ok', `All set! Genius will use ${best}. You can change it any time in Settings.`);
  };

  const connect = async () => {
    setError(null);
    try {
      if (needsRelay && settings && !settings.relayConsent) {
        if (!allowRelay) throw new Error('Tick the box below to allow the relay for this provider, or choose OpenAI, Anthropic or Gemini.');
        setSettings(await api.saveSettings({ ...settings, relayConsent: true }));
      }
      if (!keyStatus?.stored) {
        setBusy('Saving your key securely…');
        setKeyStatus(await api.saveApiKey(key.trim()));
        setKey('');
      }
      await pickModel();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="card pad quick-setup">
      <div className="row" style={{ gap: 12, alignItems: 'flex-start' }}>
        <KeyRound size={18} color="var(--accent-2)" style={{ marginTop: 2 }} />
        <div className="grow col" style={{ gap: 10 }}>
          <div>
            <div style={{ fontWeight: 650 }}>{keyStatus?.stored ? 'One more step: pick a model' : 'Connect Genius to its AI brain'}</div>
            <div className="muted small">
              {keyStatus?.stored ? (
                'Your key is saved. Genius will pick a strong coding model for you.'
              ) : (
                <>
                  {isWeb && (
                    <select className="select" style={{ marginRight: 8 }} aria-label="Provider" value={provider} onChange={(e) => settings && void api.saveSettings({ ...settings, provider: e.target.value as ProviderId, baseUrl: '', model: null }).then(setSettings)}>
                      <option value="openai">OpenAI</option>
                      <option value="anthropic">Anthropic</option>
                      <option value="gemini">Google Gemini</option>
                      <option value="nvidia">NVIDIA NIM</option>
                    </select>
                  )}
                  Paste your {providerLabel} API key
                  {KEY_LINK[provider] && (
                    <>
                      {' '}from{' '}
                      <a href="#" onClick={(e) => (e.preventDefault(), void openUrl(KEY_LINK[provider]!))}>
                        {KEY_LINK[provider]!.replace('https://', '').replace(/\/$/, '')}
                      </a>
                    </>
                  )}
                  . {isWeb ? 'It stays in this browser tab only, unless you turn on the key vault (Settings → Account): then it is saved to your account encrypted, readable only with your passphrase.' : "It's stored in your system's secure credential store, never in a file."}
                </>
              )}
            </div>
          </div>
          <div className="row">
            {!keyStatus?.stored && (
              <input
                className="input mono grow"
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder={KEY_PLACEHOLDER[provider]}
                aria-label={`${providerLabel} API key`}
                value={key}
                onChange={(e) => setKey(e.target.value)}
                onKeyDown={(e) => e.key === 'Enter' && key.trim() && void connect()}
              />
            )}
            <Button variant="primary" loading={!!busy} disabled={!keyStatus?.stored && !key.trim()} onClick={() => void connect()}>
              {keyStatus?.stored ? 'Pick a model for me' : 'Connect'}
            </Button>
            <Button variant="ghost" onClick={() => navigate({ name: 'settings', tab: 'ai' })}>
              Advanced…
            </Button>
          </div>
          {!keyStatus?.stored && vaultState === 'locked' && vaultProviders.length > 0 && (
            <Callout tone="info">
              Your account already has saved AI keys. <a href="#" onClick={(e) => (e.preventDefault(), navigate({ name: 'settings', tab: 'account' }))}>Unlock your key vault</a> and you do not need to paste one here.
            </Callout>
          )}
          {!keyStatus?.stored && vaultState === 'none' && (
            <div className="faint small">Tip: after you connect, create a key vault in Settings → Account to have this key on all your devices.</div>
          )}
          {needsRelay && !settings?.relayConsent && (
            <label className="row small" style={{ gap: 8, alignItems: 'flex-start' }}>
              <input type="checkbox" checked={allowRelay} onChange={(e) => setAllowRelay(e.target.checked)} style={{ marginTop: 3 }} />
              <span>{providerLabel} does not allow web pages to call it directly. I allow Genius Web to pass my requests through its relay, which forwards them with my own key and stores nothing.</span>
            </label>
          )}
          {busy && <div className="small muted">{busy}</div>}
          {error && <Callout tone="bad">{error}</Callout>}
        </div>
      </div>
    </div>
  );
}
