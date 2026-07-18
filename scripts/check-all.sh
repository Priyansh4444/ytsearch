#!/usr/bin/env sh
set -eu

bun run test
bun run frontend:check
scripts/check-rust-dependency-policy.sh
scripts/test-doctor.sh
scripts/test-check-queue-indexers.sh
scripts/test-index-conference-playlists.sh
scripts/test-compare-autoresearch-search.sh

for script in scripts/*.sh; do
  sh -n "$script"
done
