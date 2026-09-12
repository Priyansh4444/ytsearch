#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CANONICAL_CHANNELS_FILE="${CANONICAL_CHANNELS_FILE:-$SCRIPT_DIR/indexed-channels.txt}"
CATALOG_SNAPSHOT_JSON_PATH="${CATALOG_SNAPSHOT_JSON_PATH:-}"
FAILURE_REPORT_JSON_PATH="${FAILURE_REPORT_JSON_PATH:-}"
TRANSCRIPT_CACHE_DIRECTORY="${TRANSCRIPT_CACHE_DIRECTORY:-/var/lib/ytsearch/transcript-cache}"
DISCOVERY_MANIFEST_DIRECTORY="${DISCOVERY_MANIFEST_DIRECTORY:-$TRANSCRIPT_CACHE_DIRECTORY/.reconciliation}"
YTSEARCH_BIN="${YTSEARCH_BIN:-ytsearch}"
TRANSCRIPT_FAILURE_RETRY_SECONDS="${TRANSCRIPT_FAILURE_RETRY_SECONDS:-21600}"

for command in jq awk; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

[[ -f "$CANONICAL_CHANNELS_FILE" ]] || {
  printf 'canonical channel list does not exist: %s\n' "$CANONICAL_CHANNELS_FILE" >&2
  exit 1
}
[[ "$TRANSCRIPT_FAILURE_RETRY_SECONDS" =~ ^[1-9][0-9]*$ ]] || {
  printf 'TRANSCRIPT_FAILURE_RETRY_SECONDS must be a positive integer\n' >&2
  exit 1
}

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM

if [[ -z "$CATALOG_SNAPSHOT_JSON_PATH" ]]; then
  : "${RAW_SEARCH_ADMIN_API_KEY:?set RAW_SEARCH_ADMIN_API_KEY to read the active Raw catalog}"
  if [[ "$(printf '%s' "$RAW_SEARCH_ADMIN_API_KEY" | tr -d '\r\n')" != \
    "$RAW_SEARCH_ADMIN_API_KEY" ]]; then
    printf 'RAW_SEARCH_ADMIN_API_KEY must be a single-line value\n' >&2
    exit 1
  fi
  # shellcheck source=scripts/lib/fly-production-inventory.sh
  source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
  fly_production_inventory_load_raw_runtime
  RAW_SEARCH_API_URL="${RAW_SEARCH_API_URL:-https://${RAW_READER_APP}.fly.dev}"
  command -v curl >/dev/null 2>&1 || {
    printf 'missing required command: curl\n' >&2
    exit 1
  }
  printf 'x-raw-admin-key: %s\n' "$RAW_SEARCH_ADMIN_API_KEY" \
    > "$temporary_directory/admin-header"
  chmod 600 "$temporary_directory/admin-header"
  CATALOG_SNAPSHOT_JSON_PATH="$temporary_directory/catalog.json"
  curl -fsS --max-time 60 -H "@$temporary_directory/admin-header" \
    "${RAW_SEARCH_API_URL%/}/admin/active/catalog" > "$CATALOG_SNAPSHOT_JSON_PATH"
fi
if [[ -z "$FAILURE_REPORT_JSON_PATH" ]]; then
  command -v "$YTSEARCH_BIN" >/dev/null 2>&1 || {
    printf 'missing ytsearch executable: %s\n' "$YTSEARCH_BIN" >&2
    exit 1
  }
  FAILURE_REPORT_JSON_PATH="$temporary_directory/failures.json"
  "$YTSEARCH_BIN" failure-report "$TRANSCRIPT_CACHE_DIRECTORY" \
    --transcript-failure-retry-seconds "$TRANSCRIPT_FAILURE_RETRY_SECONDS" \
    > "$FAILURE_REPORT_JSON_PATH"
fi

