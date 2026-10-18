# Releasing

## Cut a release

1. Add a `## [x.y.z]` section to `CHANGELOG.md` (it becomes the release notes shown in the app).
2. `scripts/release.sh x.y.z` bumps every version field, commits, tags `vx.y.z`, pushes `main` and the tag.
3. `.github/workflows/release.yml` runs. Nothing is visible to the updater until the last job flips the draft to published.

Concurrency: one release run at a time (`concurrency: release`). If a third run is queued, GitHub drops the oldest waiting one; re-run it by hand.

## Gates (all must pass before the draft is published)

| Job | Refuses when |
|---|---|
| `prepare` | tag differs from `tauri.conf.json` version; commit is not on `main`; version already published |
| `ci-gate` | CI jobs `rust`, `frontend`, `windows-desktop` are not all `success` on the release commit (waits up to 60 min for running ones; `scripts/ci-gate.sh`) |
| `build` | any platform fails; each leg writes a record (name, size, SHA-256, commit, run URL) |
| `verify-artifacts` | a release installer differs from what a runner built; the tag/draft targets another commit; the Windows `*-setup.exe` is not an NSIS executable (MZ header + NSIS signature); writes `RELEASE_MANIFEST.json` |
| `checksums` | an installer is missing; files differ from the manifest; then uploads `*.sha256` and publishes |
| `verify-updater` (optional) | see below; only reports, never unpublishes |

Artifacts map to commits: the release body and `RELEASE_MANIFEST.json` carry the source commit and workflow run URL. This is a CI consistency record, **not** a signature.

## Windows updater verification

Actions, Updater (Windows), Run workflow (inputs `old_tag`, `new_tag`; empty = previous and latest published release). It only downloads published assets. To run it automatically after each release set the repository variable `VERIFY_UPDATER_AFTER_RELEASE=true` (release.yml calls it as `verify-updater`).

It runs on `windows-latest` and: verifies the old `-setup.exe` against its `.sha256`; installs it silently (per-user, default folder); creates marker files in `%APPDATA%\dev.genius.studio`; runs `cargo test -p genius-core --test updater_live` with `GENIUS_LIVE_UPDATER=1`, which uses the real updater code to check from the old version, select the NSIS asset, download and verify it, reject a tampered copy / substituted content / size mismatch / foreign hosts, and run the new installer silently (`updater::install_silent`, test-only); then asserts the installed exe version is the new one, same folder, uninstaller present, markers unchanged; finally starts the app, waits 20 s, requires a live process and a main window titled "Genius". Logs and a screenshot are uploaded.

Proves: the published pair (installer + checksum) is consistent, the updater logic works against real GitHub data, and an NSIS install over an older install yields a working newer app with user data kept.
Does not prove: the interactive installer and the in-app "Update now" exit/relaunch sequence (the app launches the interactive installer), code signing, macOS. It has never been run yet: the first run may expose problems in the PowerShell script (for example the executable or registry discovery).

## Checksums and signing, honestly

`.sha256` files come from the same release as the installers: they detect corruption and truncation, not a malicious publisher. The Windows installer is not Authenticode-signed. The updater refuses a release without a checksum.

To get authenticity, use Tauri's minisign updater signatures: generate a key (`npx tauri signer generate -w ~/.tauri/genius.key`), put the private key in the repository secret `TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`), set `bundle.createUpdaterArtifacts` and the public key in `tauri.conf.json`, and verify the `.sig` in the updater before installing. None of that exists yet and no key is in the repo.

## Formatting

`rustfmt.toml` approximates the dense hand-written style but `cargo fmt` still changes hundreds of places, so CI only reports it. Run `cargo fmt --all` once when no other branch is open, commit it alone, then make the CI step a plain `--check`.
