import type { Transaction } from '../types';

const KEY = 'finance-tracker.v1';

export const seed: Transaction[] = [
  { id: 't1', date: '2026-10-01', description: 'Salary', amount: 3200, type: 'income', category: 'Salary' },
  { id: 't2', date: '2026-10-03', description: 'Rent', amount: 1100, type: 'expense', category: 'Housing' },
  { id: 't3', date: '2026-10-05', description: 'Groceries', amount: 164.5, type: 'expense', category: 'Food' },
  { id: 't4', date: '2026-10-09', description: 'Freelance project', amount: 450, type: 'income', category: 'Salary' },
  { id: 't5', date: '2026-10-12', description: 'Metro pass', amount: 55, type: 'expense', category: 'Transport' },
];

export function load(): Transaction[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return seed;
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as Transaction[]) : seed;
  } catch {
    return seed;
  }
}

export function save(items: Transaction[]): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(items));
  } catch {
    // storage unavailable (private mode, quota): the app keeps working in memory
  }
}
