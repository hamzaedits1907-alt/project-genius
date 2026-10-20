import { describe, expect, it } from 'vitest';
import { queryTable } from '../web/engine/tables';

const CSV = ['date,category,amount,note', '2026-01-03,Food,12.50,lunch', '2026-01-04,Food,30,groceries', '2026-01-05,Rent,900,jan', '2026-02-01,Food,8,coffee', '2026-02-02,Fun,45.5,cinema', '2026-02-03,Fun,,missing amount'].join('\n');
const q = (a: object) => queryTable('spend.csv', 'csv', CSV, a);

describe('data_query (exact table queries)', () => {
  it('groups and sums over the whole file, biggest first', () => {
    const r = q({ group_by: 'category', metrics: [{ column: 'amount', fn: 'sum' }] });
    expect(r).toContain('| category | rows | sum(amount) |');
    expect(r.split('\n').filter((l) => /^\| (Rent|Fun|Food) /.test(l))).toEqual(['| Rent | 1 | 900 |', '| Food | 3 | 50.5 |', '| Fun | 2 | 45.5 |']);
  });
  it('filters with numbers compared as numbers and text case-insensitively', () => {
    expect(q({ where: [{ column: 'amount', op: '>', value: 20 }], columns: ['category', 'amount'], sort_by: 'amount', desc: true })).toContain('Showing 3 of 3 rows');
    expect(q({ where: [{ column: 'category', value: 'food' }], metrics: [{ column: 'amount', fn: 'mean' }] })).toContain('mean(amount) = 16.8333333333');
    expect(q({ where: [{ column: 'note', op: 'contains', value: 'JAN' }], columns: ['note'] })).toContain('| jan |');
  });
  it('aggregates without grouping and says how many numeric values it used', () => {
    const r = q({ metrics: [{ column: 'amount', fn: 'sum' }, { fn: 'count' }] });
    expect(r).toContain('sum(amount) = 996 (over 5 numeric values)');
    expect(r).toContain('- count = 6');
  });
  it('lists rows with a limit and reports honest totals', () => {
    const r = q({ limit: 2, sort_by: 'date', desc: true });
    expect(r).toContain('Showing 2 of 6 rows sorted by date (descending)');
    expect(r).toContain('2026-02-03');
    expect(r).not.toContain('2026-01-03');
  });
  it('explains a wrong column, operator or metric instead of guessing', () => {
    expect(() => q({ group_by: 'nope' })).toThrow(/no column "nope".*Columns: date, category/);
    expect(() => q({ where: [{ column: 'amount', op: '~', value: 1 }] })).toThrow(/unknown filter operator/);
    expect(() => q({ metrics: [{ column: 'amount', fn: 'avg' }] })).toThrow(/unknown metric/);
    expect(() => q({ metrics: [{ fn: 'sum' }] })).toThrow(/needs a "column"/);
  });
});
