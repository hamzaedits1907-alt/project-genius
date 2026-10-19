import { useEffect, useState } from 'react';
import { CheckCircle2, CircleDashed, Info, MinusCircle, Stethoscope, TriangleAlert, XCircle } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { DiagReport, DiagStep, ProviderCapabilities, ProviderId, Settings, TaskLimits } from '../lib/types';
import { useApp } from '../store/app';
import { Badge, Button, Callout, Field } from '../components/ui';

export const DEFAULT_BASE_URL: Record<ProviderId, string> = {
  nvidia: 'https://integrate.api.nvidia.com/v1',
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com/v1',
  gemini: 'https://generativelanguage.googleapis.com/v1beta',
  custom: '',
};

export const KEY_LINK: Partial<Record<ProviderId, string>> = {
  nvidia: 'https://build.nvidia.com/',
  openai: 'https://platform.openai.com/api-keys',
  anthropic: 'https://console.anthropic.com/settings/keys',
  gemini: 'https://aistudio.google.com/app/apikey',
};

export const KEY_PLACEHOLDER: Record<ProviderId, string> = { nvidia: 'nvapi-…', openai: 'sk-…', anthropic: 'sk-ant-…', gemini: 'AIza…', custom: 'API key of your endpoint' };

/** Which provider the saved key belongs to. Every provider has its own key slot. */
export function ProviderCard({ settings, save }: { settings: Settings; save: (next: Settings, quiet?: boolean) => Promise<boolean> }) {
  const [caps, setCaps] = useState<ProviderCapabilities[]>([]);
  useEffect(() => {
    api.providerCapabilities().then(setCaps).catch(() => setCaps([]));
  }, []);
  const current = caps.find((c) => c.provider === settings.provider);
  const choose = (provider: ProviderId) => {
    if (provider === settings.provider) return;
    const nv = provider === 'nvidia';
    // Auto routing is tuned for NVIDIA's model families: every other provider runs the model you pick.
    void save({ ...settings, provider, baseUrl: DEFAULT_BASE_URL[provider] || settings.baseUrl, modelMode: nv ? settings.modelMode : 'manual', model: null }, true);
  };
  return (
    <div className="card pad col" style={{ gap: 10 }}>
      <div className="row">
        <strong className="grow">Provider</strong>
        {current && <Badge tone={current.verification === 'live' ? 'ok' : 'warn'} title={current.verification === 'live' ? 'Run against the real service' : 'Covered by contract tests against a stand-in server; not yet run against the live service'}>{current.verification === 'live' ? 'verified live' : 'contract-tested only'}</Badge>}
      </div>
      <div className="row" role="radiogroup" aria-label="AI provider" style={{ flexWrap: 'wrap', gap: 6 }}>
        {(caps.length ? caps : ([{ provider: 'nvidia', label: 'NVIDIA NIM' }] as ProviderCapabilities[])).map((c) => (
          <label key={c.provider} className={`choice ${settings.provider === c.provider ? 'on' : ''}`}>
            <input type="radio" name="provider" checked={settings.provider === c.provider} onChange={() => choose(c.provider)} />
            <span>{c.label}</span>
          </label>
        ))}
      </div>
      {current?.notes.map((n) => (
        <div key={n} className="faint small">
          {n}
        </div>
      ))}
      <div className="faint small">You bring your own key. It is stored in this computer's credential store, one slot per provider, and is only ever sent to that provider.</div>
    </div>
  );
}

const ICON: Record<DiagStep['status'], React.ReactNode> = {
  ok: <CheckCircle2 size={15} color="var(--ok)" />,
  warn: <TriangleAlert size={15} color="var(--warn)" />,
  fail: <XCircle size={15} color="var(--bad)" />,
  skipped: <MinusCircle size={15} color="var(--text-faint, #888)" />,
  info: <Info size={15} color="var(--accent-2)" />,
};

