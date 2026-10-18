import type { Metric, Period, Widget, WidgetKind } from './types';

export const DEFAULT_WIDGETS: Widget[] = [
  { id: 'metrics', kind: 'metrics', title: 'Key metrics' },
  { id: 'chart', kind: 'chart', title: 'Revenue chart' },
  { id: 'tasks', kind: 'tasks', title: 'Tasks' },
  { id: 'notes', kind: 'notes', title: 'Notes' },
];

export const KIND_LABELS: Record<WidgetKind, string> = { metrics: 'Metric', chart: 'Chart', tasks: 'Tasks', notes: 'Notes' };

export const PERIODS: { value: Period; label: string }[] = [
  { value: '7d', label: 'Last 7 days' },
  { value: '30d', label: 'Last 30 days' },
];

export function metricsFor(period: Period): Metric[] {
  return period === '7d'
    ? [
        { label: 'Revenue', value: 48210, delta: '+12%', currency: true },
        { label: 'Orders', value: 1204, delta: '+4%' },
        { label: 'Customers', value: 932, delta: '+7%' },
      ]
    : [
        { label: 'Revenue', value: 182400, delta: '+9%', currency: true },
        { label: 'Orders', value: 4710, delta: '+6%' },
        { label: 'Customers', value: 3105, delta: '+11%' },
      ];
}

export function seriesFor(period: Period): number[] {
  const n = period === '7d' ? 7 : 30;
  return Array.from({ length: n }, (_, i) => 20 + ((i * 37) % 23) + i);
}
