import { useMemo, useState } from 'react';
import CategoryChart from './components/CategoryChart';
import SummaryCards from './components/SummaryCards';
import TransactionForm from './components/TransactionForm';
import TransactionList from './components/TransactionList';
import { useTransactions } from './hooks/useTransactions';
import { money } from './lib/format';
import { byCategory, summarize } from './lib/summary';
import type { Draft, Transaction } from './types';

type View = 'dashboard' | 'transactions' | 'categories';

const VIEWS: { id: View; label: string }[] = [
  { id: 'dashboard', label: 'Dashboard' },
  { id: 'transactions', label: 'Transactions' },
  { id: 'categories', label: 'Categories' },
];

export default function App() {
  const { items, add, update, remove } = useTransactions();
  const [view, setView] = useState<View>('dashboard');
  const [editing, setEditing] = useState<Transaction | null>(null);
  const summary = useMemo(() => summarize(items), [items]);
  const categories = useMemo(() => byCategory(items), [items]);

  const submit = (d: Draft) => {
    if (editing) {
      update(editing.id, d);
      setEditing(null);
    } else {
      add(d);
    }
  };

  return (
    <div className="shell">
      <header className="top">
        <h1>Personal Finance Tracker</h1>
        <nav aria-label="Views">
          {VIEWS.map((v) => (
            <button key={v.id} className={view === v.id ? 'tab on' : 'tab'} aria-current={view === v.id ? 'page' : undefined} onClick={() => setView(v.id)}>
              {v.label}
            </button>
          ))}
        </nav>
      </header>
      <div className="layout">
        <aside>
          <TransactionForm editing={editing} onSubmit={submit} onCancel={() => setEditing(null)} />
        </aside>
        <main>
          {view === 'dashboard' && (
            <>
              <SummaryCards summary={summary} />
              <section className="panel">
                <CategoryChart rows={categories} />
              </section>
              <section className="panel" aria-label="Recent activity">
                <h2>Recent activity</h2>
                <ul className="recent">
                  {items.slice(0, 5).map((t) => (
                    <li key={t.id}>
                      <span>{t.description}</span>
                      <span className={t.type}>{t.type === 'expense' ? '−' : '+'}{money(t.amount)}</span>
                    </li>
                  ))}
                </ul>
              </section>
            </>
          )}
          {view === 'transactions' && <TransactionList items={items} onEdit={setEditing} onDelete={remove} />}
          {view === 'categories' && (
            <section className="panel">
              <h2>Where the money goes</h2>
              <CategoryChart rows={categories} />
              <div className="table-scroll">
              <table className="tx">
                <tbody>
                  {categories.map((c) => (
                    <tr key={c.category} data-testid="category-row"><td>{c.category}</td><td className="num">{money(c.total)}</td></tr>
                  ))}
                </tbody>
              </table>
              </div>
            </section>
          )}
        </main>
      </div>
    </div>
  );
}
