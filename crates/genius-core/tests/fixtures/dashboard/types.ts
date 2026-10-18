export type Theme = 'light' | 'dark';
export type Period = '7d' | '30d';
export type WidgetKind = 'metrics' | 'chart' | 'tasks' | 'notes';
export type Widget = { id: string; kind: WidgetKind; title: string };
export type Task = { id: string; text: string; done: boolean };
export type Metric = { label: string; value: number; delta: string; currency?: boolean };
