#!/usr/bin/env node
// Release provenance. Two modes:
//
//   build <records-dir> <assets-dir> <out.json>
//       Compares the build records written by every build leg (scripts/release-record.mjs)
//       with the assets actually on the draft release, checks that the Windows installer is a
//       real NSIS executable, and writes RELEASE_MANIFEST.json (artifact name, size, SHA-256,
//       source commit, workflow run URL). Exits 1 with ::error:: annotations on any deviation.
//
//   check <manifest.json> <assets-dir>
//       Re-verifies every artifact listed in a manifest against the files on disk.
//
// Env (build): TAG, VERSION, GITHUB_SHA, GITHUB_RUN_ID, GITHUB_SERVER_URL, GITHUB_REPOSITORY,
//              EXPECTED_RECORDS (default 4: windows, macOS arm64, macOS x64, linux)
// What this proves: the files published on the release are byte-identical to what the CI runners
// built from the tagged commit. It is a consistency record, not a signature: see docs/RELEASING.md.
import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const errors = [];
const fail = (title, msg) => {
  errors.push(`${title}: ${msg}`);
  console.log(`::error title=${title}::${msg}`);
};
const sha256 = (bytes) => createHash('sha256').update(bytes).digest('hex');
const INSTALLER = /(-setup\.exe|\.msi|\.dmg|\.appimage|\.deb)$/i;
const META = /(\.sha256|\.sha256sum|\.sig)$|^RELEASE_MANIFEST\.json$/;
const REQUIRED_BY_PLATFORM = {
  'windows-latest': [/-setup\.exe$/i, /\.msi$/i],
  'macos-latest': [/\.dmg$/i],
  'ubuntu-22.04': [/\.appimage$/i, /\.deb$/i],
};

function listAssets(dir) {
  return readdirSync(dir)
    .filter((n) => statSync(join(dir, n)).isFile() && !META.test(n))
    .map((n) => {
      const bytes = readFileSync(join(dir, n));
      return { name: n, size: bytes.length, sha256: sha256(bytes), bytes };
    });
}

