#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/api" "$test_dir/data"

CLOUD_INDEX_QUEUE_LIBRARY_ONLY=1
INDEX_QUEUE_API_BASE_URL="file://$test_dir"
IDLE_GAP_FILL_STATE_FILE="$test_dir/data/cursor"
DAILY_UPDATE_STATE_FILE="$test_dir/data/daily-update.last-completed"
DAILY_UPDATE_INITIAL_DELAY_SECONDS=0
DAILY_UPDATE_INTERVAL_SECONDS=86400
TRANSCRIPT_CACHE_DIR="$test_dir/data/transcript-cache"
RAW_PENDING_PUBLICATION_FILE="$test_dir/data/raw-publication.pending"
RAW_LAST_PUBLICATION_FILE="$test_dir/data/raw-publication.last-completed"
IDLE_GAP_FILL_STARTED_FILE="$test_dir/data/idle-gap-fill.last-started"
export CLOUD_INDEX_QUEUE_LIBRARY_ONLY INDEX_QUEUE_API_BASE_URL IDLE_GAP_FILL_STATE_FILE DAILY_UPDATE_STATE_FILE DAILY_UPDATE_INITIAL_DELAY_SECONDS DAILY_UPDATE_INTERVAL_SECONDS TRANSCRIPT_CACHE_DIR RAW_PENDING_PUBLICATION_FILE RAW_LAST_PUBLICATION_FILE IDLE_GAP_FILL_STARTED_FILE
. "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"

status=75
log_queue_event error status_regression "" "@status" "" raw failed >/dev/null 2>&1
[ "$status" -eq 75 ] || {
  echo "queue logging must not overwrite a caller's status" >&2
  exit 1
}

cat > "$test_dir/api/youtubers" <<'JSON'
{"channels":[
  {"handle":"@complete","public_video_count":10,"indexed_video_count":10},
  {"handle":"@bravo","public_video_count":12,"indexed_video_count":10},
  {"handle":"@alpha","public_video_count":9,"indexed_video_count":3}
]}
JSON

first="$(pull_idle_gap_handle)"
[ "$first" = "@alpha" ] || {
  echo "expected @alpha first, got: $first" >&2
  exit 1
}

remember_idle_gap_handle "$first"
second="$(pull_idle_gap_handle)"
[ "$second" = "@bravo" ] || {
  echo "expected @bravo second, got: $second" >&2
  exit 1
}

remember_idle_gap_handle "$second"
wrapped="$(pull_idle_gap_handle)"
[ "$wrapped" = "@alpha" ] || {
  echo "expected rotation to wrap to @alpha, got: $wrapped" >&2
  exit 1
}

first_daily="$(next_daily_update_epoch 1000)"
[ "$first_daily" = "1000" ] || {
  echo "expected an immediate first daily update, got: $first_daily" >&2
  exit 1
}

remember_daily_update_completion 1000
next_daily="$(next_daily_update_epoch 2000)"
[ "$next_daily" = "87400" ] || {
  echo "expected persisted daily cadence at 87400, got: $next_daily" >&2
  exit 1
}

printf '%s\n' invalid > "$DAILY_UPDATE_STATE_FILE"
recovered_daily="$(next_daily_update_epoch 3000)"
[ "$recovered_daily" = "3000" ] || {
  echo "expected invalid daily state to recover immediately, got: $recovered_daily" >&2
  exit 1
}

gap_handles="$test_dir/gap-handles"
index_log="$test_dir/index.log"
publication_log="$test_dir/publication.log"
printf '%s\n' @alpha @bravo > "$gap_handles"
pull_idle_gap_handle() {
  handle="$(sed -n '1p' "$gap_handles")"
  sed '1d' "$gap_handles" > "$gap_handles.next"
  mv "$gap_handles.next" "$gap_handles"
  printf '%s\n' "$handle"
}
remember_idle_gap_handle() { :; }
set_index_activity() { :; }
log_queue_event() { :; }
index_youtuber_for_backends() {
  printf '%s\n' "$*" >> "$index_log"
}
publish_raw_generation() {
  printf 'publish\n' >> "$publication_log"
}

