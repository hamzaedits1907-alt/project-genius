import { money } from '../lib/format';

type Row = { category: string; total: number };

export default function CategoryChart({ rows }: { rows: Row[] }) {
  if (rows.length === 0) return <p className="muted">No expenses yet.</p>;
  const max = Math.max(...rows.map((r) => r.total), 1);
  const rowH = 30;
  return (
    <figure className="chart">
      <figcaption>Spending by category</figcaption>
      <svg viewBox={`0 0 420 ${rows.length * rowH + 6}`} role="img" aria-label="Spending by category">
        {rows.map((r, i) => (
          <g key={r.category} transform={`translate(0 ${i * rowH + 4})`} data-testid="chart-bar">
            <text x="0" y="16" className="bar-label">{r.category}</text>
            <rect x="96" y="3" height="18" rx="4" width={Math.max(4, (r.total / max) * 230)} className="bar" />
            <text x={96 + Math.max(4, (r.total / max) * 230) + 6} y="16" className="bar-value">{money(r.total)}</text>
          </g>
        ))}
      </svg>
    </figure>
  );
}
