#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
scratch=$(mktemp -d /tmp/ytsearch-lazy-exact-measure-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT

for run in a b; do
  "$script_dir/measure-lazy-exact-prototype.sh" \
    "$scratch/$run.jsonl" --documents 20003 --rounds 2 >/dev/null
  jq -S '
    del(.resources) |
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
  .schema_version == 3 and
  .outcome == "complete_with_exclusions" and
  .throwaway_schema9_prototype and
  .run_identity.report_schema_version == 3 and
  (.run_identity.identity_digest | test("^blake3:[0-9a-f]{64}$")) and
  (.run_identity.source_commit | test("^[0-9a-f]{40}$")) and
  (.run_identity.source_dirty | type == "boolean") and
  (.run_identity.rustc_version | startswith("rustc ")) and
  (.run_identity.rustc_target | length > 0) and
  .run_identity.build_profile == "release" and
  (.run_identity.environment | length > 0) and
  .counters == {attempted: 24, eligible: 20, excluded: 4, failed: 0} and
  (.resources.final_peak_rss_bytes > 0 or .resources.rss_unavailable_reason != null) and
  (.resources.proc_io_read_bytes_delta != null or .resources.proc_io_unavailable_reason != null) and
  (.exact_set_count | length == 7) and
  (.exact_moment_aggregation | length == 7) and
  (.phrase_candidates | length == 7) and
  (.semantic_contracts | length == 3) and
  (.semantic_contracts | all(.outcome.status == "eligible" and .actual == .expected)) and
  (.semantic_contracts | all(.measurement.p95_ns >= .measurement.median_ns)) and
  ([.exact_set_count[].construction_measurement_order] | unique | length == 2) and
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


for failure in oracle measurement; do
  "$script_dir/measure-lazy-exact-prototype.sh" \
    "$scratch/$failure.jsonl" --documents 20003 --rounds 2 \
    --inject-failure "$failure" >/dev/null
  jq -e --arg failure "${failure}_mismatch" '
    .outcome == "complete_with_failures" and
    .counters == {attempted: 24, eligible: 19, excluded: 4, failed: 1} and
    (.exact_set_count[]
      | select(.case_id == "set-count-df-1000")
      | .outcome.status == "failed" and
        (.outcome.reason == $failure or
         ($failure == "measurement_mismatch" and .outcome.reason == "measurement_failure")) and
        .bitmap_construction == null and
        .sparse_construction == null and
        .bitmap_and_popcount == null and
        .sparse_skip_leapfrog == null)
  ' "$scratch/$failure.jsonl" >/dev/null
done

if "$script_dir/measure-lazy-exact-prototype.sh" \
  "$scratch/invalid.jsonl" --documents 0 --rounds 1 >/dev/null 2>&1
then
  printf 'prototype accepted an invalid effective configuration\n' >&2
  exit 1
fi
jq -e '
  .schema_version == 3 and
  .outcome == "failed" and
  .throwaway_schema9_prototype and
  (.reason | contains("positive"))
' "$scratch/invalid.jsonl.partial" >/dev/null

printf 'lazy-exact prototype measurement tests passed\n'
