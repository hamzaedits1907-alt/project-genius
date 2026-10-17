#!/usr/bin/env bash
# Publish a new Genius version: bump every version field, commit, tag and push.
# GitHub Actions (.github/workflows/release.yml) then builds the installers, creates the
# release with the CHANGELOG.md notes and publishes SHA-256 checksums. Installed copies of
# Genius find it through the in-app updater.
#
# Usage: scripts/release.sh 0.2.1
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
  echo "usage: scripts/release.sh <version>   (e.g. 0.2.1)" >&2
  exit 1
fi
if ! grep -qE "^## \[?v?${version//./\\.}\]?" CHANGELOG.md; then
  echo "CHANGELOG.md has no section for $version. Add one like:" >&2
  echo "  ## [$version]" >&2
  echo "  - What changed" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain -- . ':!CHANGELOG.md')" ]]; then
  echo "Commit or stash your other changes first." >&2
  exit 1
fi
if git rev-parse "v$version" >/dev/null 2>&1; then
  echo "Tag v$version already exists." >&2
  exit 1
fi

# Workspace Cargo.toml, Tauri config, package.json and package-lock.json.
sed -i.bak -E "0,/^version = \"[^\"]+\"/s//version = \"$version\"/" Cargo.toml && rm Cargo.toml.bak
node -e '
  const fs = require("fs"); const v = process.argv[1];
  for (const f of ["src-tauri/tauri.conf.json", "package.json"]) {
    const j = JSON.parse(fs.readFileSync(f, "utf8")); j.version = v;
    fs.writeFileSync(f, JSON.stringify(j, null, 2) + "\n");
  }
  const lock = JSON.parse(fs.readFileSync("package-lock.json", "utf8"));
  lock.version = v; if (lock.packages && lock.packages[""]) lock.packages[""].version = v;
  fs.writeFileSync("package-lock.json", JSON.stringify(lock, null, 2) + "\n");
' "$version"
cargo update -w --offline >/dev/null 2>&1 || cargo update -w >/dev/null

git add Cargo.toml Cargo.lock src-tauri/tauri.conf.json package.json package-lock.json CHANGELOG.md
git commit -m "Release v$version"
git tag "v$version"
git push origin HEAD
if ! git push origin "v$version"; then
  echo "Could not push the tag. Start the release from GitHub instead: Actions → Release → Run workflow (branch main)." >&2
  exit 1
fi
echo "Pushed v$version. Follow the build at https://github.com/hamzaedits1907-alt/project-genius/actions"
