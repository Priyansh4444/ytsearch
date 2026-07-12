#!/usr/bin/env sh
set -eu

bun run test
bun run frontend:check
scripts/test-doctor.sh
scripts/test-compare-autoresearch-search.sh

for script in scripts/*.sh; do
  sh -n "$script"
done
