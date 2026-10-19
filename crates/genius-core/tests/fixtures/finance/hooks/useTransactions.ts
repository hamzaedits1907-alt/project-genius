import { useCallback, useEffect, useState } from 'react';
import { load, save } from '../lib/storage';
import type { Draft, Transaction } from '../types';

const newId = (): string => `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

export function useTransactions() {
  const [items, setItems] = useState<Transaction[]>(() => load());

  useEffect(() => {
    save(items);
  }, [items]);

  const add = useCallback((d: Draft) => setItems((prev) => [{ ...d, id: newId() }, ...prev]), []);
  const update = useCallback((id: string, d: Draft) => setItems((prev) => prev.map((t) => (t.id === id ? { ...d, id } : t))), []);
  const remove = useCallback((id: string) => setItems((prev) => prev.filter((t) => t.id !== id)), []);

  return { items, add, update, remove };
}
