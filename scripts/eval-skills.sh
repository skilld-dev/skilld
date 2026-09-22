#!/usr/bin/env bash
# Score the skilld-maintained Skills with `claude plugin eval`.
#
# Each case runs twice: once with the Skills loaded, once without. The reported
# delta is the value the Skills add. Every run is a real Agent session on your
# own credential, so the suite costs money and is never part of `pnpm test`.
#
# Usage: scripts/eval-skills.sh [claude plugin eval flags]
#   scripts/eval-skills.sh --case project-skill-real-paths
#   scripts/eval-skills.sh --runs 1 --ablation none
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
stage="${TMPDIR:-/tmp}/skilld-eval-plugin"

# The repository holds target/ and node_modules/, which overflow the eval's
# argument list. Stage only the plugin's own files.
rm -rf "$stage"
mkdir -p "$stage"
cp -r "$root/.claude-plugin" "$root/skills" "$root/evals" "$stage/"

cd "$stage"
claude plugin eval . \
  --scaffold \
  --trust-plugin \
  --allow-tools Bash Write Edit \
  --no-publish \
  "$@"

results="$stage/evals/results"
if [ -d "$results" ]; then
  mkdir -p "$root/evals/results"
  cp -r "$results/." "$root/evals/results/"
  echo "Results copied to $root/evals/results"
fi
