import { useMemo, useState } from 'react';
import { money } from '../lib/format';
import { filterTransactions } from '../lib/summary';
import { CATEGORIES, type Transaction, type TxType } from '../types';

type Props = { items: Transaction[]; onEdit: (t: Transaction) => void; onDelete: (id: string) => void };

export default function TransactionList({ items, onEdit, onDelete }: Props) {
  const [query, setQuery] = useState('');
  const [type, setType] = useState<'all' | TxType>('all');
  const [category, setCategory] = useState('all');
  const rows = useMemo(() => filterTransactions(items, query, type, category), [items, query, type, category]);

  return (
    <section className="panel" aria-label="Transactions">
      <div className="filters">
        <input id="tx-search" type="search" placeholder="Search transactions" aria-label="Search transactions" value={query} onChange={(e) => setQuery(e.target.value)} />
        <select id="tx-filter-type" aria-label="Filter by type" value={type} onChange={(e) => setType(e.target.value as 'all' | TxType)}>
          <option value="all">All types</option>
          <option value="income">Income</option>
          <option value="expense">Expenses</option>
        </select>
        <select id="tx-filter-category" aria-label="Filter by category" value={category} onChange={(e) => setCategory(e.target.value)}>
          <option value="all">All categories</option>
          {CATEGORIES.map((c) => (
            <option key={c} value={c}>{c}</option>
          ))}
        </select>
      </div>
      {rows.length === 0 ? (
        <p className="muted">No transactions match.</p>
      ) : (
        <div className="table-scroll">
        <table className="tx">
          <thead>
            <tr><th>Date</th><th>Description</th><th>Category</th><th className="num">Amount</th><th /></tr>
          </thead>
          <tbody>
            {rows.map((t) => (
              <tr key={t.id} data-testid="tx-row">
                <td>{t.date}</td>
                <td>{t.description}</td>
                <td>{t.category}</td>
                <td className={`num ${t.type}`}>{t.type === 'expense' ? '−' : '+'}{money(t.amount)}</td>
                <td className="row-actions">
                  <button aria-label={`Edit ${t.description}`} onClick={() => onEdit(t)}>Edit</button>
                  <button aria-label={`Delete ${t.description}`} className="danger" onClick={() => onDelete(t.id)}>Delete</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        </div>
      )}
    </section>
  );
}
