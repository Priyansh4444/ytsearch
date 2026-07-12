#!/usr/bin/env sh
set -eu

bun run test
bun run frontend:check
scripts/test-doctor.sh

for script in scripts/*.sh; do
  sh -n "$script"
done
