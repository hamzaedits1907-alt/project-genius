import { useMemo, useState } from 'react';
import { Pencil, RefreshCw, Wand2 } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import type { ProjectSpec } from '../../lib/types';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Button, Callout, Field, Spinner } from '../../components/ui';

const toLines = (items: { name: string; description?: string; reason?: string }[]) => items.map((i) => `${i.name}${i.description || i.reason ? `: ${i.description ?? i.reason}` : ''}`).join('\n');
const splitLine = (l: string): [string, string] => {
  const i = l.indexOf(':');
  return i < 0 ? [l.trim(), ''] : [l.slice(0, i).trim(), l.slice(i + 1).trim()];
};
const lines = (s: string) => s.split('\n').map((x) => x.trim()).filter(Boolean);

export default function PlanView() {
  const project = useWorkspace((s) => s.project);
  const tasks = useWorkspace((s) => s.tasks);
  const aiStream = useWorkspace((s) => s.aiStream);
  const activity = useWorkspace((s) => s.activity);
  const refresh = useWorkspace((s) => s.refreshProject);
  const set = useWorkspace((s) => s.set);
  const toast = useApp((s) => s.toast);
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);

  const planTask = tasks.find((t) => t.kind === 'plan');
  const genTask = tasks.find((t) => t.kind === 'generate');
  const lastPlanFailure = useMemo(() => [...activity].reverse().find((e) => e.kind === 'task' && e.detail.taskKind === 'plan'), [activity]);

  if (!project) return null;
  const spec = project.spec;

  if (planTask) {
    const text = aiStream[planTask.id] ?? '';
    return (
      <div className="plan">
        <div className="row">
          <Spinner />
          <h2 className="grow">Preparing the implementation plan…</h2>
          <Button size="sm" variant="ghost" onClick={() => void api.cancelTask(planTask.id)}>
            Cancel
          </Button>
        </div>
        <p className="muted">The model's response streams below as it arrives. It becomes an editable plan when complete.</p>
        <pre className="stream" style={{ maxHeight: 'none' }}>{text || 'Waiting for the first tokens…'}</pre>
      </div>
    );
  }

  if (!spec) {
    const err = lastPlanFailure?.status === 'failed' ? (lastPlanFailure.detail.error as { message?: string } | undefined)?.message : null;
    const retry = async () => {
      setBusy(true);
      try {
        await api.startPlan(project.id, project.description, '');
      } catch (e) {
        toast('bad', errorMessage(e));
      } finally {
        setBusy(false);
      }
    };
    return (
      <div className="plan">
        <h2>No plan yet</h2>
        {err && <Callout tone="bad" title="Planning failed">{err}</Callout>}
        {project.description ? (
          <>
            <p className="muted">{project.description}</p>
            <Button variant="primary" icon={<RefreshCw size={14} />} loading={busy} onClick={() => void retry()}>
              {err ? 'Try again' : 'Create plan'}
            </Button>
          </>
        ) : (
          <p className="muted">This project was opened from a folder. Use the Assistant to request changes to its code.</p>
        )}
      </div>
    );
  }

  const generate = async () => {
    setBusy(true);
    try {
      await api.startGenerate(project.id);
      set({ bottomTab: 'activity', rightTab: 'assistant' });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  if (editing) return <PlanEditor spec={spec} onDone={async () => (setEditing(false), await refresh())} />;
  const generated = project.status.stage === 'generated';

  return (
    <div className="scroll" style={{ height: '100%' }}>
      <div className="plan">
        <div className="row">
          <h2 className="grow">{spec.name}</h2>
          <Button size="sm" icon={<Pencil size={13} />} onClick={() => setEditing(true)} disabled={!!genTask}>
            Edit plan
          </Button>
          <Button variant="primary" icon={<Wand2 size={14} />} loading={busy || !!genTask} onClick={() => void generate()}>
            {genTask ? 'Generating…' : generated ? 'Regenerate (review first)' : 'Generate application'}
          </Button>
        </div>
        <p className="muted" style={{ fontSize: 14 }}>
          {spec.purpose}
        </p>
        {!generated && <Callout tone="info">Review and edit the plan. Generation writes real files into the project folder, then installs dependencies (with your approval), builds and starts the app.</Callout>}
        <h3>Features</h3>
        <ul>
          {spec.features.map((f) => (
            <li key={f.name}>
              <strong>{f.name}</strong> {f.description && <span className="muted">— {f.description}</span>}
            </li>
          ))}
        </ul>
        {spec.pages.length > 0 && (
          <>
            <h3>Pages</h3>
            <ul>
              {spec.pages.map((p) => (
                <li key={p.name}>
                  <strong>{p.name}</strong> <span className="muted">— {p.description}</span> {p.components.length > 0 && <span className="faint small">({p.components.join(', ')})</span>}
                </li>
              ))}
            </ul>
          </>
        )}
        {spec.components.length > 0 && (
          <>
            <h3>Components</h3>
            <div className="chips">
              {spec.components.map((c) => (
                <span className="chip" key={c.name} title={c.description}>
                  {c.name}
                </span>
              ))}
            </div>
          </>
        )}
        {spec.dataModels.length > 0 && (
          <>
            <h3>Data models</h3>
            <ul>
              {spec.dataModels.map((d) => (
                <li key={d.name}>
                  <strong>{d.name}</strong> <span className="mono faint small">{d.fields.join(' · ')}</span>
                  {d.relations.length > 0 && <div className="faint small">{d.relations.join('; ')}</div>}
                </li>
              ))}
            </ul>
          </>
        )}
        <h3>Architecture</h3>
        <p className="muted" style={{ whiteSpace: 'pre-wrap', margin: 0 }}>
          {spec.architecture || '—'}
        </p>
        <h3>Dependencies</h3>
        {spec.dependencies.length === 0 ? (
          <p className="muted" style={{ margin: 0 }}>
            None beyond React.
          </p>
        ) : (
          <ul>
            {spec.dependencies.map((d) => (
              <li key={d.name}>
                <span className="mono">{d.name}</span> <span className="muted">— {d.reason}</span>
              </li>
            ))}
          </ul>
        )}
        {spec.milestones.length > 0 && (
          <>
            <h3>Milestones</h3>
            <ol style={{ margin: 0, paddingLeft: 18 }}>
              {spec.milestones.map((m) => (
                <li key={m}>{m}</li>
              ))}
            </ol>
          </>
        )}
        <h3>Test strategy</h3>
        <p className="muted" style={{ margin: 0 }}>
          {spec.testStrategy || '—'}
        </p>
        {(spec.assumptions.length > 0 || spec.risks.length > 0) && (
          <div className="row" style={{ alignItems: 'flex-start', gap: 24 }}>
            {spec.assumptions.length > 0 && (
              <div className="grow">
                <h3>Assumptions</h3>
                <ul>
                  {spec.assumptions.map((a) => (
                    <li key={a}>{a}</li>
                  ))}
                </ul>
              </div>
            )}
            {spec.risks.length > 0 && (
              <div className="grow">
                <h3>Risks</h3>
                <ul>
                  {spec.risks.map((a) => (
                    <li key={a}>{a}</li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        )}
        {spec.files.length > 0 && (
          <>
            <h3>Planned files</h3>
            <ul>
              {spec.files.map((f) => (
                <li key={f.path}>
                  <span className="mono small">{f.path}</span> <span className="faint small">— {f.purpose}</span>
                </li>
              ))}
            </ul>
          </>
        )}
      </div>
    </div>
  );
}

function PlanEditor({ spec, onDone }: { spec: ProjectSpec; onDone: () => Promise<void> }) {
  const project = useWorkspace((s) => s.project)!;
  const toast = useApp((s) => s.toast);
  const [f, setF] = useState({
    name: spec.name,
    purpose: spec.purpose,
    features: toLines(spec.features),
    pages: toLines(spec.pages),
    components: toLines(spec.components),
    dataModels: spec.dataModels.map((d) => `${d.name}: ${d.fields.join(', ')}`).join('\n'),
    architecture: spec.architecture,
    dependencies: toLines(spec.dependencies),
    milestones: spec.milestones.join('\n'),
    testStrategy: spec.testStrategy,
    assumptions: spec.assumptions.join('\n'),
    risks: spec.risks.join('\n'),
  });
  const [busy, setBusy] = useState(false);
  const save = async () => {
    const next: ProjectSpec = {
      ...spec,
      name: f.name.trim(),
      purpose: f.purpose.trim(),
      features: lines(f.features).map((l) => {
        const [name, description] = splitLine(l);
        return { name, description };
      }),
      pages: lines(f.pages).map((l) => {
        const [name, description] = splitLine(l);
        return { name, description, components: spec.pages.find((p) => p.name === name)?.components ?? [] };
      }),
      components: lines(f.components).map((l) => {
        const [name, description] = splitLine(l);
        return { name, description };
      }),
      dataModels: lines(f.dataModels).map((l) => {
        const [name, rest] = splitLine(l);
        return { name, fields: rest.split(',').map((x) => x.trim()).filter(Boolean), relations: spec.dataModels.find((d) => d.name === name)?.relations ?? [] };
      }),
      architecture: f.architecture,
      dependencies: lines(f.dependencies).map((l) => {
        const [name, reason] = splitLine(l);
        return { name, reason };
      }),
      milestones: lines(f.milestones),
      testStrategy: f.testStrategy,
      assumptions: lines(f.assumptions),
      risks: lines(f.risks),
    };
    setBusy(true);
    try {
      await api.updateSpec(project.id, next);
      toast('ok', 'Plan updated');
      await onDone();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  const area = (k: keyof typeof f, label: string, hint?: string, rows = 4) => (
    <Field label={label} hint={hint}>
      <textarea className="textarea" rows={rows} value={f[k]} onChange={(e) => setF({ ...f, [k]: e.target.value })} />
    </Field>
  );
  return (
    <div className="scroll" style={{ height: '100%' }}>
      <div className="plan col" style={{ gap: 14 }}>
        <div className="row">
          <h2 className="grow">Edit plan</h2>
          <Button onClick={() => void onDone()}>Cancel</Button>
          <Button variant="primary" loading={busy} onClick={() => void save()}>
            Save plan
          </Button>
        </div>
        <Field label="Name">
          <input className="input" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
        </Field>
        {area('purpose', 'Purpose', undefined, 2)}
        {area('features', 'Features', 'One per line — “Name: description”', 6)}
        {area('pages', 'Pages', 'One per line — “Name: description”')}
        {area('components', 'Components', 'One per line — “Name: description”')}
        {area('dataModels', 'Data models', 'One per line — “Model: field: type, field: type”')}
        {area('architecture', 'Architecture')}
        {area('dependencies', 'Dependencies', 'npm package names, one per line — “package: reason”', 3)}
        {area('milestones', 'Milestones', 'One per line', 3)}
        {area('testStrategy', 'Test strategy', undefined, 2)}
        {area('assumptions', 'Assumptions', 'One per line', 3)}
        {area('risks', 'Risks', 'One per line', 3)}
      </div>
    </div>
  );
}
