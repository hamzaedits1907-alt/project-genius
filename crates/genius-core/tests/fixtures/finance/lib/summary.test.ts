import { describe, expect, it } from 'vitest';
import { byCategory, filterTransactions, summarize } from './summary';
import type { Transaction } from '../types';

const items: Transaction[] = [
  { id: '1', date: '2026-10-01', description: 'Salary', amount: 3000, type: 'income', category: 'Salary' },
  { id: '2', date: '2026-10-02', description: 'Rent', amount: 1000, type: 'expense', category: 'Housing' },
  { id: '3', date: '2026-10-03', description: 'Groceries', amount: 120.5, type: 'expense', category: 'Food' },
];

describe('summary', () => {
  it('adds income and expenses into a balance', () => {
    expect(summarize(items)).toEqual({ income: 3000, expenses: 1120.5, balance: 1879.5 });
  });

  it('groups expenses by category, largest first', () => {
    expect(byCategory(items).map((c) => c.category)).toEqual(['Housing', 'Food']);
  });

  it('filters by text, type and category', () => {
    expect(filterTransactions(items, 'rent', 'all', 'all')).toHaveLength(1);
    expect(filterTransactions(items, '', 'income', 'all')).toHaveLength(1);
    expect(filterTransactions(items, '', 'all', 'Food')).toHaveLength(1);
  });
});
