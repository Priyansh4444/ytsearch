#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/api" "$test_dir/data" "$test_dir/tmp"
TMPDIR="$test_dir/tmp"
export TMPDIR

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
pull_queue_tsv() {
  case "${FAKE_WAITING_QUEUE:-empty}" in
    waiting) printf 'request\t@queued\tworker\traw\n' ;;
    failure) return 1 ;;
  esac
}
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
TRANSCRIBE_MISSING=1
WHISPER_MODEL="$test_dir/model with spaces.bin"
: > "$catalog_command_log"
index_catalog_backend raw raw 1
index_one_backend raw raw 1 @alpha
[ "$(grep -c -- '--transcribe-missing --transcription-backend whisper-cpp' "$catalog_command_log")" -eq 2 ]
[ "$(grep -c -- '--transcription-budget-seconds 480' "$catalog_command_log")" -eq 2 ]
TRANSCRIBE_MISSING=0
index_list_backend() {
  echo "daily refresh must use one catalog-scoped index-youtubers process" >&2
  return 1
}
catalog_input="$test_dir/catalog-input"
catalog_path_log="$test_dir/catalog-path"
cat > "$DAILY_UPDATE_LIST_FILE" <<'LIST'
# Approved creators retain their spelling and labels.
@alpha | Alpha
@Cursor_AI | Cursor label
@ardens. | Ardens label
LIST
cp "$DAILY_UPDATE_LIST_FILE" "$test_dir/canonical-original"
cat > "$test_dir/api/youtubers" <<'JSON'
{"channels":[
  {"handle":"@complete","public_video_count":10,"indexed_video_count":10},
  {"handle":"@alpha","public_video_count":9,"indexed_video_count":3},
  {"handle":"@cursor-ai","public_video_count":4,"indexed_video_count":4},
  {"handle":"@ardens","public_video_count":2,"indexed_video_count":2},
  {"handle":"@complete","public_video_count":10,"indexed_video_count":10}
],"pending_requests":[{"handle":"@unapproved"}]}
JSON
cp "$test_dir/api/youtubers" "$test_dir/approved-directory"
index_catalog_backend() {
  printf 'daily-catalog %s\n' "$*" >> "$index_log"
  cat "$DAILY_UPDATE_LIST_FILE" > "$catalog_input"
  printf '%s\n' "$DAILY_UPDATE_LIST_FILE" > "$catalog_path_log"
}
assert_no_daily_catalog_files() {
  for catalog_file in "$TMPDIR"/ytsearch-daily-catalog.*; do
    if [ -e "$catalog_file" ]; then
      echo "daily catalog temporary file leaked: $catalog_file" >&2
      exit 1
    fi
  done
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

# The canonical source stays read-only. Approved creators with zero count
# difference still join the same full catalog process, unlike gap repair.
cmp "$DAILY_UPDATE_LIST_FILE" "$test_dir/canonical-original"
[ "$(cat "$catalog_path_log")" != "$DAILY_UPDATE_LIST_FILE" ]
[ ! -e "$(cat "$catalog_path_log")" ]
[ "$(grep -c '^@' "$catalog_input")" -eq 4 ]
grep -Fx '# Approved creators retain their spelling and labels.' "$catalog_input" >/dev/null
grep -Fx '@Cursor_AI | Cursor label' "$catalog_input" >/dev/null
grep -Fx '@ardens. | Ardens label' "$catalog_input" >/dev/null
grep -Fx '@complete' "$catalog_input" >/dev/null
if grep -Eq '^@(cursor-ai|ardens|unapproved)$' "$catalog_input"; then
  echo "daily catalog must deduplicate stable creator aliases and exclude pending requests" >&2
  exit 1
fi
assert_no_daily_catalog_files

# A malformed or unavailable directory fails before any indexing, publication,
# publication marker, or completion timestamp can be written.
remember_daily_update_completion 1234
for malformed_directory in \
  '[]' '{}' '{"channels":{}}' '{"channels":[null]}' \
  '{"channels":[{"handle":null}]}' '{"channels":[{"handle":42}]}' \
  '{"channels":[{"handle":""}]}' '{"channels":[{"handle":"@bad|entry"}]}' \
  '{"channels":[{"handle":"@good"},{"handle":"@bad\nentry"}]}' 'invalid json'; do
  printf '%s\n' "$malformed_directory" > "$test_dir/api/youtubers"
  : > "$index_log"
  : > "$publication_log"
  rm -f "$RAW_PENDING_PUBLICATION_FILE"
  if index_daily_update_for_backends > /dev/null 2>&1; then
    echo "a malformed approved directory must fail the daily sweep: $malformed_directory" >&2
    exit 1
  fi
  [ ! -s "$index_log" ] && [ ! -s "$publication_log" ]
  [ ! -e "$RAW_PENDING_PUBLICATION_FILE" ]
  [ "$(cat "$DAILY_UPDATE_STATE_FILE")" = 1234 ]
  cmp "$DAILY_UPDATE_LIST_FILE" "$test_dir/canonical-original"
  assert_no_daily_catalog_files
done
rm "$test_dir/api/youtubers"
if index_daily_update_for_backends > /dev/null 2>&1; then
  echo "an unavailable approved directory must fail the daily sweep" >&2
  exit 1
fi
[ ! -s "$index_log" ] && [ ! -s "$publication_log" ]
[ ! -e "$RAW_PENDING_PUBLICATION_FILE" ]
[ "$(cat "$DAILY_UPDATE_STATE_FILE")" = 1234 ]
assert_no_daily_catalog_files
cp "$test_dir/approved-directory" "$test_dir/api/youtubers"

# A failed or blocked pass leaves a routine marker and does not rebuild the
# generation again while the last publication is recent.
ROUTINE_PUBLICATION_INTERVAL_SECONDS=21600
for catalog_exit in 1 69; do
  : > "$publication_log"
  rm -f "$RAW_PENDING_PUBLICATION_FILE"
  remember_epoch "$RAW_LAST_PUBLICATION_FILE" "$(date +%s)"
  index_catalog_backend() { return "$catalog_exit"; }
  if index_daily_update_for_backends >/dev/null; then
    echo "a daily catalog exit $catalog_exit must fail the daily update" >&2
    exit 1
  fi
  [ ! -s "$publication_log" ] || {
    echo "a failed daily pass (exit $catalog_exit) must not publish before the routine interval" >&2
    exit 1
  }
  assert_no_daily_catalog_files
  grep -q ' routine$' "$RAW_PENDING_PUBLICATION_FILE" || {
    echo "a failed daily pass must leave a routine publication marker" >&2
    exit 1
  }
done
# Every creator attempted with only retryable gaps still publishes at once.
index_catalog_backend() { return 75; }
if ! index_daily_update_for_backends >/dev/null; then
  echo "a daily pass with only retryable gaps must complete" >&2
  exit 1
fi
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "a completed daily pass must publish at once despite the routine interval" >&2
  exit 1
}
ROUTINE_PUBLICATION_INTERVAL_SECONDS=0
index_catalog_backend() {
  printf 'daily-catalog %s\n' "$*" >> "$index_log"
}

