import { useState } from 'react';
import './App.css';
import { DEFAULT_WIDGETS, KIND_LABELS, PERIODS } from './data';
import { useLocalStorage, useTheme } from './hooks/storage';
import { ChartWidget, MetricWidget, NotesWidget, TasksWidget, WidgetFrame } from './components/widgets';
import type { Period, Widget, WidgetKind } from './types';

function renderWidget(w: Widget, period: Period) {
  switch (w.kind) {
    case 'metrics':
      return <MetricWidget period={period} />;
    case 'chart':
      return <ChartWidget period={period} title={w.title} />;
    case 'tasks':
      return <TasksWidget id={w.id} />;
    case 'notes':
      return <NotesWidget id={w.id} />;
  }
}

export default function App() {
  const { theme, toggle } = useTheme();
  const [period, setPeriod] = useLocalStorage<Period>('dash.period', '7d');
  const [widgets, setWidgets] = useLocalStorage<Widget[]>('dash.widgets', DEFAULT_WIDGETS);
  const [kind, setKind] = useState<WidgetKind>('metrics');
  const add = () => setWidgets((ws) => [...ws, { id: `w${Date.now()}`, kind, title: `New ${KIND_LABELS[kind].toLowerCase()} ${ws.length + 1}` }]);
  const remove = (id: string) => setWidgets((ws) => ws.filter((w) => w.id !== id));
  return (
    <div className="app">
      <header className="topbar">
        <h1>Team dashboard</h1>
        <div className="controls">
          <label>
            Period{' '}
            <select id="period" value={period} onChange={(e) => setPeriod(e.target.value as Period)}>
              {PERIODS.map((p) => (
                <option key={p.value} value={p.value}>
                  {p.label}
                </option>
              ))}
            </select>
          </label>
          <button type="button" onClick={toggle}>
            {theme === 'light' ? 'Switch to dark theme' : 'Switch to light theme'}
          </button>
        </div>
      </header>
      <main>
        <form
          className="adder"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <label>
            Widget type{' '}
            <select id="add-kind" value={kind} onChange={(e) => setKind(e.target.value as WidgetKind)}>
              {(Object.keys(KIND_LABELS) as WidgetKind[]).map((k) => (
                <option key={k} value={k}>
                  {KIND_LABELS[k]}
                </option>
              ))}
            </select>
          </label>
          <button type="submit">Add widget</button>
        </form>
        <div className="grid">
          {widgets.map((w) => (
            <WidgetFrame key={w.id} id={w.id} title={w.title} onRemove={() => remove(w.id)}>
              {renderWidget(w, period)}
            </WidgetFrame>
          ))}
        </div>
      </main>
    </div>
  );
}
