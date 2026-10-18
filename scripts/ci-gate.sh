#!/usr/bin/env bash
# Release gate: refuse to continue unless the CI workflow is green on the commit being released.
#
# The release workflow starts on a tag push, usually seconds after the push of the commit to main
# that starts CI, so this waits (up to CI_GATE_TIMEOUT seconds) for the required CI jobs to finish
# instead of failing on "not started yet". Any job that finished with something other than
# "success" (failure, cancelled, skipped, ...) fails the gate at once.
#
# Usage: scripts/ci-gate.sh [sha] [owner/repo]       needs: gh (GH_TOKEN with checks:read), jq
set -euo pipefail

sha="${1:-${GITHUB_SHA:?sha required}}"
repo="${2:-${GITHUB_REPOSITORY:?repository required}}"
# Job ids of .github/workflows/ci.yml (check-run names default to the job id).
read -r -a required <<< "${CI_GATE_JOBS:-rust frontend windows-desktop}"
timeout_s="${CI_GATE_TIMEOUT:-3600}"
interval="${CI_GATE_INTERVAL:-30}"
start=$SECONDS

while :; do
  runs=$(gh api "repos/$repo/commits/$sha/check-runs?per_page=100&filter=latest" --jq '[.check_runs[] | {name, status, conclusion}]')
  pending=(); failed=(); passed=0
  for name in "${required[@]}"; do
    row=$(jq -c --arg n "$name" '[.[] | select(.name == $n)] | .[0] // empty' <<< "$runs")
    if [[ -z "$row" ]]; then
      pending+=("$name (not started)")
      continue
    fi
    status=$(jq -r '.status' <<< "$row")
    conclusion=$(jq -r '.conclusion // ""' <<< "$row")
    if [[ "$status" != "completed" ]]; then
      pending+=("$name ($status)")
    elif [[ "$conclusion" == "success" ]]; then
      passed=$((passed + 1))
    else
      failed+=("$name=$conclusion")
    fi
  done

  if (( ${#failed[@]} > 0 )); then
    echo "::error title=CI is not green on ${sha:0:12}::These CI jobs did not succeed: ${failed[*]}. Fix them (or re-run CI) before releasing."
    exit 1
  fi
  if (( passed == ${#required[@]} )); then
    echo "::notice title=CI is green on ${sha:0:12}::${required[*]} all succeeded."
    exit 0
  fi
  if (( SECONDS - start > timeout_s )); then
    echo "::error title=CI did not finish on ${sha:0:12}::Still waiting after ${timeout_s}s for: ${pending[*]}. CI must run and pass on the release commit."
    exit 1
  fi
  echo "waiting for CI on ${sha:0:12}: ${pending[*]}"
  sleep "$interval"
done
