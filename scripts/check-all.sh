#!/usr/bin/env sh
set -eu

bun run check:precommit
scripts/check-rust-packages.sh
cargo test --workspace
bun run test:e2e
scripts/check-rust-dependency-policy.sh
scripts/test-doctor.sh
scripts/test-check-queue-indexers.sh
scripts/test-index-conference-playlists.sh
scripts/test-compare-autoresearch-search.sh
scripts/test-idle-gap-fill.sh

for script in scripts/*.sh; do
  sh -n "$script"
done