function buildMode(recordsDir, assetsDir, outFile) {
  const commit = process.env.GITHUB_SHA;
  const tag = process.env.TAG;
  if (!commit || !tag) {
    console.error('::error::GITHUB_SHA and TAG must be set');
    process.exit(1);
  }
  const expected = Number(process.env.EXPECTED_RECORDS || 4);
  const records = readdirSync(recordsDir)
    .filter((n) => /^build-record.*\.json$/.test(n))
    .map((n) => JSON.parse(readFileSync(join(recordsDir, n), 'utf8')));
  if (records.length !== expected) fail('Build records', `expected ${expected} build records (one per build leg), found ${records.length}`);

  const assets = listAssets(assetsDir);
  const manifestArtifacts = [];
  for (const rec of records) {
    const label = `${rec.platform} ${rec.args || ''}`.trim();
    if (rec.commit !== commit) fail('Build commit', `${label} was built from ${rec.commit}, the release commit is ${commit}`);
    for (const art of rec.artifacts) {
      const sameName = assets.find((a) => a.name === art.name);
      const sameHash = assets.filter((a) => a.sha256 === art.sha256);
      if (sameName && sameName.sha256 === art.sha256 && sameName.size === art.size) {
        manifestArtifacts.push({ name: art.name, size: art.size, sha256: art.sha256, platform: rec.platform, args: rec.args || '', sourceCommit: rec.commit, workflowRun: rec.runUrl });
      } else if (INSTALLER.test(art.name)) {
        fail('Release asset differs from the build', `${art.name} (${label}): built ${art.sha256}, ${sameName ? `release has ${sameName.sha256}` : sameHash.length ? `release has it as ${sameHash.map((a) => a.name).join(', ')}` : 'missing from the release'}`);
      } else {
        console.log(`::warning title=Not on the release::${art.name} (${label}) was built but is not a release asset under that name`);
      }
    }
    for (const need of REQUIRED_BY_PLATFORM[rec.platform] ?? []) {
      if (!manifestArtifacts.some((m) => m.platform === rec.platform && m.args === (rec.args || '') && need.test(m.name))) fail('Missing installer', `${label}: no verified installer matching ${need}`);
    }
  }

  // The Windows installer must be a real NSIS executable.
  const setups = assets.filter((a) => /-setup\.exe$/i.test(a.name));
  if (setups.length === 0) fail('Windows installer', 'the release has no *-setup.exe');
  for (const s of setups) {
    const isPe = s.bytes[0] === 0x4d && s.bytes[1] === 0x5a;
    const isNsis = s.bytes.includes('NullsoftInst');
    if (s.size < 1_000_000 || !isPe || !isNsis) fail('Windows installer', `${s.name} (${s.size} bytes) is not an NSIS executable (MZ header: ${isPe}, NSIS signature: ${isNsis})`);
    else if (!manifestArtifacts.some((m) => m.name === s.name && /^windows/.test(m.platform))) fail('Windows installer', `${s.name} was not produced by the Windows build leg of this run`);
    else console.log(`::notice title=Windows installer::${s.name} is a ${s.size}-byte NSIS executable built from ${commit.slice(0, 12)} (SHA-256 ${s.sha256})`);
  }

  // Everything installable on the release must come from this run.
  for (const a of assets) {
    if (INSTALLER.test(a.name) && !manifestArtifacts.some((m) => m.name === a.name)) fail('Unexpected release asset', `${a.name} is on the release but no build leg of this run produced it`);
  }
  if (errors.length > 0) process.exit(1);

  const server = process.env.GITHUB_SERVER_URL;
  const repo = process.env.GITHUB_REPOSITORY;
  const runId = process.env.GITHUB_RUN_ID;
  const manifest = {
    schema: 1,
    repository: repo,
    tag,
    version: process.env.VERSION || tag.replace(/^v/, ''),
    sourceCommit: commit,
    workflowRun: { id: runId, url: server && repo && runId ? `${server}/${repo}/actions/runs/${runId}` : null },
    generatedAt: new Date().toISOString(),
    note: 'Sizes and SHA-256 hashes are of the files the CI runners built from sourceCommit and equal the files on the release. This is a consistency record, not a cryptographic signature.',
    artifacts: manifestArtifacts.sort((x, y) => x.name.localeCompare(y.name)),
  };
  writeFileSync(outFile, JSON.stringify(manifest, null, 2) + '\n');
  console.log(`::notice title=RELEASE_MANIFEST.json::${manifest.artifacts.length} artifacts, source commit ${commit.slice(0, 12)}`);
}

function checkMode(manifestFile, assetsDir) {
  const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'));
  if (process.env.GITHUB_SHA && manifest.sourceCommit !== process.env.GITHUB_SHA) fail('Manifest commit', `${manifest.sourceCommit} is not ${process.env.GITHUB_SHA}`);
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0) fail('Manifest', 'lists no artifacts');
  for (const art of manifest.artifacts ?? []) {
    const p = join(assetsDir, art.name);
    if (!existsSync(p)) {
      fail('Manifest', `${art.name} is listed but not on the release`);
      continue;
    }
    const bytes = readFileSync(p);
    if (bytes.length !== art.size || sha256(bytes) !== art.sha256) fail('Manifest', `${art.name} differs from the manifest (size ${bytes.length} vs ${art.size}, sha256 ${sha256(bytes)} vs ${art.sha256})`);
  }
  if (errors.length > 0) process.exit(1);
  console.log(`::notice title=Manifest verified::${manifest.artifacts.length} artifacts equal their recorded SHA-256`);
}

const [mode, a, b, c] = process.argv.slice(2);
if (mode === 'build' && a && b && c) buildMode(a, b, c);
else if (mode === 'check' && a && b) checkMode(a, b);
else {
  console.error('usage: release-manifest.mjs build <records-dir> <assets-dir> <out.json> | check <manifest.json> <assets-dir>');
  process.exit(2);
}
