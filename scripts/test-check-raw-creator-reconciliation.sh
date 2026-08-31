#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM
mkdir -p "$temporary_directory/discoveries"
printf 'catalog-test-run\n' > "$temporary_directory/discoveries/active-run-id"

printf '@Alpha | Alpha\n@Beta | Beta\n' > "$temporary_directory/channels.txt"
cat > "$temporary_directory/catalog.json" <<'JSON'
{"generation":42,"last_sequence_before":42,"last_sequence_after":42,
 "channels":[
  {"id":"alpha","handle":"@Alpha","public_video_count":3,"indexed_video_count":2,"indexed_chunk_count":20},
  {"id":"beta","handle":"@beta","public_video_count":4,"indexed_video_count":4,"indexed_chunk_count":40}
 ],
 "video_ids_by_creator":{"alpha":["alpha-1","alpha-2"],
   "beta":["beta-1","beta-2","beta-3","beta-4"]}}
JSON
cat > "$temporary_directory/failures.json" <<'JSON'
{
  "total_records":1,"structured_records":1,"legacy_records":0,
  "attributed_records":1,"unattributed_records":0,
  "totals":{"total":1,"retryable":0,"ready_retry":0,"terminal":1,
    "categories":{"no_captions":1}},
  "by_creator":{"alpha":{"total":1,"retryable":0,"ready_retry":0,
    "terminal":1,"categories":{"no_captions":1}}},
  "records":[{"video_id":"alpha-3","creator_id":"alpha",
    "category":"no_captions","retry_state":"terminal"}]
}
JSON
cat > "$temporary_directory/discoveries/alpha.json" <<'JSON'
{"schema_version":1,"status":"complete","run_id":"catalog-test-run","creator_id":"alpha","handle":"@alpha",
  "video_ids":["alpha-1","alpha-2","alpha-3"]}
JSON
cat > "$temporary_directory/discoveries/beta.json" <<'JSON'
{"schema_version":1,"status":"complete","run_id":"catalog-test-run","creator_id":"beta","handle":"@beta",
  "video_ids":["beta-1","beta-2","beta-3","beta-4"]}
JSON
common_env=(
  CANONICAL_CHANNELS_FILE="$temporary_directory/channels.txt"
  CATALOG_SNAPSHOT_JSON_PATH="$temporary_directory/catalog.json"
  FAILURE_REPORT_JSON_PATH="$temporary_directory/failures.json"
  DISCOVERY_MANIFEST_DIRECTORY="$temporary_directory/discoveries"
)
output="$(env "${common_env[@]}" "$SCRIPT_DIR/check-raw-creator-reconciliation.sh")"
jq -e '
  .ok == true and .expected_profiles == 2 and .active_profiles == 2 and
  .indexed_videos == 6 and .reported_public_videos == 7 and
  .reported_gap_videos == 1 and .classified_records == 1
' <<< "$output" >/dev/null

assert_rejected() {
  local name="$1"
  local catalog="$2"
  local failures="$3"
  if CANONICAL_CHANNELS_FILE="$temporary_directory/channels.txt" \
    CATALOG_SNAPSHOT_JSON_PATH="$catalog" FAILURE_REPORT_JSON_PATH="$failures" \
    DISCOVERY_MANIFEST_DIRECTORY="$temporary_directory/discoveries" \
    "$SCRIPT_DIR/check-raw-creator-reconciliation.sh" >/dev/null 2>&1; then
    printf 'reconciliation checker accepted %s\n' "$name" >&2
    exit 1
  fi
}

jq 'del(.channels[1])' "$temporary_directory/catalog.json" > "$temporary_directory/missing.json"
assert_rejected 'a missing canonical profile' "$temporary_directory/missing.json" \
  "$temporary_directory/failures.json"

jq '.total_records = 0 | .structured_records = 0 | .attributed_records = 0 |
  .totals = {total:0,retryable:0,ready_retry:0,terminal:0,categories:{}} |
  .by_creator = {} | .records = []' "$temporary_directory/failures.json" \
  > "$temporary_directory/unclassified.json"
assert_rejected 'an unclassified public-video gap' "$temporary_directory/catalog.json" \
  "$temporary_directory/unclassified.json"

jq '.legacy_records = 1 | .total_records = 2' "$temporary_directory/failures.json" \
  > "$temporary_directory/legacy.json"
assert_rejected 'a legacy outcome' "$temporary_directory/catalog.json" \
  "$temporary_directory/legacy.json"

jq '.totals.retryable = 1 | .totals.ready_retry = 1 | .totals.terminal = 0 |
  .totals.categories = {import:1} |
  .by_creator.alpha.retryable = 1 | .by_creator.alpha.ready_retry = 1 |
  .by_creator.alpha.terminal = 0 | .by_creator.alpha.categories = {import:1} |
  .records[0].category = "import" | .records[0].retry_state = "ready"' \
  "$temporary_directory/failures.json" \
  > "$temporary_directory/ready-retry.json"
