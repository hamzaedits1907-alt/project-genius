import { useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Download, ExternalLink, PartyPopper } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { api, errorMessage } from '../lib/api';
import { bytes } from '../lib/logic';
import { useSubscription } from '../lib/effects';
import type { UpdateProgress } from '../lib/types';
import { useApp } from '../store/app';
import { Button, Callout, Modal } from '../components/ui';
import { Markdown } from '../components/Markdown';

/** Shows a new GitHub release and installs it: download → verify → run the installer. */
export default function UpdateDialog({ onClose }: { onClose: () => void }) {
  const update = useApp((s) => s.update);
  const justUpdated = useApp((s) => s.justUpdated);
  const skipVersion = useApp((s) => s.skipVersion);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useSubscription('update progress', () => listen<UpdateProgress>('genius://update-progress', (e) => setProgress(e.payload)), []);

  if (!update) {
    // First start after an update, before the release check has answered.
    return justUpdated ? (
      <Modal title={`Welcome to Genius ${justUpdated}`} icon={<PartyPopper size={18} color="var(--accent-2)" />} onClose={onClose} footer={<Button variant="primary" onClick={onClose}>Let's go</Button>}>
        <div className="muted">The update was installed successfully. Your projects and settings are all here.</div>
      </Modal>
    ) : null;
  }
  const pct = progress?.total ? Math.round((progress.downloaded / progress.total) * 100) : null;
  const install = async () => {
    setBusy(true);
    setError(null);
    try {
      await api.installUpdate();
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  const stageText: Record<string, string> = {
    downloading: 'Downloading',
    verifying: 'Verifying the download',
    installing: 'Starting the installer — Genius will close and reopen',
    done: 'Done',
  };

  return (
    <Modal
      title={update.available ? `Genius ${update.latestVersion} is available` : justUpdated ? `Welcome to Genius ${justUpdated}` : 'Genius is up to date'}
      icon={<PartyPopper size={18} color="var(--accent-2)" />}
      onClose={busy ? () => undefined : onClose}
      footer={
        <>
          {update.releaseUrl && (
            <Button variant="ghost" icon={<ExternalLink size={14} />} onClick={() => void openUrl(update.releaseUrl!)}>
              Release page
            </Button>
          )}
          <span className="spacer" />
          {update.available && update.latestVersion && (
            <Button variant="ghost" onClick={() => skipVersion(update.latestVersion)} disabled={busy}>
              Skip this version
            </Button>
          )}
          <Button onClick={onClose} disabled={busy}>
            Later
          </Button>
          {update.available && (
            <Button variant="primary" icon={<Download size={14} />} loading={busy} disabled={!update.asset} onClick={() => void install()}>
              Update now
            </Button>
          )}
        </>
      }
    >
      {justUpdated && !update.available && <Callout tone="ok">The update was installed successfully — here's what's new.</Callout>}
      <div className="muted small">
        You have {update.currentVersion}
        {update.latestVersion && <> · latest is {update.latestVersion}</>}
        {update.publishedAt && <> · released {new Date(update.publishedAt).toLocaleDateString()}</>}
      </div>
      {update.available && !update.asset && <Callout tone="warn">This release has no installer for your system yet. Open the release page to download it manually.</Callout>}
      {update.asset && (
        <div className="small faint">
          Installer: <span className="mono">{update.asset.name}</span> ({bytes(update.asset.size)}){update.asset.sha256Url ? ' · checksum verified after download' : ''}
        </div>
      )}
      {update.notes.trim() && (
        <div className="release-notes">
          <Markdown text={update.notes} />
        </div>
      )}
      {progress && (
        <div className="col" style={{ gap: 6 }}>
          <div className="small">
            {stageText[progress.stage] ?? progress.stage}
            {progress.stage === 'downloading' && (pct != null ? ` · ${pct}%` : ` · ${bytes(progress.downloaded)}`)}
          </div>
          <div className="progress">
            <div style={{ width: `${progress.stage === 'downloading' ? (pct ?? 30) : 100}%` }} />
          </div>
        </div>
      )}
      {error && <Callout tone="bad" title="The update could not be installed">{error}</Callout>}
      {update.available && update.asset && /\.dmg$/i.test(update.asset.name) && (
        <div className="faint tiny">macOS: the disk image opens — drag Genius into Applications and replace the old copy. If macOS says the app is damaged, run <span className="mono">xattr -cr /Applications/Genius.app</span> (the app is not notarized).</div>
      )}
      {update.available && update.asset && /\.(exe|msi)$/i.test(update.asset.name) && (
        <div className="faint tiny">Windows: if SmartScreen warns about an unknown publisher, choose “More info → Run anyway” (the installer is not code-signed).</div>
      )}
      <div className="faint tiny">Your projects, settings and API key are kept. Updates come only from the official GitHub releases of Project Genius.</div>
    </Modal>
  );
}
