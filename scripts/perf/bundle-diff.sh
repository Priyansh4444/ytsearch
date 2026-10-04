#!/usr/bin/env bash
# Explains a bundle-size change: which frontend modules and which Worker
# packages grew between a git ref and the working tree.
#
#   scripts/perf/bundle-diff.sh <base-ref>
#
# Use it when `bun run perf:bundle` or `perf:worker` fails, before touching a
# budget. The base ref is built in a temporary worktree with its own
# `bun install --frozen-lockfile`, so dependency upgrades are compared fairly.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

if [ $# -ne 1 ]; then
  sed -n '2,10p' "$0" >&2
  exit 2
fi
base_dir="$(mktemp -d /tmp/ytsearch-bundle-base.XXXXXX)"
results="$(mktemp -d /tmp/ytsearch-bundle-diff.XXXXXX)"
git -C "$REPO_ROOT" worktree add --quiet --detach "$base_dir" "$1"
trap 'git -C "$REPO_ROOT" worktree remove --force "$base_dir"; rm -rf "$results"' EXIT

(cd "$base_dir" && bun install --frozen-lockfile >/dev/null &&
  bun "$PERF_DIR/bundle-attribution.ts" "$results/base.json")
(cd "$REPO_ROOT" && bun "$PERF_DIR/bundle-attribution.ts" "$results/head.json")
bun "$PERF_DIR/bundle-attribution.ts" --compare "$results/base.json" "$results/head.json"
