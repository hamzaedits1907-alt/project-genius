import { useEffect, useState, type FormEvent } from 'react';
import { today } from '../lib/format';
import { CATEGORIES, type Draft, type Transaction, type TxType } from '../types';

type Props = { editing: Transaction | null; onSubmit: (d: Draft) => void; onCancel: () => void };

export default function TransactionForm({ editing, onSubmit, onCancel }: Props) {
  const [description, setDescription] = useState('');
  const [amount, setAmount] = useState('');
  const [type, setType] = useState<TxType>('expense');
  const [category, setCategory] = useState<string>('Food');
  const [date, setDate] = useState(today());
  const [error, setError] = useState('');

  useEffect(() => {
    if (editing) {
      setDescription(editing.description);
      setAmount(String(editing.amount));
      setType(editing.type);
      setCategory(editing.category);
      setDate(editing.date);
    } else {
      setDescription('');
      setAmount('');
      setDate(today());
    }
    setError('');
  }, [editing]);

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const value = Number(amount);
    if (!description.trim()) return setError('Add a description.');
    if (!Number.isFinite(value) || value <= 0) return setError('Enter an amount greater than zero.');
    setError('');
    onSubmit({ date, description: description.trim(), amount: value, type, category });
  };

  return (
    <form className="panel form" onSubmit={submit} aria-label={editing ? 'Edit transaction' : 'Add transaction'}>
      <h2>{editing ? 'Edit transaction' : 'New transaction'}</h2>
      <label>
        Description
        <input id="tx-description" value={description} onChange={(e) => setDescription(e.target.value)} placeholder="e.g. Groceries" />
      </label>
      <label>
        Amount
        <input id="tx-amount" inputMode="decimal" value={amount} onChange={(e) => setAmount(e.target.value)} placeholder="0.00" />
      </label>
      <div className="two">
        <label>
          Type
          <select id="tx-type" value={type} onChange={(e) => setType(e.target.value as TxType)}>
            <option value="expense">Expense</option>
            <option value="income">Income</option>
          </select>
        </label>
        <label>
          Category
          <select id="tx-category" value={category} onChange={(e) => setCategory(e.target.value)}>
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>{c}</option>
            ))}
          </select>
        </label>
      </div>
      <label>
        Date
        <input id="tx-date" type="date" value={date} onChange={(e) => setDate(e.target.value)} />
      </label>
      {error && <p className="error" role="alert">{error}</p>}
      <div className="actions">
        <button id="tx-submit" type="submit" className="primary">{editing ? 'Save changes' : 'Add transaction'}</button>
        {editing && <button type="button" onClick={onCancel}>Cancel</button>}
      </div>
    </form>
  );
}
