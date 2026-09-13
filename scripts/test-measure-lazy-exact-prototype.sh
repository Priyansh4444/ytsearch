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
  .schema_version == 2 and
  .outcome == "complete_with_exclusions" and
  .throwaway_schema9_prototype and
  .run_identity.report_schema_version == 2 and
  (.run_identity.identity_digest | test("^blake3:[0-9a-f]{64}$")) and
  (.exact_set_count | length == 7) and
  (.exact_moment_aggregation | length == 7) and
  (.phrase_candidates | length == 7) and
  (.semantic_contracts | length == 2) and
  (.semantic_contracts | all(.outcome.status == "eligible" and .actual == .expected)) and
  ([.exact_set_count[], .exact_moment_aggregation[], .phrase_candidates[]]
    | length == 21) and
  ([.exact_set_count[], .exact_moment_aggregation[], .phrase_candidates[]]
    | all(.outcome.status == "eligible" or .outcome.status == "ineligible" or .outcome.status == "failed")) and
  ([.exact_set_count[], .exact_moment_aggregation[], .phrase_candidates[]]
    | map(select(.outcome.status == "ineligible")) | length > 0) and
  (.exact_set_count[0].bitmap_and_popcount == null) and
  (.exact_set_count[0].sparse_skip_leapfrog == null)
' "$scratch/a.jsonl" >/dev/null

if "$script_dir/measure-lazy-exact-prototype.sh" \
  "/var/tmp/ytsearch-should-not-exist.jsonl" --documents 1 --rounds 1 >/dev/null 2>&1
then
  printf 'prototype writer accepted a non-temporary output path\n' >&2
  exit 1
fi

if "$script_dir/measure-lazy-exact-prototype.sh" \
  "$scratch/invalid.jsonl" --documents 0 --rounds 1 >/dev/null 2>&1
then
  printf 'prototype accepted an invalid effective configuration\n' >&2
  exit 1
fi
jq -e '
  .schema_version == 2 and
  .outcome == "failed" and
  .throwaway_schema9_prototype and
  (.reason | contains("positive"))
' "$scratch/invalid.jsonl.partial" >/dev/null

printf 'lazy-exact prototype measurement tests passed\n'