# Failed passes back off 1h, 2h, 4h, then hold at the 6h maximum, and the
# retry time survives a worker restart.
DAILY_UPDATE_FAILURE_RETRY_SECONDS=3600
DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS=21600
rm -f "$DAILY_UPDATE_RETRY_FILE"
for expected_delay in 3600 7200 14400 21600 21600; do
  before="$(date +%s)"
  retry_at="$(remember_daily_update_failure)"
  delay=$(( retry_at - before ))
  [ "$delay" -ge "$expected_delay" ] && [ "$delay" -le $(( expected_delay + 2 )) ] || {
    echo "expected a daily retry delay of $expected_delay, got: $delay" >&2
    exit 1
  }
done
[ "$(daily_update_retry_field 2)" = "5" ] || {
  echo "expected five consecutive daily failures to be recorded" >&2
  exit 1
}
remember_daily_update_completion 1000
remember_epoch "$DAILY_UPDATE_RETRY_FILE" "200000 1"
[ "$(next_daily_update_epoch 190000)" = "200000" ] || {
  echo "a persisted retry time must delay an overdue daily pass" >&2
  exit 1
}
[ "$(next_daily_update_epoch 100000)" = "87400" ] || {
  echo "a retry time beyond the maximum backoff must be ignored" >&2
  exit 1
}
remember_daily_update_completion 1000
[ ! -f "$DAILY_UPDATE_RETRY_FILE" ] || {
  echo "a completed daily pass must clear its retry state" >&2
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

# A queued request or an unreadable queue yields before another creator.
for queue_state in waiting failure; do
  FAKE_WAITING_QUEUE="$queue_state"
  printf '%s\n' @alpha @bravo > "$gap_handles"
  : > "$index_log"
  : > "$publication_log"
  process_idle_gap_batch 4
  [ "$IDLE_GAP_BATCH_PASSES" -eq 1 ]
  [ "$(wc -l < "$index_log" | tr -d ' ')" -eq 1 ]
  [ "$(sed -n '1p' "$gap_handles")" = @bravo ]
done
FAKE_WAITING_QUEUE=empty

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