shopt -s nullglob
discovery_manifest_files=("$DISCOVERY_MANIFEST_DIRECTORY"/*.json)
shopt -u nullglob
if (( ${#discovery_manifest_files[@]} == 0 )); then
  printf 'no complete-discovery manifests found in %s\n' "$DISCOVERY_MANIFEST_DIRECTORY" >&2
  exit 1
fi
if [[ ! -f "$DISCOVERY_MANIFEST_DIRECTORY/active-run-id" ]]; then
  printf 'catalog reconciliation run marker is missing: %s\n' \
    "$DISCOVERY_MANIFEST_DIRECTORY/active-run-id" >&2
  exit 1
fi
reconciliation_run_id="$(tr -d '\r\n' < "$DISCOVERY_MANIFEST_DIRECTORY/active-run-id")"
if [[ ! "$reconciliation_run_id" =~ ^[A-Za-z0-9._-]{8,128}$ ]]; then
  printf 'catalog reconciliation run marker is invalid\n' >&2
  exit 1
fi
jq -s '{manifests:.}' "${discovery_manifest_files[@]}" \
  > "$temporary_directory/discoveries.json"

for input in "$CATALOG_SNAPSHOT_JSON_PATH" "$FAILURE_REPORT_JSON_PATH" \
  "$temporary_directory/discoveries.json"; do
  jq -e . "$input" >/dev/null || {
    printf 'invalid reconciliation JSON: %s\n' "$input" >&2
    exit 1
  }
done

awk -F '|' '
  /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
  {
    handle = $1
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", handle)
    print handle
  }
' "$CANONICAL_CHANNELS_FILE" |
  jq -Rsc 'split("\n") | map(select(length > 0) | ascii_downcase) | sort' \
    > "$temporary_directory/expected-handles.json"

result="$(jq -cn \
  --arg reconciliation_run_id "$reconciliation_run_id" \
  --slurpfile expected "$temporary_directory/expected-handles.json" \
  --slurpfile catalog_input "$CATALOG_SNAPSHOT_JSON_PATH" \
  --slurpfile failure_input "$FAILURE_REPORT_JSON_PATH" \
  --slurpfile discovery_input "$temporary_directory/discoveries.json" '
  def nonnegative_integer:
    type == "number" and . >= 0 and floor == .;
  def counts_valid:
    (.total | nonnegative_integer) and
    (.retryable | nonnegative_integer) and
    (.ready_retry | nonnegative_integer) and
    (.terminal | nonnegative_integer) and
    (.categories | type) == "object" and
    ([.categories[] | select(nonnegative_integer | not)] | length) == 0 and
    ([.categories[]] | add // 0) == .total and
    (.retryable + .terminal) == .total and
    .ready_retry <= .retryable;
  def record_counts:
    reduce .[] as $record (
      {total:0,retryable:0,ready_retry:0,terminal:0,categories:{}};
      .total += 1 |
      if $record.retry_state == "terminal" then
        .terminal += 1
      else
        .retryable += 1 |
        if $record.retry_state == "ready" then .ready_retry += 1 else . end
      end |
      .categories[$record.category] = ((.categories[$record.category] // 0) + 1)
    );
  ($expected[0]) as $expected_handles |
  ($catalog_input[0]) as $catalog |
  ($failure_input[0]) as $failures |
  ($catalog.video_ids_by_creator // {}) as $indexed_by_creator |
  ($discovery_input[0].manifests // []) as $manifests |
  ($catalog.channels // []) as $channels |
  [$channels[]? | (.handle // "" | ascii_downcase)] as $live_handles |
  [$channels[]? | .id // ""] as $live_ids |
  ($live_handles | sort) as $sorted_live_handles |
  ($expected_handles - $sorted_live_handles) as $missing_handles |
  ($sorted_live_handles - $expected_handles) as $unexpected_handles |
  [$channels[]? | select(
    ((.id | type) != "string") or (.id | length) == 0 or
    ((.handle | type) != "string") or (.handle | length) == 0 or
    (.public_video_count | nonnegative_integer | not) or
    (.indexed_video_count | nonnegative_integer | not) or
    (.indexed_chunk_count | nonnegative_integer | not) or
    .indexed_video_count == 0 or .indexed_chunk_count == 0
  ) | .handle // .id // "unknown"] as $invalid_profiles |
  [($failures.by_creator // {}) | keys[] as $creator_id |
    select(($live_ids | index($creator_id)) == null) | $creator_id
  ] as $unexpected_failure_creators |
  [($indexed_by_creator | keys[]) as $creator_id |
    select(($live_ids | index($creator_id)) == null) | $creator_id
  ] as $unexpected_indexed_creators |
  [$manifests[]?.creator_id] as $manifest_creator_ids |
  [$manifests[]?.video_ids[]?] as $all_discovered_video_ids |
  [($indexed_by_creator // {})[]?[]?] as $all_indexed_video_ids |
  [($manifest_creator_ids | unique[]) as $creator_id |
    select(($live_ids | index($creator_id)) == null) | $creator_id
  ] as $unexpected_manifest_creators |
  [$channels[]? as $channel |
    ([$manifests[]? | select(.creator_id == $channel.id)]) as $creator_manifests |
    ($creator_manifests[0] // {}) as $manifest |
    ($manifest.video_ids // []) as $discovered |
    ($indexed_by_creator[$channel.id] // []) as $indexed |
    [$failures.records[]? | select(.creator_id == $channel.id) | .video_id] as $classified |
    {
      id:$channel.id,
      handle:$channel.handle,
      manifest_count:($creator_manifests | length),
      manifest_valid:(
        ($creator_manifests | length) == 1 and
        $manifest.schema_version == 1 and $manifest.status == "complete" and
        $manifest.run_id == $reconciliation_run_id and
        ($manifest.handle | ascii_downcase) == ($channel.handle | ascii_downcase) and
        ($discovered | all(type == "string" and length > 0)) and
        ($discovered | unique | length) == ($discovered | length)
      ),
      discovered_videos:($discovered | length),
      indexed_videos:($indexed | length),
      classified_videos:($classified | length),
      public_count_matches_discovery:($channel.public_video_count == ($discovered | length)),
      indexed_count_matches_ids:($channel.indexed_video_count == ($indexed | length)),
      missing_outcomes:($discovered - $indexed - $classified),
      stale_indexed:($indexed - $discovered),
      stale_classified:($classified - $discovered),
      indexed_and_classified:([
        $indexed[] as $video_id |
        select(($classified | index($video_id)) != null) |
        $video_id
      ]),
      duplicate_indexed_ids:(($indexed | unique | length) != ($indexed | length)),
      duplicate_classified_ids:(($classified | unique | length) != ($classified | length))
    }
  ] as $creator_evidence |
  [$creator_evidence[] | select(
    (.manifest_valid | not) or (.public_count_matches_discovery | not) or
    (.indexed_count_matches_ids | not) or (.missing_outcomes | length) > 0 or
    (.stale_indexed | length) > 0 or (.stale_classified | length) > 0 or
    (.indexed_and_classified | length) > 0 or .duplicate_indexed_ids or
    .duplicate_classified_ids
  )] as $creator_failures |
  (($failures.totals // {}) | counts_valid) as $aggregate_counts_valid |
  ([($failures.by_creator // {})[] | select(counts_valid | not)] | length == 0)
    as $creator_counts_valid |
  (($failures.total_records // -1) ==
    (($failures.structured_records // -2) + ($failures.legacy_records // -3))) as $record_total_valid |
  (($failures.structured_records // -1) ==
    (($failures.attributed_records // -2) + ($failures.unattributed_records // -3))) as $attribution_total_valid |
  (($failures.totals.total // -1) == ($failures.structured_records // -2))
    as $structured_total_valid |
  ([($failures.by_creator // {})[].total] | add // 0) as $creator_record_total |
  (reduce (($failures.by_creator // {})[] // empty) as $counts (
    {total:0,retryable:0,ready_retry:0,terminal:0,categories:{}};
    .total += $counts.total |
    .retryable += $counts.retryable |
    .ready_retry += $counts.ready_retry |
    .terminal += $counts.terminal |
    reduce ($counts.categories | to_entries[]) as $entry (.;
      .categories[$entry.key] = ((.categories[$entry.key] // 0) + $entry.value))
  )) as $summed_creator_counts |
  ($summed_creator_counts | .categories |= with_entries(select(.value != 0))) as $normalized_creator_counts |
  (($failures.totals // {}) == $normalized_creator_counts) as $aggregate_matches_creators |
  (($failures.records // []) | record_counts) as $counts_from_records |
  (reduce (($failures.records // [])[]?) as $record ({};
    .[$record.creator_id] = ((.[$record.creator_id] // []) + [$record])
  ) | with_entries(.value |= record_counts)) as $creator_counts_from_records |
  (($failures.totals // {}) == $counts_from_records) as $aggregate_matches_records |
  (($failures.by_creator // {}) == $creator_counts_from_records) as $creators_match_records |
  (([$failures.records[]? | .video_id] | unique | length) ==
    ([$failures.records[]? | .video_id] | length)) as $failure_video_ids_unique |
  (($all_discovered_video_ids | unique | length) ==
    ($all_discovered_video_ids | length)) as $discovered_video_ids_unique |
  (($all_indexed_video_ids | unique | length) ==
    ($all_indexed_video_ids | length)) as $indexed_video_ids_unique |
  (($failures.records | type) == "array" and all($failures.records[]?;
    (.video_id | type) == "string" and (.video_id | length) > 0 and
    (.creator_id | type) == "string" and (.creator_id | length) > 0 and
    (.category | type) == "string" and (.retry_state | type) == "string"
  )) as $record_schema_valid |
  ([ $failures.records[]? | select(
      .category == "unknown" or .retry_state == "ready" or
      (.retry_state != "terminal" and .retry_state != "deferred")
    ) ] | length == 0) as $record_states_valid |
  {
    ok:(
      ($expected_handles | length) > 0 and
      ($expected_handles | unique | length) == ($expected_handles | length) and
      ($live_handles | unique | length) == ($live_handles | length) and
      ($live_ids | unique | length) == ($live_ids | length) and
      ($missing_handles | length) == 0 and
      ($unexpected_handles | length) == 0 and
      ($invalid_profiles | length) == 0 and
      $record_total_valid and $attribution_total_valid and
      $structured_total_valid and $aggregate_counts_valid and
      $creator_counts_valid and $aggregate_matches_creators and
      $aggregate_matches_records and $creators_match_records and
      $creator_record_total == ($failures.attributed_records // -1) and
      ($failures.records | length) == ($failures.structured_records // -1) and
      $failure_video_ids_unique and $discovered_video_ids_unique and
      $indexed_video_ids_unique and $record_schema_valid and $record_states_valid and
      ($catalog.generation | nonnegative_integer) and
      $catalog.generation == $catalog.last_sequence_before and
      $catalog.generation == $catalog.last_sequence_after and
      ($failures.legacy_records // -1) == 0 and
      ($failures.unattributed_records // -1) == 0 and
      ($failures.totals.ready_retry // -1) == 0 and
      ($failures.totals.categories.unknown // 0) == 0 and
      ($unexpected_failure_creators | length) == 0 and
      ($unexpected_indexed_creators | length) == 0 and
      ($unexpected_manifest_creators | length) == 0 and
      ($manifest_creator_ids | unique | length) == ($manifest_creator_ids | length) and
      ($creator_failures | length) == 0
    ),
    expected_profiles:($expected_handles | length),
    active_profiles:($channels | length),
    indexed_videos:([$channels[]?.indexed_video_count] | add // 0),
    reported_public_videos:([$channels[]?.public_video_count] | add // 0),
    reported_gap_videos:([$channels[]? |
      [.public_video_count - .indexed_video_count, 0] | max] | add // 0),
    classified_records:($failures.structured_records // null),
    retryable_records:($failures.totals.retryable // null),
    terminal_records:($failures.totals.terminal // null),
    ready_retry_records:($failures.totals.ready_retry // null),
    generation:($catalog.generation // null),
    missing_handles:$missing_handles,
    unexpected_handles:$unexpected_handles,
    invalid_profiles:$invalid_profiles,
    legacy_records:($failures.legacy_records // null),
    unattributed_records:($failures.unattributed_records // null),
    unknown_records:($failures.totals.categories.unknown // 0),
    unexpected_failure_creators:$unexpected_failure_creators,
    unexpected_indexed_creators:$unexpected_indexed_creators,
    unexpected_manifest_creators:$unexpected_manifest_creators,
    creator_failures:$creator_failures
  }
')"

printf '%s\n' "$result"
jq -e '.ok == true' <<< "$result" >/dev/null
