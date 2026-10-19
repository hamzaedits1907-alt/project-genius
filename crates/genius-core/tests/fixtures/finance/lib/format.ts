const usd = new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD' });

export const money = (n: number): string => usd.format(n);

export const percent = (ratio: number): string => `${Math.round(ratio * 100)}%`;

export const today = (): string => new Date().toISOString().slice(0, 10);
