#!/usr/bin/env sh
set -eu

cargo test
bun run frontend:check

for script in scripts/*.sh; do
  sh -n "$script"
done
