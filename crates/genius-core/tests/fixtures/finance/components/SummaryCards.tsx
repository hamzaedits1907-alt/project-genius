import { money } from '../lib/format';
import type { Summary } from '../lib/summary';

export default function SummaryCards({ summary }: { summary: Summary }) {
  return (
    <section className="cards" aria-label="Summary">
      <article className="card income">
        <h3>Income</h3>
        <p className="figure" data-testid="income">{money(summary.income)}</p>
      </article>
      <article className="card expense">
        <h3>Expenses</h3>
        <p className="figure" data-testid="expenses">{money(summary.expenses)}</p>
      </article>
      <article className="card balance">
        <h3>Balance</h3>
        <p className="figure" data-testid="balance">{money(summary.balance)}</p>
      </article>
    </section>
  );
}
