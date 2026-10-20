import { describe, expect, it } from 'vitest';
import { compareVersions, newerRelease } from '../web/appUpdate';

const release = (tag: string, assets: string[], body = 'Notes\n---\nBuilt by CI') => async () =>
  new Response(JSON.stringify({ tag_name: tag, body, assets: assets.map((name) => ({ name, browser_download_url: `https://example.org/${name}` })) }), { status: 200 });

describe('app update check', () => {
  it('compares versions numerically, not as text', () => {
    expect(compareVersions('0.10.0', '0.9.2')).toBe(1);
    expect(compareVersions('v0.7.1', '0.7.0')).toBe(1);
    expect(compareVersions('0.7.0', '0.7.0')).toBe(0);
    expect(compareVersions('0.6.9', '0.7.0')).toBe(-1);
    expect(compareVersions('1.0', '0.9.9')).toBe(1);
    expect(compareVersions('0.7.1-beta', '0.7.1')).toBe(0);
  });

  it('offers the APK of a newer release and says when it is debug-signed', async () => {
    const u = await newerRelease('0.7.0', release('v0.7.1', ['Genius_0.7.1_x64-setup.exe', 'Genius_0.7.1_android-debug.apk', 'Genius_0.7.1_android-debug.apk.sha256']) as unknown as typeof fetch);
    expect(u).toMatchObject({ version: '0.7.1', name: 'Genius_0.7.1_android-debug.apk', url: 'https://example.org/Genius_0.7.1_android-debug.apk', debugSigned: true, notes: 'Notes' });
    const signed = await newerRelease('0.7.0', release('v0.7.1', ['Genius_0.7.1_android.apk']) as unknown as typeof fetch);
    expect(signed?.debugSigned).toBe(false);
  });

  it('offers nothing when up to date, when the release has no APK, or when GitHub fails', async () => {
    expect(await newerRelease('0.7.1', release('v0.7.1', ['Genius_0.7.1_android.apk']) as unknown as typeof fetch)).toBeNull();
    expect(await newerRelease('0.7.0', release('v0.7.1', ['Genius_0.7.1_x64-setup.exe']) as unknown as typeof fetch)).toBeNull();
    await expect(newerRelease('0.7.0', (async () => new Response('limit', { status: 403 })) as unknown as typeof fetch)).rejects.toThrow(/403/);
  });
});
