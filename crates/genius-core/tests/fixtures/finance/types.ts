export type TxType = 'income' | 'expense';

export interface Transaction {
  id: string;
  date: string;
  description: string;
  amount: number;
  type: TxType;
  category: string;
}

export type Draft = Omit<Transaction, 'id'>;

export const CATEGORIES = ['Salary', 'Housing', 'Food', 'Transport', 'Leisure', 'Health', 'Other'] as const;
