#!/usr/bin/env node
// Written by every release build leg right after tauri-action: records which files THIS runner
// produced from THIS commit (name, size, SHA-256, commit, workflow run). The `verify-artifacts`
// job later compares the published release assets with these records.
//
// Usage: node scripts/release-record.mjs <out.json>
// Env:   ARTIFACT_PATHS (JSON array from tauri-action's `artifactPaths` output), PLATFORM, ARGS,
//        GITHUB_SHA, GITHUB_RUN_ID, GITHUB_RUN_ATTEMPT, GITHUB_SERVER_URL, GITHUB_REPOSITORY
import { createHash } from 'node:crypto';
import { readFileSync, statSync, writeFileSync } from 'node:fs';
import { basename } from 'node:path';

const out = process.argv[2];
if (!out) {
  console.error('usage: release-record.mjs <out.json>');
  process.exit(2);
}
const need = (name) => {
  const v = process.env[name];
  if (!v) {
    console.error(`::error::${name} is not set`);
    process.exit(1);
  }
  return v;
};

let paths;
try {
  paths = JSON.parse(need('ARTIFACT_PATHS'));
} catch (e) {
  console.error(`::error::ARTIFACT_PATHS is not valid JSON: ${e.message}`);
  process.exit(1);
}
if (!Array.isArray(paths) || paths.length === 0) {
  console.error('::error title=No build artifacts::tauri-action reported no artifact paths for this build');
  process.exit(1);
}

// macOS reports the unpacked `.app` bundle (a directory) next to the files it uploads.
const artifacts = paths.filter((p) => statSync(p).isFile()).map((p) => {
  const bytes = readFileSync(p);
  return { name: basename(p), size: statSync(p).size, sha256: createHash('sha256').update(bytes).digest('hex') };
});

const server = need('GITHUB_SERVER_URL');
const repo = need('GITHUB_REPOSITORY');
const runId = need('GITHUB_RUN_ID');
const record = {
  platform: need('PLATFORM'),
  args: process.env.ARGS || '',
  commit: need('GITHUB_SHA'),
  runId,
  runAttempt: process.env.GITHUB_RUN_ATTEMPT || '1',
  runUrl: `${server}/${repo}/actions/runs/${runId}`,
  artifacts,
};
writeFileSync(out, JSON.stringify(record, null, 2) + '\n');
for (const a of artifacts) console.log(`${a.sha256}  ${a.size}  ${a.name}`);
console.log(`::notice title=Build recorded (${record.platform} ${record.args})::${artifacts.length} artifacts from commit ${record.commit.slice(0, 12)}`);