safe_pending_file="$RAW_PENDING_PUBLICATION_FILE"
RAW_PENDING_PUBLICATION_FILE="/proc/ytsearch-raw-publication.pending"
printf '%s\n' @marker-failure > "$gap_handles"
: > "$index_log"
: > "$publication_log"
if process_idle_gap_batch 1; then
  echo "an unwritable publication marker must fail the gap batch" >&2
  exit 1
fi
[ "$IDLE_GAP_BATCH_PASSES" -eq 0 ]
if [ -s "$index_log" ] || [ -s "$publication_log" ]; then
  echo "marker failure must prevent gap mutation and publication" >&2
  exit 1
fi

DAILY_UPDATE_LIST_FILE="$test_dir/daily-list"
if index_daily_update_for_backends; then
  echo "a missing creator list must not count as a completed daily update" >&2
  exit 1
fi
for contents in '' '# no creators'; do
  printf '%s\n' "$contents" > "$DAILY_UPDATE_LIST_FILE"
  if index_daily_update_for_backends; then
    echo "an empty or comment-only creator list must not count as completion" >&2
    exit 1
  fi
done
printf '%s\n' '@alpha | Alpha' > "$DAILY_UPDATE_LIST_FILE"
RAW_SEARCH_API_URL="http://raw.invalid"
RAW_SEARCH_ADMIN_API_KEY="test-key"
catalog_command_log="$test_dir/catalog-command.log"
ytsearch() { printf '%s\n' "$*" >> "$catalog_command_log"; }
index_catalog_backend raw raw 3
[ "$(wc -l < "$catalog_command_log" | tr -d ' ')" -eq 1 ]
grep -F "index-youtubers $DAILY_UPDATE_LIST_FILE --backend raw --mode update" \
  "$catalog_command_log" >/dev/null
grep -F -- '--jobs 3 --channel-jobs 1' "$catalog_command_log" >/dev/null
grep -F -- '--defer-publish' "$catalog_command_log" >/dev/null
index_list_backend() {
  echo "daily refresh must use one catalog-scoped index-youtubers process" >&2
  return 1
}
index_catalog_backend() {
  printf 'daily-catalog %s\n' "$*" >> "$index_log"
}
if index_daily_update_for_backends; then
  echo "an unwritable publication marker must fail the daily update" >&2
  exit 1
fi
if [ -s "$index_log" ] || [ -s "$publication_log" ]; then
  echo "marker failure must prevent daily mutation and publication" >&2
  exit 1
fi
RAW_PENDING_PUBLICATION_FILE="$safe_pending_file"
: > "$index_log"
: > "$publication_log"
if ! index_daily_update_for_backends; then
  echo "a valid daily catalog refresh must succeed" >&2
  exit 1
fi
[ "$(grep -c '^daily-catalog raw raw ' "$index_log")" -eq 1 ] || {
  echo "daily refresh must run one Raw catalog process" >&2
  exit 1
}
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "daily refresh must publish exactly one generation" >&2
  exit 1
}
[ ! -f "$RAW_PENDING_PUBLICATION_FILE" ] || {
  echo "successful daily publication must clear its durable marker" >&2
  exit 1
}
: > "$index_log"
: > "$publication_log"
printf '%s\n' @alpha @bravo > "$gap_handles"

process_idle_gap_batch 4
[ "$IDLE_GAP_HAD_WORK" = "1" ]
[ "$IDLE_GAP_BATCH_PASSES" -eq 2 ]
[ "$(wc -l < "$index_log" | tr -d ' ')" -eq 2 ]
[ "$(grep -c -- '--defer-publish' "$index_log")" -eq 2 ] || {
  echo "every gap repair must defer publication until the batch boundary" >&2
  exit 1
}
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "one gap batch must build exactly one generation" >&2
  exit 1
}

printf '%s\n' @publication-failure > "$gap_handles"
: > "$index_log"
: > "$publication_log"
index_youtuber_for_backends() {
  printf '%s\n' "$*" >> "$index_log"
}
publish_raw_generation() {
  printf 'failed\n' >> "$publication_log"
  return 1
}
if process_idle_gap_batch 4; then
  echo "a failed generation must fail the gap batch" >&2
  exit 1
