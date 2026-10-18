import { useLocalStorage } from './storage';

export function useDensity(): { compact: boolean; toggle: () => void } {
  const [compact, setCompact] = useLocalStorage<boolean>('dash.compact', false);
  return { compact, toggle: () => setCompact((c) => !c) };
}
