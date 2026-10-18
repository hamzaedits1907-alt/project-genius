import { useState, type FormEvent, type ReactNode } from 'react';
import { metricsFor, seriesFor } from '../data';
import { useLocalStorage } from '../hooks/storage';
import type { Period, Task } from '../types';

export function WidgetFrame({ id, title, onRemove, children }: { id: string; title: string; onRemove: () => void; children: ReactNode }) {
  const heading = `widget-title-${id}`;
  return (
    <section className="widget" aria-labelledby={heading} data-widget={id}>
      <header className="widget-head">
        <h2 id={heading}>{title}</h2>
        <button type="button" className="icon-btn" aria-label={`Remove ${title}`} onClick={onRemove}>
          ×
        </button>
      </header>
      {children}
    </section>
  );
}

export function MetricWidget({ period }: { period: Period }) {
  return (
    <dl className="metrics">
      {metricsFor(period).map((m) => (
        <div key={m.label} className="metric">
          <dt>{m.label}</dt>
          <dd>{`${m.currency ? '$' : ''}${m.value.toLocaleString('en-US')}`}</dd>
          <dd className="delta">{m.delta}</dd>
        </div>
      ))}
    </dl>
  );
}

export function ChartWidget({ period, title }: { period: Period; title: string }) {
  const points = seriesFor(period);
  const w = 320;
  const h = 140;
  const gap = 2;
  const max = Math.max(...points, 1);
  const bw = w / points.length - gap;
  return (
    <figure className="chart">
      <svg viewBox={`0 0 ${w} ${h}`} role="img" aria-label={`${title}: ${points.length} data points`} preserveAspectRatio="none">
        {points.map((p, i) => (
          <rect key={i} className="bar" x={i * (bw + gap)} y={h - (p / max) * h} width={bw} height={(p / max) * h} />
        ))}
      </svg>
      <figcaption>Peak value {max}</figcaption>
    </figure>
  );
}

export function TasksWidget({ id }: { id: string }) {
  const [tasks, setTasks] = useLocalStorage<Task[]>(`dash.w.${id}.tasks`, []);
  const [draft, setDraft] = useState('');
  const add = (e: FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text) return;
    setTasks((t) => [...t, { id: `${Date.now()}-${t.length}`, text, done: false }]);
    setDraft('');
  };
  const done = tasks.filter((t) => t.done).length;
  return (
    <div>
      <form onSubmit={add} className="row">
        <input aria-label="New task" placeholder="New task" value={draft} onChange={(e) => setDraft(e.target.value)} />
        <button type="submit">Add task</button>
      </form>
      <ul className="tasks">
        {tasks.map((t) => (
          <li key={t.id} className={t.done ? 'task done' : 'task'}>
            <label>
              <input type="checkbox" checked={t.done} onChange={() => setTasks((all) => all.map((x) => (x.id === t.id ? { ...x, done: !x.done } : x)))} /> {t.text}
            </label>
          </li>
        ))}
      </ul>
      <p className="muted">{`${tasks.length - done} open, ${done} done`}</p>
      <button type="button" disabled={done === 0} onClick={() => setTasks((all) => all.filter((x) => !x.done))}>
        Clear completed
      </button>
    </div>
  );
}

export function NotesWidget({ id }: { id: string }) {
  const [text, setText] = useLocalStorage<string>(`dash.w.${id}.notes`, '');
  return (
    <div>
      <textarea aria-label="Notes" rows={5} value={text} onChange={(e) => setText(e.target.value)} />
      <p className="muted">{`${text.length} characters`}</p>
    </div>
  );
}
