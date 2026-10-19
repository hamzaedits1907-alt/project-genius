// The model/provider chip of the composer: what answers, whether the provider is reachable, a model
// picker, and a one-click diagnostic. It shows only measured or configured facts.
import { useEffect, useRef, useState } from 'react';
import { Check, ChevronDown, Loader2, Settings2, Stethoscope } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import type { DiagReport, ModelInfo } from '../lib/types';
import { useApp } from '../store/app';

const PROVIDER_LABEL: Record<string, string> = { nvidia: 'NVIDIA NIM', openai: 'OpenAI', anthropic: 'Anthropic', gemini: 'Google Gemini', custom: 'Custom endpoint' };

export function ModelChip() {
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const navigate = useApp((s) => s.navigate);
  const toast = useApp((s) => s.toast);
  const [open, setOpen] = useState(false);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [report, setReport] = useState<DiagReport | null>(null);
  const [busy, setBusy] = useState<'models' | 'diag' | null>(null);
  const root = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const away = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    window.addEventListener('mousedown', away);
    window.addEventListener('keydown', esc);
    return () => {
      window.removeEventListener('mousedown', away);
      window.removeEventListener('keydown', esc);
    };
  }, [open]);

  if (!settings) return null;
  const connected = !!keyStatus?.stored;
  const auto = settings.modelMode !== 'manual';
  const label = !connected ? 'Connect a provider' : auto ? 'Auto' : (settings.model ?? 'Choose a model').split('/').pop();
  const dot = !connected ? 'warn' : report ? (report.ok ? 'ok' : 'bad') : '';

  const loadModels = async () => {
    setBusy('models');
    try {
      const l = await api.listModels(false);
      setModels(l.models.filter((m) => ['chat', 'code'].includes(m.inferredCategory)));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };
  const pick = async (model: string | null) => {
    try {
      if (model) useApp.getState().setSettings(await api.selectModel(model));
      else useApp.getState().setSettings(await api.saveSettings({ ...settings, modelMode: 'auto' }));
    } catch (e) {
      toast('bad', errorMessage(e));
    }
  };
  const diagnose = async () => {
    setBusy('diag');
    try {
      setReport(await api.diagnoseProvider(false));
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="gx-modelchip" ref={root}>
      <button type="button" aria-haspopup="dialog" aria-expanded={open} aria-label={`Model: ${label}`} title="Model and provider" onClick={() => { setOpen(!open); if (!open && !models && connected) void loadModels(); }}>
        <span className={`dot ${dot}`} aria-hidden />
        <span className="ellipsis">{label}</span>
        <ChevronDown size={12} />
      </button>
      {open && (
        <div className="gx-modelpop" role="dialog" aria-label="Model and provider">
          <h5>Provider</h5>
          <dl>
            <dt>Provider</dt>
            <dd>{PROVIDER_LABEL[settings.provider] ?? settings.provider}</dd>
            <dt>Key</dt>
            <dd>{connected ? `saved${keyStatus?.hint ? ` (${keyStatus.hint})` : ''}` : 'not saved'}</dd>
            <dt>Model</dt>
            <dd>{auto ? 'chosen automatically for each request' : settings.model}</dd>
            {report && (
              <>
                <dt>Check</dt>
                <dd>{report.ok ? 'passed' : 'failed'} — {report.verdict}</dd>
              </>
            )}
          </dl>
          {connected && (
            <>
              <h5>Model</h5>
              <div className="opts" role="listbox" aria-label="Models">
                <button type="button" role="option" aria-selected={auto} className={auto ? 'on' : ''} onClick={() => void pick(null)}>
                  {auto && <Check size={12} />} Auto (recommended)
                </button>
                {busy === 'models' && <span className="small faint" style={{ padding: 6 }}><Loader2 size={12} className="spin" /> Loading models…</span>}
                {(models ?? []).map((m) => (
                  <button type="button" role="option" aria-selected={!auto && settings.model === m.id} key={m.id} className={!auto && settings.model === m.id ? 'on' : ''} onClick={() => void pick(m.id)}>
                    {!auto && settings.model === m.id && <Check size={12} />} <span className="ellipsis">{m.id}</span>
                  </button>
                ))}
              </div>
            </>
          )}
          <div className="row">
            {connected && (
              <button type="button" className="btn sm" disabled={busy === 'diag'} onClick={() => void diagnose()}>
                {busy === 'diag' ? <Loader2 size={12} className="spin" /> : <Stethoscope size={12} />} Test connection
              </button>
            )}
            <button type="button" className="btn sm ghost" onClick={() => { setOpen(false); navigate({ name: 'settings', tab: 'ai' }); }}>
              <Settings2 size={12} /> Provider settings
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
