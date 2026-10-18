import { Segmented } from '../../components/ui';
import { useApp } from '../../store/app';

export type AppMode = 'chat' | 'code' | 'research';

/** Chat / Code / Research. Switching keeps every mode's state (open project, conversations). */
export function ModeSwitch({ mode }: { mode: AppMode }) {
  const navigate = useApp((s) => s.navigate);
  return (
    <Segmented<AppMode>
      label="Mode"
      value={mode}
      onChange={(m) => navigate(m === 'chat' ? { name: 'chat' } : m === 'research' ? { name: 'research' } : { name: 'home' })}
      options={[
        { value: 'chat', label: 'Chat' },
        { value: 'code', label: 'Code' },
        { value: 'research', label: 'Research' },
      ]}
    />
  );
}