assert_rejected 'a retry whose cooldown elapsed' "$temporary_directory/catalog.json" \
  "$temporary_directory/ready-retry.json"

jq '.totals.categories = {unknown:1} | .by_creator.alpha.categories = {unknown:1} |
  .records[0].category = "unknown"' \
  "$temporary_directory/failures.json" \
  > "$temporary_directory/unknown.json"
assert_rejected 'an unknown outcome' "$temporary_directory/catalog.json" \
  "$temporary_directory/unknown.json"

jq '.total_records = 2 | .structured_records = 2 | .attributed_records = 2 |
  .totals.total = 2 | .totals.terminal = 2 |
  .totals.categories = {no_captions:1,unavailable:1} |
  .by_creator.removed = {total:1,retryable:0,ready_retry:0,terminal:1,categories:{unavailable:1}} |
  .records += [{video_id:"removed-1",creator_id:"removed",category:"unavailable",retry_state:"terminal"}]' \
  "$temporary_directory/failures.json" > "$temporary_directory/removed-creator.json"
assert_rejected 'outcomes attributed to a removed creator' "$temporary_directory/catalog.json" \
  "$temporary_directory/removed-creator.json"

jq '.records[0].video_id = "old-deleted-video"' "$temporary_directory/failures.json" \
  > "$temporary_directory/stale-failure.json"
assert_rejected 'a stale classified ID masking a current missing video' \
  "$temporary_directory/catalog.json" "$temporary_directory/stale-failure.json"

jq '.by_creator.alpha.ready_retry = 1 | .by_creator.alpha.retryable = 1 |
  .by_creator.alpha.terminal = 0 | .by_creator.alpha.categories = {import:1} |
  .records[0].category = "import" | .records[0].retry_state = "ready"' \
  "$temporary_directory/failures.json" > "$temporary_directory/creator-ready-drift.json"
assert_rejected 'per-creator ready-retry drift from the aggregate' \
  "$temporary_directory/catalog.json" "$temporary_directory/creator-ready-drift.json"

jq '.by_creator.alpha.categories = {unknown:1} |
  .records[0].category = "unknown"' "$temporary_directory/failures.json" \
  > "$temporary_directory/creator-unknown-drift.json"
assert_rejected 'per-creator unknown drift from the aggregate' \
  "$temporary_directory/catalog.json" "$temporary_directory/creator-unknown-drift.json"

jq '.records[0].category = "private"' "$temporary_directory/failures.json" \
  > "$temporary_directory/record-category-drift.json"
assert_rejected 'an exact record category drifting from count buckets' \
  "$temporary_directory/catalog.json" "$temporary_directory/record-category-drift.json"

jq '.records[0].retry_state = "deferred"' "$temporary_directory/failures.json" \
  > "$temporary_directory/record-state-drift.json"
assert_rejected 'an exact record retry state drifting from count buckets' \
  "$temporary_directory/catalog.json" "$temporary_directory/record-state-drift.json"

jq '.video_ids_by_creator.alpha = ["alpha-1","stale-indexed-video"]' \
  "$temporary_directory/catalog.json" > "$temporary_directory/stale-indexed.json"
assert_rejected 'a stale indexed ID masking a missing video' \
  "$temporary_directory/stale-indexed.json" "$temporary_directory/failures.json"

mkdir -p "$temporary_directory/stale-run"
cp "$temporary_directory/discoveries/"*.json "$temporary_directory/stale-run/"
cp "$temporary_directory/discoveries/active-run-id" "$temporary_directory/stale-run/active-run-id"
jq '.run_id = "older-catalog-run"' "$temporary_directory/stale-run/beta.json" \
  > "$temporary_directory/stale-run/beta.next.json"
mv "$temporary_directory/stale-run/beta.next.json" "$temporary_directory/stale-run/beta.json"
if env "${common_env[@]}" \
  DISCOVERY_MANIFEST_DIRECTORY="$temporary_directory/stale-run" \
  "$SCRIPT_DIR/check-raw-creator-reconciliation.sh" >/dev/null 2>&1; then
  printf 'reconciliation checker accepted discovery evidence from an older catalog run\n' >&2
  exit 1
fi

jq '.last_sequence_after = 43' "$temporary_directory/catalog.json" \
  > "$temporary_directory/unpublished.json"
assert_rejected 'a catalog snapshot crossed a mutation sequence' \
  "$temporary_directory/unpublished.json" "$temporary_directory/failures.json"

printf 'raw_creator_reconciliation=ok\n'
