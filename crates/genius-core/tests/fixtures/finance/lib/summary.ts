import type { Transaction, TxType } from '../types';

export interface Summary {
  income: number;
  expenses: number;
  balance: number;
}

const round = (n: number): number => Math.round(n * 100) / 100;

export function summarize(items: Transaction[]): Summary {
  let income = 0;
  let expenses = 0;
  for (const t of items) {
    if (t.type === 'income') income += t.amount;
    else expenses += t.amount;
  }
  return { income: round(income), expenses: round(expenses), balance: round(income - expenses) };
}

/** Expense totals per category, largest first. */
export function byCategory(items: Transaction[]): { category: string; total: number }[] {
  const totals = new Map<string, number>();
  for (const t of items) {
    if (t.type === 'expense') totals.set(t.category, (totals.get(t.category) ?? 0) + t.amount);
  }
  return [...totals.entries()].map(([category, total]) => ({ category, total: round(total) })).sort((a, b) => b.total - a.total);
}

export function filterTransactions(items: Transaction[], query: string, type: 'all' | TxType, category: string): Transaction[] {
  const q = query.trim().toLowerCase();
  return items.filter((t) => (type === 'all' || t.type === type) && (category === 'all' || t.category === category) && (q === '' || t.description.toLowerCase().includes(q)));
}