/** The one place to find out why the provider does or does not work. */
export function DiagnosticsCard({ keyStored, onDone }: { keyStored: boolean; onDone?: () => void }) {
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState<'quick' | 'deep' | null>(null);
  const [report, setReport] = useState<DiagReport | null>(null);
  const run = async (deep: boolean) => {
    setBusy(deep ? 'deep' : 'quick');
    try {
      setReport(await api.diagnoseProvider(deep));
      onDone?.();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  return (
    <div className="card pad col" style={{ gap: 10 }}>
      <div className="row">
        <Stethoscope size={16} />
        <strong className="grow">Connection diagnostics</strong>
        <Button loading={busy === 'quick'} disabled={busy === 'deep'} onClick={() => void run(false)}>
          Run diagnostics
        </Button>
        <Button variant="ghost" loading={busy === 'deep'} disabled={busy === 'quick' || !keyStored} onClick={() => void run(true)} title="Also sends one request to check that the model can follow Genius's tool-call format">
          Deep check
        </Button>
      </div>
      {!report && <div className="faint small">Checks the key, address, model list, model access and a real test message, and tells you what to do about anything that fails. Your key is never shown.</div>}
      {report && (
        <div className="col" style={{ gap: 6 }}>
          <Callout tone={report.ok ? 'ok' : 'bad'}>{report.verdict}</Callout>
          {report.steps.map((s) => (
            <div key={s.id} className="col" style={{ gap: 2 }}>
              <div className="row small" style={{ alignItems: 'flex-start' }}>
                {ICON[s.status] ?? <CircleDashed size={15} />}
                <strong>{s.title}</strong>
                <span className="grow faint">{s.detail}</span>
                {s.ms != null && <span className="faint tiny">{s.ms} ms</span>}
              </div>
              {s.hint && (
                <div className="small" style={{ marginLeft: 23, color: s.status === 'fail' ? 'var(--bad)' : 'var(--warn)' }}>
                  {s.hint}
                </div>
              )}
            </div>
          ))}
          <div className="faint tiny">
            {report.label} at {report.host} · checked {new Date(report.checkedAt).toLocaleTimeString()} · integration: {report.integrationVerified === 'live' ? 'verified live' : 'contract tests only'}
          </div>
        </div>
      )}
    </div>
  );
}

const LIMIT_FIELDS: { key: keyof TaskLimits; label: string; hint: string; min: number; max: number }[] = [
  { key: 'maxModelCalls', label: 'Model calls per task', hint: '0 = no limit', min: 0, max: 1000 },
  { key: 'maxToolCalls', label: 'Tool calls per task', hint: '0 = no limit', min: 0, max: 5000 },
  { key: 'maxMinutes', label: 'Minutes per task', hint: '0 = no limit', min: 0, max: 1440 },
  { key: 'maxTokens', label: 'Tokens per task', hint: '0 = no limit. Provider-reported plus estimated.', min: 0, max: 100_000_000 },
  { key: 'maxConcurrentTasks', label: 'Tasks at the same time', hint: 'Across all projects', min: 1, max: 32 },
  { key: 'maxProviderRetries', label: 'Retries on rate limits / network errors', hint: 'Per request', min: 0, max: 10 },
];

/** Limits on Genius's own work. They stop runaway loops; they do not control the provider's billing. */
export function LimitsCard({ settings, save }: { settings: Settings; save: (next: Settings, quiet?: boolean) => Promise<boolean> }) {
  const [limits, setLimits] = useState<TaskLimits>(settings.limits);
  const dirty = JSON.stringify(limits) !== JSON.stringify(settings.limits);
  return (
    <div className="card pad col" style={{ gap: 12 }}>
      <strong>Limits</strong>
      <div className="faint small">
        These stop a task that loops or runs away: Genius says which limit it hit and what was kept. They are not a spending cap — token figures are what your provider reports (or are marked estimated), and your provider's dashboard is the authority for billing.
      </div>
      <div className="row" style={{ flexWrap: 'wrap', gap: 14, alignItems: 'flex-start' }}>
        {LIMIT_FIELDS.map((f) => (
          <Field key={f.key} label={f.label} hint={f.hint}>
            <input className="input" type="number" aria-label={f.label} min={f.min} max={f.max} value={limits[f.key]} onChange={(e) => setLimits({ ...limits, [f.key]: Math.max(f.min, Math.min(f.max, Number(e.target.value) || 0)) })} />
          </Field>
        ))}
      </div>
      <div>
        <Button variant="primary" disabled={!dirty} onClick={() => void save({ ...settings, limits })}>
          Save limits
        </Button>
      </div>
    </div>
  );
}
