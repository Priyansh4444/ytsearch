#!/usr/bin/env sh
set -eu

bun run test
bun run frontend:check

for script in scripts/*.sh; do
  sh -n "$script"
done
