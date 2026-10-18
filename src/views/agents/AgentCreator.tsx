// Create or edit a custom agent. Covers every field of CustomAgent; advanced settings
// (model, tools, scope, budgets, approval...) are shown in Advanced mode.
import { useEffect, useMemo, useState, type ReactNode } from 'react';
import { Wand2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import {
  APPROVAL_LABEL,
  AVATAR_PRESETS,
  BUDGET_LABELS,
  BUDGET_MAX,
  EMOJI_CHOICES,
  INSTRUCTION_TEMPLATE,
  RETRY_MAX,
  WEB_TOOLS,
  avatarEmoji,
  toolCatalog,
  validateAgent,
  normalizeAgent,
} from '../../lib/agents';
import type { AgentApproval, AgentBudget, AgentDraft, CapabilityReport, CustomAgent, ModelInfo } from '../../lib/types';
import { useAgents } from '../../store/agents';
import { useApp } from '../../store/app';
import { Button, Callout, Field, Modal, Segmented, Switch } from '../../components/ui';
import { AgentAvatar, ChipInput, Section } from './parts';

function ErrorText({ children }: { children?: ReactNode }) {
  return children ? <div className="field-error" role="alert">{children}</div> : null;
}

export default function AgentCreator({ initial, onClose, onSaved }: { initial: AgentDraft; onClose: () => void; onSaved?: (a: CustomAgent) => void }) {
  const advanced = useApp((s) => (s.settings?.uiMode ?? 'simple') === 'advanced');
  const saveAgent = useAgents((s) => s.saveAgent);
  const [draft, setDraft] = useState<AgentDraft>(initial);
  const [submitted, setSubmitted] = useState(false);
  const [saving, setSaving] = useState(false);
  const [models, setModels] = useState<ModelInfo[] | null>(null);
  const [modelErr, setModelErr] = useState<string | null>(null);
  const [report, setReport] = useState<CapabilityReport | null>(null);
  const editing = !!initial.id;
  const patch = (p: Partial<AgentDraft>) => setDraft((d) => ({ ...d, ...p }));

  useEffect(() => {
    if (!advanced) return;
    let alive = true;
    api
      .listModels(false)
      .then((l) => alive && setModels(l.models))
      .catch((e) => alive && setModelErr(errorMessage(e)));
    api
      .capabilityReport()
      .then((r) => alive && setReport(r))
      .catch(() => undefined); // the mirrored tool registry is used instead
    return () => {
      alive = false;
    };
  }, [advanced]);

  const errors = useMemo(() => validateAgent(draft), [draft]);
  const shown = (k: string) => (submitted ? errors[k] : undefined);
  const tools = useMemo(() => toolCatalog(report), [report]);
  const readTools = tools.filter((t) => t.readOnly);
  const writeTools = tools.filter((t) => !t.readOnly);
  const liveTools = (report?.agentTools?.length ?? 0) > 0;

  const toggleTool = (name: string, on: boolean) => patch({ tools: on ? [...draft.tools, name] : draft.tools.filter((t) => t !== name) });
  const setBudget = (k: keyof AgentBudget, raw: string) => patch({ budget: { ...draft.budget, [k]: raw === '' ? NaN : Number(raw) } });

  const save = async () => {
    setSubmitted(true);
    if (Object.keys(errors).length) {
      // Move focus to the first problem so keyboard users land on it.
      requestAnimationFrame(() => document.querySelector<HTMLElement>('[aria-invalid="true"], .field-error')?.closest('.field, .ag-section')?.querySelector<HTMLElement>('input, textarea, select, button')?.focus());
      return;
    }
    setSaving(true);
    const saved = await saveAgent(normalizeAgent(draft));
    setSaving(false);
    if (saved) {
      onSaved?.(saved);
      onClose();
    }
  };

  const modelOptions = models ?? [];
  const withCurrent = draft.model && !modelOptions.some((m) => m.id === draft.model) ? [{ id: draft.model } as ModelInfo, ...modelOptions] : modelOptions;

  return (
    <Modal
      wide
      title={editing ? `Edit ${initial.name || 'agent'}` : 'Create an agent'}
      onClose={onClose}
      footer={
        <>
          {submitted && Object.keys(errors).length > 0 && <span className="small" style={{ color: 'var(--bad)' }} role="status">{Object.keys(errors).length} thing(s) to fix</span>}
          <span className="grow" />
          <Button variant="ghost" onClick={onClose}>Cancel</Button>
          <Button variant="primary" onClick={() => void save()} loading={saving}>{editing ? 'Save changes' : 'Create agent'}</Button>
        </>
      }
    >
      <div className="ag-form">
        <Section title="Who is this agent?">
          <div className="ag-basics">
            <div className="field">
              <span className="label-like">Avatar</span>
              <div className="ag-avatar-pick" role="radiogroup" aria-label="Avatar">
                {AVATAR_PRESETS.map((p) => (
                  <button key={p.key} type="button" role="radio" aria-checked={draft.avatar === p.key} aria-label={`${p.label} avatar`} className={draft.avatar === p.key ? 'on' : ''} onClick={() => patch({ avatar: p.key })}>
                    {p.key === 'spark' ? <AgentAvatar avatar="spark" size={32} /> : <span style={{ fontSize: 20 }}>{p.emoji}</span>}
                  </button>
                ))}
                {EMOJI_CHOICES.map((e) => (
                  <button key={e} type="button" role="radio" aria-checked={draft.avatar === e} aria-label={`Emoji ${e}`} className={draft.avatar === e ? 'on' : ''} onClick={() => patch({ avatar: e })}>
                    <span style={{ fontSize: 20 }}>{e}</span>
                  </button>
                ))}
              </div>
              <div className="row">
                <label htmlFor="ag-emoji" className="small muted">Or your own emoji</label>
                <input id="ag-emoji" className="input" style={{ width: 70 }} maxLength={8} value={AVATAR_PRESETS.some((p) => p.key === draft.avatar) ? '' : draft.avatar} placeholder={avatarEmoji(draft.avatar).slice(0, 2)} onChange={(e) => patch({ avatar: e.target.value.trim() })} />
              </div>
              <ErrorText>{shown('avatar')}</ErrorText>
            </div>
            <div className="col grow" style={{ gap: 12 }}>
              <Field label="Name" htmlFor="ag-name" hint="Shown everywhere, e.g. “Pixel”.">
                <input id="ag-name" className="input" value={draft.name} maxLength={80} onChange={(e) => patch({ name: e.target.value })} aria-invalid={shown('name') ? true : undefined} autoFocus />
                <ErrorText>{shown('name')}</ErrorText>
              </Field>
              <Field label="Role" htmlFor="ag-role" hint="A few words: what is this agent in the team?">
                <input id="ag-role" className="input" value={draft.role} maxLength={100} onChange={(e) => patch({ role: e.target.value })} aria-invalid={shown('role') ? true : undefined} placeholder="UI designer" />
                <ErrorText>{shown('role')}</ErrorText>
              </Field>
            </div>
          </div>
          <Field label="Description" htmlFor="ag-desc" hint="One or two sentences for people (not sent as instructions).">
            <textarea id="ag-desc" className="textarea" rows={2} value={draft.description} onChange={(e) => patch({ description: e.target.value })} aria-invalid={shown('description') ? true : undefined} />
            <ErrorText>{shown('description')}</ErrorText>
          </Field>
        </Section>

        <Section title="How it works" hint="These instructions are what the agent follows on every task.">
          <Field label="Instructions" htmlFor="ag-instr">
            <textarea id="ag-instr" className="textarea" rows={9} value={draft.instructions} onChange={(e) => patch({ instructions: e.target.value })} aria-invalid={shown('instructions') ? true : undefined} placeholder="Describe how this agent should work…" />
            <div className="row">
              <Button type="button" size="sm" icon={<Wand2 size={13} />} onClick={() => {
                  if (draft.instructions.trim() && !window.confirm('Replace the current instructions with the template?')) return;
                  patch({ instructions: INSTRUCTION_TEMPLATE.replace('{role}', draft.role.trim() || 'assistant') });
                }}>
                Insert template
              </Button>
              <span className="grow" />
              <span className="tiny faint">{draft.instructions.length} / 8000</span>
            </div>
            <ErrorText>{shown('instructions')}</ErrorText>
          </Field>
          <ChipInput id="ag-spec" label="Specializations" values={draft.specialization} onChange={(v) => patch({ specialization: v })} placeholder="e.g. accessibility, charts, SQL" error={shown('specialization')} hint="Press Enter to add. Helps coordinators pick the right agent." />
          <ChipInput id="ag-know" label="Knowledge sources" values={draft.knowledgeSources} onChange={(v) => patch({ knowledgeSources: v })} placeholder="docs/guide.md or https://example.com/spec" error={shown('knowledgeSources')} hint="Project paths (relative) or https links the agent may read. Web content is treated as untrusted." />
        </Section>

        {!advanced ? (
          <Callout tone="info" title="Safe defaults are on">
            This agent starts with read-only tools, asks before changing files, and uses automatic model selection with sensible limits. Switch to Advanced mode (Settings or the toggle in a project) to change tools, budgets, permissions and more.
          </Callout>
        ) : (
          <>
            <Section title="Model" hint="Advanced: most agents should use Auto.">
              <Segmented
                label="Model policy"
                value={draft.modelPolicy}
                onChange={(v) => patch({ modelPolicy: v, model: v === 'manual' ? draft.model : null })}
                options={[{ value: 'auto', label: 'Auto' }, { value: 'max', label: 'Max quality' }, { value: 'manual', label: 'Manual' }]}
              />
              <div className="small muted">
                {draft.modelPolicy === 'auto' ? 'Genius picks a good model for each job and falls back if one is unavailable.' : draft.modelPolicy === 'max' ? 'Always prefers the strongest available model (slower, uses more tokens).' : 'Always uses the model you choose (advanced). If it is unavailable the task fails instead of switching.'}
              </div>
              {draft.modelPolicy === 'manual' && (
                <Field label="Model (advanced)" htmlFor="ag-model">
                  <select id="ag-model" className="select" value={draft.model ?? ''} onChange={(e) => patch({ model: e.target.value || null })} aria-invalid={shown('model') ? true : undefined}>
                    <option value="">Choose a model…</option>
                    {withCurrent.map((m) => (
                      <option key={m.id} value={m.id}>{m.id}</option>
                    ))}
                  </select>
                  {modelErr && <div className="hint">Could not load the model list ({modelErr}).</div>}
                  {!modelErr && models && models.length === 0 && <div className="hint">No models are loaded yet. Refresh the model list in Settings, then come back.</div>}
                  <ErrorText>{shown('model')}</ErrorText>
                </Field>
              )}
            </Section>

            <Section title="Tools" hint={liveTools ? 'The tools Genius currently offers.' : 'The tools Genius offers to agents.'}>
              <fieldset className="tool-group">
                <legend>Read-only (cannot change anything)</legend>
                {readTools.map((t) => (
                  <label key={t.name} className="tool-row">
                    <input type="checkbox" checked={draft.tools.includes(t.name)} onChange={(e) => toggleTool(t.name, e.target.checked)} />
                    <span><b className="mono">{t.name}</b><span className="muted"> — {t.description}</span></span>
                  </label>
                ))}
              </fieldset>
              <fieldset className="tool-group write">
                <legend>Can change files or run code</legend>
                {writeTools.map((t) => (
                  <label key={t.name} className="tool-row">
                    <input type="checkbox" checked={draft.tools.includes(t.name)} onChange={(e) => toggleTool(t.name, e.target.checked)} />
                    <span><b className="mono">{t.name}</b><span className="muted"> — {t.description}</span></span>
                  </label>
                ))}
              </fieldset>
              <ErrorText>{shown('tools')}</ErrorText>
            </Section>

            <Section title="Permissions">
              <Field label="Where may it work?">
                <Segmented label="Project scope" value={draft.scope.projects} onChange={(v) => patch({ scope: { ...draft.scope, projects: v } })} options={[{ value: 'current', label: 'Only the project of the task' }, { value: 'any', label: 'Any project' }]} />
              </Field>
              <ChipInput id="ag-scope" label="Allowed folders" values={draft.scope.paths} onChange={(v) => patch({ scope: { ...draft.scope, paths: v } })} placeholder="src/components" error={shown('scope')} hint="Project-relative folders. Empty means the whole project." />
              <div className="row">
                <Switch checked={draft.network} onChange={(v) => patch({ network: v, tools: v ? draft.tools : draft.tools.filter((t) => !WEB_TOOLS.includes(t)) })} label="Network access" />
                <span><b>Network access</b> <span className="muted">— lets web tools read public pages (your secrets are never sent).</span></span>
              </div>
              <Field label="Approval mode" htmlFor="ag-approval">
                <select id="ag-approval" className="select" value={draft.approval} onChange={(e) => patch({ approval: e.target.value as AgentApproval })}>
                  {(Object.keys(APPROVAL_LABEL) as AgentApproval[]).map((k) => (
                    <option key={k} value={k}>{APPROVAL_LABEL[k]}</option>
                  ))}
                </select>
              </Field>
              <Field label="Memory" htmlFor="ag-memory" hint="What the agent may remember between tasks.">
                <select id="ag-memory" className="select" value={draft.memoryPolicy} onChange={(e) => patch({ memoryPolicy: e.target.value as AgentDraft['memoryPolicy'] })}>
                  <option value="none">None — every task starts fresh</option>
                  <option value="project">Project memory — shared decisions and conventions</option>
                  <option value="agent">Its own memory — notes only this agent sees</option>
                </select>
              </Field>
            </Section>

            <Section title="Budgets" hint="Hard limits per task. The agent stops when any limit is reached.">
              <div className="ag-budgets">
                {(Object.keys(BUDGET_LABELS) as (keyof AgentBudget)[]).map((k) => (
                  <Field key={k} label={`${BUDGET_LABELS[k].label} (max ${BUDGET_MAX[k].toLocaleString()})`} htmlFor={`ag-b-${k}`} hint={BUDGET_LABELS[k].hint}>
                    <input id={`ag-b-${k}`} type="number" className="input" min={1} max={BUDGET_MAX[k]} value={Number.isNaN(draft.budget[k]) ? '' : draft.budget[k]} onChange={(e) => setBudget(k, e.target.value)} aria-invalid={shown(`budget.${k}`) ? true : undefined} />
                    <ErrorText>{shown(`budget.${k}`)}</ErrorText>
                  </Field>
                ))}
                <Field label={`Retry limit (max ${RETRY_MAX})`} htmlFor="ag-retry" hint="How often a failed task may be retried automatically.">
                  <input id="ag-retry" type="number" className="input" min={0} max={RETRY_MAX} value={Number.isNaN(draft.retryLimit) ? '' : draft.retryLimit} onChange={(e) => patch({ retryLimit: e.target.value === '' ? NaN : Number(e.target.value) })} aria-invalid={shown('retryLimit') ? true : undefined} />
                  <ErrorText>{shown('retryLimit')}</ErrorText>
                </Field>
              </div>
            </Section>

            <Section title="Quality bar">
              <Field label="Acceptance criteria" htmlFor="ag-accept" hint="When is a task good enough? e.g. “builds without errors, no new accessibility issues”.">
                <textarea id="ag-accept" className="textarea" rows={3} value={draft.acceptanceCriteria} onChange={(e) => patch({ acceptanceCriteria: e.target.value })} aria-invalid={shown('acceptanceCriteria') ? true : undefined} />
                <ErrorText>{shown('acceptanceCriteria')}</ErrorText>
              </Field>
              <Field label="Output requirements" htmlFor="ag-output" hint="How should results be written? e.g. “a short summary with a list of changed files”.">
                <textarea id="ag-output" className="textarea" rows={3} value={draft.outputRequirements} onChange={(e) => patch({ outputRequirements: e.target.value })} aria-invalid={shown('outputRequirements') ? true : undefined} />
                <ErrorText>{shown('outputRequirements')}</ErrorText>
              </Field>
            </Section>
          </>
        )}
      </div>
    </Modal>
  );
}
