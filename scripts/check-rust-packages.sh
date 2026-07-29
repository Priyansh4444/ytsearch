#!/usr/bin/env sh
set -eu

cargo metadata --no-deps --format-version 1 |
  jq -r '
    .workspace_members as $members
    | .packages[]
    | select(.id as $id | $members | index($id))
    | .name
  ' |
  while IFS= read -r package; do
    cargo check --package "$package" --all-targets
  done
