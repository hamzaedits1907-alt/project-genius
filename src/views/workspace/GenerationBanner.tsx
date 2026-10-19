import { useState } from 'react';
import { Check, FileDiff, X } from 'lucide-react';
import { api, errorMessage } from '../../lib/api';
import { useApp } from '../../store/app';
import { useWorkspace } from '../../store/workspace';
import { Button } from '../../components/ui';

/**
 * Says plainly what state the generated project is in. Drafts the AI wrote but nobody applied
 * are not part of the project: the build, the preview and the file list on disk describe the
 * version WITHOUT them, and this banner is where that is stated.
 */
export default function GenerationBanner({ onReview, onPlan, onFix }: { onReview: (changesetId: string) => void; onPlan: () => void; onFix: () => void }) {
  const project = useWorkspace((s) => s.project);
  const drafts = useWorkspace((s) => s.drafts);
  const tasks = useWorkspace((s) => s.tasks);
  const toast = useApp((s) => s.toast);
  const [busy, setBusy] = useState<'apply' | 'reject' | null>(null);
  if (!project) return null;
  if (tasks.some((t) => t.kind === 'generate')) return null; // still working: the activity log shows the stage

  const pending = project.status.pendingChangeset ?? null;
  const generation = project.status.generation?.state;
  const draftCount = Object.values(drafts).filter((d) => d.status !== 'streaming').length;

  const act = async (what: 'apply' | 'reject') => {
    if (!pending) return;
    setBusy(what);
    try {
      if (what === 'apply') {
        await api.applyChangeset(pending);
        toast('ok', 'Applied. Genius is now building the real files.');
      } else {
        await api.rejectChangeset(pending);
      }
      useWorkspace.setState((s) => ({ drafts: {}, historyVersion: s.historyVersion + 1, gitVersion: s.gitVersion + 1 }));
      await useWorkspace.getState().refreshFiles();
      await useWorkspace.getState().refreshProject();
    } catch (e) {
      toast('bad', errorMessage(e));
    } finally {
      setBusy(null);
    }
  };

  if (pending) {
    return (
      <div className="callout warn" role="status" data-testid="generation-banner">
        <FileDiff size={16} />
        <div className="grow">
          <strong>{draftCount > 0 ? `${draftCount} AI draft file(s)` : 'AI-written files'} are NOT in your project yet.</strong> The preview and the build status describe the version without them. Apply the changes to put the new files into the project and build them.
        </div>
        <Button size="sm" variant="primary" icon={<Check size={13} />} loading={busy === 'apply'} disabled={!!busy} onClick={() => void act('apply')}>
          Apply and build
        </Button>
        <Button size="sm" onClick={() => onReview(pending)}>
          Review diff
        </Button>
        <Button size="sm" variant="ghost" icon={<X size={13} />} loading={busy === 'reject'} disabled={!!busy} onClick={() => void act('reject')}>
          Discard
        </Button>
      </div>
    );
  }
  if (project.status.stage === 'generation_incomplete' || generation === 'incomplete') {
    return (
      <div className="callout bad" role="alert" data-testid="generation-banner">
        <div className="grow">
          <strong>The application was not generated.</strong> src/App.tsx is still the starter page, so the preview only shows the template. Some files may have been written; nothing is verified.
        </div>
        <Button size="sm" variant="primary" onClick={onPlan}>
          Open plan and regenerate
        </Button>
      </div>
    );
  }
  if (generation === 'build_failed' || generation === 'runtime_errors') {
    return (
      <div className="callout bad" role="alert" data-testid="generation-banner">
        <div className="grow">
          <strong>{generation === 'build_failed' ? 'The generated project does not build.' : 'The generated app builds but fails when opened.'}</strong> The application is written to the project, but it is not working yet.
        </div>
        <Button size="sm" variant="primary" onClick={onFix}>
          Fix it for me
        </Button>
      </div>
    );
  }
  if (generation === 'preview_failed') {
    return (
      <div className="callout warn" role="status" data-testid="generation-banner">
        <div className="grow">
          <strong>Built, but the preview did not start.</strong> Press Run to try again; the files are in the project.
        </div>
      </div>
    );
  }
  if (generation === 'built') {
    return (
      <div className="callout info" role="status" data-testid="generation-banner">
        <div className="grow">
          <strong>Built successfully.</strong> It was not opened in a real browser (none found, or the check could not run), so how it behaves is not verified yet.
        </div>
      </div>
    );
  }
  return null;
}
