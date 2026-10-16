import { useState } from 'react';
import { ArrowRight, Globe, Monitor } from 'lucide-react';
import { api, errorMessage } from '../lib/api';
import { useApp } from '../store/app';
import { Button, Callout, Field } from '../components/ui';

const EXAMPLES = [
  'A modern task manager with projects, a kanban board, search, due dates and a clean responsive interface.',
  'A personal finance tracker: add income and expenses, categories, monthly charts, and data saved in the browser.',
  'A recipe book where I can add recipes with ingredients and steps, tag them, and filter by tag.',
  'A pomodoro timer with configurable intervals, session history and a minimal dark design.',
];

export default function CreateProject() {
  const navigate = useApp((s) => s.navigate);
  const settings = useApp((s) => s.settings);
  const keyStatus = useApp((s) => s.keyStatus);
  const toast = useApp((s) => s.toast);
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [tech, setTech] = useState('');
  const [design, setDesign] = useState('');
  const [constraints, setConstraints] = useState('');
  const [busy, setBusy] = useState(false);

  const ready = !!keyStatus?.stored && !!settings?.model;
  const valid = name.trim().length > 0 && description.trim().length >= 10;

  const submit = async () => {
    setBusy(true);
    try {
      const p = await api.createProject(name, description);
      const extra = [tech && `Preferred technologies: ${tech}`, design && `Design preferences: ${design}`, constraints && `Other constraints: ${constraints}`].filter(Boolean).join('\n');
      await api.startPlan(p.id, description, extra);
      navigate({ name: 'project', id: p.id });
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <div className="page-inner" style={{ maxWidth: 820 }}>
        <h1>Create a new application</h1>
        <p className="lead">Describe it in plain language. Genius will prepare a plan you can review and edit before any code is written.</p>
        <div className="steps" aria-label="Progress">
          <div className="step on">
            <span className="n">1</span>Describe
          </div>
          <div className="step">
            <span className="n">2</span>Review plan
          </div>
          <div className="step">
            <span className="n">3</span>Generate & run
          </div>
        </div>

        {!ready && (
          <div style={{ marginBottom: 16 }}>
            <Callout tone="warn" title="AI provider not configured">
              {!keyStatus?.stored ? 'Save your NVIDIA API key' : 'Choose a model'} in Settings before planning.{' '}
              <a href="#" onClick={(e) => (e.preventDefault(), navigate({ name: 'settings', tab: 'ai' }))}>
                Open AI settings
              </a>
            </Callout>
          </div>
        )}

        <div className="card pad col" style={{ gap: 18 }}>
          <Field label="Project name" htmlFor="pname">
            <input id="pname" className="input" maxLength={80} value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. TaskFlow" autoFocus />
          </Field>
          <Field label="What should it do?" htmlFor="pdesc" hint="Features, pages, who uses it. Genius makes reasonable decisions for anything you leave out and lists them as assumptions.">
            <textarea id="pdesc" className="textarea" rows={6} value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Build a modern task management application with user profiles, projects, task boards, search…" />
          </Field>
          <div className="examples" aria-label="Examples">
            {EXAMPLES.map((ex) => (
              <button key={ex} onClick={() => setDescription(ex)}>
                {ex.split(':')[0].split(' with')[0].slice(0, 48)}…
              </button>
            ))}
          </div>
          <Field label="Target platform">
            <div className="row">
              <span className="badge accent" style={{ height: 30, padding: '0 12px' }}>
                <Globe size={13} /> Web application (Vite + React + TypeScript)
              </span>
              <span className="badge" style={{ height: 30, padding: '0 12px' }} title="Not supported yet">
                <Monitor size={13} /> Desktop app — not yet supported
              </span>
            </div>
          </Field>
          <div className="row" style={{ gap: 14, alignItems: 'flex-start' }}>
            <div className="grow">
              <Field label="Preferred technologies (optional)" htmlFor="ptech">
                <input id="ptech" className="input" value={tech} onChange={(e) => setTech(e.target.value)} placeholder="e.g. React Router, no UI library" />
              </Field>
            </div>
            <div className="grow">
              <Field label="Design preferences (optional)" htmlFor="pdesign">
                <input id="pdesign" className="input" value={design} onChange={(e) => setDesign(e.target.value)} placeholder="e.g. minimal, dark, rounded cards" />
              </Field>
            </div>
          </div>
          <Field label="Other constraints (optional)" htmlFor="pcons">
            <input id="pcons" className="input" value={constraints} onChange={(e) => setConstraints(e.target.value)} placeholder="e.g. must work offline, no external APIs" />
          </Field>
          <div className="row">
            <span className="faint small grow">Your description is sent to NVIDIA using model {settings?.model ?? '—'}.</span>
            <Button variant="primary" size="lg" disabled={!valid || !ready} loading={busy} onClick={() => void submit()} icon={<ArrowRight size={16} />}>
              Create plan
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