fi
[ -f "$RAW_PENDING_PUBLICATION_FILE" ] || {
  echo "a failed generation must retain the durable publication marker" >&2
  exit 1
}
[ "$(wc -l < "$index_log" | tr -d ' ')" -eq 1 ]
publish_raw_generation() {
  printf 'recovered\n' >> "$publication_log"
}
publish_pending_raw_generation
[ ! -f "$RAW_PENDING_PUBLICATION_FILE" ]
[ "$(wc -l < "$index_log" | tr -d ' ')" -eq 1 ] || {
  echo "publication recovery must not reindex the creator" >&2
  exit 1
}

printf '%s\n' @retryable > "$gap_handles"
: > "$index_log"
: > "$publication_log"
index_youtuber_for_backends() {
  printf '%s\n' "$*" >> "$index_log"
  return 75
}
if process_idle_gap_batch 4; then
  echo "a retryable gap failure must fail the batch" >&2
  exit 1
fi
[ "$IDLE_GAP_BATCH_PASSES" -eq 1 ]
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "successful mutations before a retryable gap must still publish once" >&2
  exit 1
}

# Routine gap repair publishes at most once per interval; urgent work does not
# wait. Each case starts from no marker.
index_youtuber_for_backends() {
  printf '%s\n' "$*" >> "$index_log"
}
publish_raw_generation() {
  printf 'publish\n' >> "$publication_log"
}
ROUTINE_PUBLICATION_INTERVAL_SECONDS=3600
rm -f "$RAW_PENDING_PUBLICATION_FILE"
remember_epoch "$RAW_LAST_PUBLICATION_FILE" "$(date +%s)"
printf '%s\n' @fresh > "$gap_handles"
: > "$index_log"
: > "$publication_log"
process_idle_gap_batch 4
[ "$(wc -l < "$index_log" | tr -d ' ')" -eq 1 ]
[ ! -s "$publication_log" ] || {
  echo "a routine batch within the interval must not publish" >&2
  exit 1
}
grep -q ' routine$' "$RAW_PENDING_PUBLICATION_FILE" || {
  echo "a deferred routine batch must keep a routine marker" >&2
  exit 1
}
if pending_publication_due; then
  echo "a routine marker must wait for the interval" >&2
  exit 1
fi

mark_raw_publication_pending urgent
pending_publication_due || {
  echo "an urgent marker must publish at once" >&2
  exit 1
}
mark_raw_publication_pending routine
grep -q ' urgent$' "$RAW_PENDING_PUBLICATION_FILE" || {
  echo "routine work must not downgrade an urgent marker" >&2
  exit 1
}

printf '%s\n' "$(date +%s)" > "$RAW_PENDING_PUBLICATION_FILE"
pending_publication_due || {
  echo "a marker from an older worker must count as urgent" >&2
  exit 1
}

rm -f "$RAW_PENDING_PUBLICATION_FILE"
remember_epoch "$RAW_LAST_PUBLICATION_FILE" "$(( $(date +%s) - 7200 ))"
printf '%s\n' @due > "$gap_handles"
: > "$publication_log"
process_idle_gap_batch 4
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "a routine batch past the interval must publish once" >&2
  exit 1
}
[ ! -f "$RAW_PENDING_PUBLICATION_FILE" ]
last="$(cat "$RAW_LAST_PUBLICATION_FILE")"
[ $(( $(date +%s) - last )) -lt 60 ] || {
  echo "a publication must record its completion time" >&2
  exit 1
}

rm -f "$IDLE_GAP_FILL_STARTED_FILE"
interval_elapsed 3600 "$IDLE_GAP_FILL_STARTED_FILE"
remember_epoch "$IDLE_GAP_FILL_STARTED_FILE" "$(date +%s)"
if interval_elapsed 3600 "$IDLE_GAP_FILL_STARTED_FILE"; then
  echo "a gap sweep that just started must not be due again" >&2
  exit 1
fi
interval_elapsed 0 "$IDLE_GAP_FILL_STARTED_FILE"
remember_epoch "$IDLE_GAP_FILL_STARTED_FILE" "$(( $(date +%s) - 3601 ))"
interval_elapsed 3600 "$IDLE_GAP_FILL_STARTED_FILE"
ROUTINE_PUBLICATION_INTERVAL_SECONDS=0

echo "idle gap fill and daily schedule selection passed"
