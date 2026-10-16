// Pure helpers for presenting agent activity. Labels describe recorded operations only.
import type { AgentReport, AgentStep } from './types';

export function stepLabel(s: AgentStep): string {
  const a = s.args as Record<string, unknown>;
  const str = (k: string) => (typeof a[k] === 'string' ? (a[k] as string) : '');
  const base: Record<string, string> = {
    project_facts: 'Inspected project structure',
    list_files: `Listed files${str('prefix') ? ` in ${str('prefix')}` : ''}`,
    search: `Searched “${str('query')}”`,
    read_file: `Read ${str('path')}`,
    git_status: 'Checked Git status',
    git_diff: `Git diff of ${str('path')}`,
    recent_changes: 'Reviewed recent changes',
    memory: 'Read project memory',
    search_references: `Searched imported files for “${str('query')}”`,
    read_reference: 'Read an imported file',
    capabilities: 'Checked its live capabilities',
    last_run_output: `Read last ${str('check') || 'run'} output`,
    run_check: `Ran ${str('check')}`,
    remember: 'Saved a project memory',
    delegate: `Asked the ${str('role')} subagent`,
  };
  const label = base[s.tool] ?? `Called ${s.tool}`;
  return s.ok ? `${label} — ${s.summary}` : `${label} — ${s.summary || 'failed'}`;
}

export function reportSummary(r: AgentReport): string {
  const parts = [`${r.steps} step${r.steps === 1 ? '' : 's'}`, `${r.filesRead.length} file${r.filesRead.length === 1 ? '' : 's'} read`];
  if (r.checks.length) parts.push(`${r.checks.filter((c) => c.ok).length}/${r.checks.length} checks passed`);
  else parts.push('no checks run');
  return parts.join(' · ');
}

export function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error('could not read file'));
    reader.onload = () => {
      const s = String(reader.result);
      resolve(s.slice(s.indexOf(',') + 1));
    };
    reader.readAsDataURL(file);
  });
}
