#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
scratch=$(mktemp -d /tmp/ytsearch-lazy-exact-measure-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT

for run in a b; do
  "$script_dir/measure-lazy-exact-prototype.sh" \
    "$scratch/$run.jsonl" --documents 20003 --rounds 2 >/dev/null
  jq -S '
    walk(
      if type == "object" and has("samples_ns") then
        {scope, warmup_runs}
      else
        .
      end
    )
  ' "$scratch/$run.jsonl" > "$scratch/$run.semantic.json"
done

cmp "$scratch/a.semantic.json" "$scratch/b.semantic.json"
jq -e '
  .format_version == 1 and
  .outcome == "ok" and
  .throwaway_schema9_prototype and
  (.exact_set_count | length == 7) and
  (.exact_moment_aggregation | length == 7) and
  (.phrase_candidates | length == 7)
' "$scratch/a.jsonl" >/dev/null

if "$script_dir/measure-lazy-exact-prototype.sh" \
  "/var/tmp/ytsearch-should-not-exist.jsonl" --documents 1 --rounds 1 >/dev/null 2>&1
then
  printf 'prototype writer accepted a non-temporary output path\n' >&2
  exit 1
fi

printf 'lazy-exact prototype measurement tests passed\n'
