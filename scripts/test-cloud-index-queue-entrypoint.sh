#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT

printf '%s\n' 'AIza12345678901234567890123456789012345' > "$test_dir/youtube-api-key.txt"
printf '# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t4102444800\tSAPISID\ttest-cookie-value\n' \
  > "$test_dir/yt-dlp-cookies.txt"
mkdir -p "$test_dir/bin"
cat > "$test_dir/bin/ytsearch" <<'SH'
#!/usr/bin/env sh
set -eu
[ "$1" = "init-backend" ]
[ "${FAKE_RAW_BACKEND_COMPATIBLE:-1}" = "1" ]
printf 'Raw backend ready: generation=1, durable_sequence=1\n'
SH
chmod +x "$test_dir/bin/ytsearch"

run_credential_preflight() {
  env \
    PATH="$test_dir/bin:$PATH" \
    FAKE_RAW_BACKEND_COMPATIBLE="${FAKE_RAW_BACKEND_COMPATIBLE:-1}" \
    TRANSCRIBE_MISSING="${TRANSCRIBE_MISSING:-0}" \
    WHISPER_BINARY="${WHISPER_BINARY:-/missing/whisper-cli}" \
    WHISPER_MODEL="${WHISPER_MODEL:-/missing/model.bin}" \
    CLOUD_INDEX_QUEUE_PREFLIGHT_ONLY=1 \
    INDEX_QUEUE_API_BASE_URL=https://queue.invalid \
    YTSEARCH_INDEX_QUEUE_TOKEN=test-token \
    RAW_SEARCH_API_URL=http://raw.invalid:8108 \
    RAW_SEARCH_ADMIN_API_KEY=test-admin-key \
    YOUTUBE_API_KEY_FILE="$1" \
    YT_DLP_COOKIES_FILE="$2" \
    CREDENTIAL_PREFLIGHT_MARKER="$3" \
    CREDENTIAL_PREFLIGHT_GENERATION=test-generation \
    TRANSCRIPT_CACHE_DIR="$test_dir/preflight-cache" \
    "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"
}

run_credential_preflight \
  "$test_dir/youtube-api-key.txt" \
  "$test_dir/yt-dlp-cookies.txt" \
  "$test_dir/preflight.ok" > "$test_dir/preflight.out"
test -f "$test_dir/preflight.ok"
grep -Fx 'cloud-queue-indexer: credential preflight passed generation=test-generation' "$test_dir/preflight.out" >/dev/null

: > "$test_dir/missing-asr.ok"
if TRANSCRIBE_MISSING=1 run_credential_preflight \
  "$test_dir/youtube-api-key.txt" "$test_dir/yt-dlp-cookies.txt" \
  "$test_dir/missing-asr.ok" >/dev/null 2>&1; then
  echo 'enabled missing transcription runtime must fail preflight' >&2
  exit 1
fi
test ! -e "$test_dir/missing-asr.ok"

: > "$test_dir/incompatible-reader.ok"
if FAKE_RAW_BACKEND_COMPATIBLE=0 run_credential_preflight \
  "$test_dir/youtube-api-key.txt" \
  "$test_dir/yt-dlp-cookies.txt" \
  "$test_dir/incompatible-reader.ok" >/dev/null 2>&1; then
  echo "credential preflight must reject an incompatible Raw reader" >&2
  exit 1
fi
test ! -e "$test_dir/incompatible-reader.ok"

printf '%s\n' '#!/usr/bin/env sh' 'echo not-a-key' > "$test_dir/invalid-api-key.txt"
if run_credential_preflight \
  "$test_dir/invalid-api-key.txt" \
  "$test_dir/yt-dlp-cookies.txt" \
  "$test_dir/invalid-key.ok" >/dev/null 2>&1; then
  echo "credential preflight must reject a non-key file" >&2
  exit 1
fi
test ! -e "$test_dir/invalid-key.ok"

printf '# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t1\tSAPISID\texpired-value\n' \
  > "$test_dir/expired-cookies.txt"
if run_credential_preflight \
  "$test_dir/youtube-api-key.txt" \
  "$test_dir/expired-cookies.txt" \
  "$test_dir/expired-cookie.ok" >/dev/null 2>&1; then
  echo "credential preflight must reject expired auth cookies" >&2
  exit 1
fi
test ! -e "$test_dir/expired-cookie.ok"

printf '# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t4102444800\tPREF\tbenign-value\n' \
  > "$test_dir/non-auth-cookies.txt"
if run_credential_preflight \
  "$test_dir/youtube-api-key.txt" \
  "$test_dir/non-auth-cookies.txt" \
  "$test_dir/non-auth-cookie.ok" >/dev/null 2>&1; then
  echo "credential preflight must require an auth-bearing cookie" >&2
  exit 1
fi
test ! -e "$test_dir/non-auth-cookie.ok"

CLOUD_INDEX_QUEUE_LIBRARY_ONLY=1
INDEX_QUEUE_API_BASE_URL="https://queue.invalid"
YTSEARCH_INDEX_QUEUE_TOKEN="test-token"
TRANSCRIPT_CACHE_DIR="$test_dir/cache"
QUEUE_RETRY_STATE_FILE="$test_dir/retries.tsv"
RAW_PENDING_PUBLICATION_FILE="$test_dir/raw-publication.pending"
TRANSCRIPT_FAILURE_RETRY_SECONDS=21600
export CLOUD_INDEX_QUEUE_LIBRARY_ONLY INDEX_QUEUE_API_BASE_URL YTSEARCH_INDEX_QUEUE_TOKEN TRANSCRIPT_CACHE_DIR QUEUE_RETRY_STATE_FILE RAW_PENDING_PUBLICATION_FILE TRANSCRIPT_FAILURE_RETRY_SECONDS
. "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"

printf 'pending\n' > "$RAW_PENDING_PUBLICATION_FILE"
timeout 5 env \
  PATH="$test_dir/bin:$PATH" CLOUD_INDEX_QUEUE_LIBRARY_ONLY=0 \
  CLOUD_INDEX_QUEUE_PREFLIGHT_ONLY=0 QUEUE_LOOP=1 \
  IDLE_EXIT_AFTER_SECONDS=1 QUEUE_POLL_INTERVAL_SECONDS=1 \
  RAW_SEARCH_API_URL=http://raw.invalid:8108 RAW_SEARCH_ADMIN_API_KEY=test-admin-key \
  YOUTUBE_API_KEY_FILE="$test_dir/youtube-api-key.txt" \
  YT_DLP_COOKIES_FILE="$test_dir/yt-dlp-cookies.txt" \
  CREDENTIAL_PREFLIGHT_MARKER="$test_dir/daemon-preflight.ok" \
  CREDENTIAL_PREFLIGHT_GENERATION=test-generation \
  "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh" > "$test_dir/daemon.out" 2>&1
grep -q 'pending Raw publication failed' "$test_dir/daemon.out"
grep -q 'idle timeout reached' "$test_dir/daemon.out"
test "$(cat "$RAW_PENDING_PUBLICATION_FILE")" = pending
rm "$RAW_PENDING_PUBLICATION_FILE"

log_queue_event() { :; }
set_index_activity() { :; }
publication_log="$test_dir/publications.log"
publish_raw_generation() {
  printf 'publish\n' >> "$publication_log"
}

queue_request() { return 1; }
if pull_queue_tsv > "$test_dir/pull.tsv" 2> "$test_dir/pull.err"; then
  echo "queue transport failure must fail the pull" >&2
  exit 1
fi
grep -F "queue request failed" "$test_dir/pull.err" >/dev/null

queue_request() { printf 'not-json\n'; }
if pull_queue_tsv > "$test_dir/invalid.tsv" 2> "$test_dir/invalid.err"; then
  echo "malformed queue JSON must fail the pull" >&2
  exit 1
fi
grep -F "invalid queue response" "$test_dir/invalid.err" >/dev/null

if process_queue_once > "$test_dir/process-pull.out" 2> "$test_dir/process-pull.err"; then
  echo "the queue pass must propagate a pull failure" >&2
  exit 1
fi
grep -F "queue pull failed" "$test_dir/process-pull.err" >/dev/null
if grep -F "queue is empty" "$test_dir/process-pull.out" >/dev/null; then
  echo "a failed pull must not masquerade as an empty queue" >&2
  exit 1
fi

safe_pending_file="$RAW_PENDING_PUBLICATION_FILE"
RAW_PENDING_PUBLICATION_FILE="/proc/ytsearch-raw-publication.pending"
guard_log="$test_dir/marker-guard.log"
: > "$guard_log"
pull_queue_tsv() { printf 'request-marker\t@example\tworker\traw\n'; }
set_queue_status() { printf 'status %s %s\n' "$1" "$2" >> "$guard_log"; }
index_youtuber_for_queue_target() { printf 'index\n' >> "$guard_log"; }
publish_raw_generation() { printf 'publish\n' >> "$guard_log"; }
queue_request() { printf 'ack\n' >> "$guard_log"; }
if process_queue_once > "$test_dir/marker.out" 2> "$test_dir/marker.err"; then
  echo "an unwritable publication marker must fail the queue pass" >&2
  exit 1
fi
grep -Fx 'status request-marker running' "$guard_log" >/dev/null
grep -Fx 'status request-marker queued' "$guard_log" >/dev/null
if grep -Eq '^(index|publish|ack)$' "$guard_log"; then
  echo "marker failure must prevent mutation, publication, and acknowledgement" >&2
  exit 1
fi
RAW_PENDING_PUBLICATION_FILE="$safe_pending_file"
: > "$QUEUE_RETRY_STATE_FILE"

status_log="$test_dir/status.log"
pull_queue_tsv() {
  printf 'request-1\t@example\tworker\traw\n'
}
set_queue_status() {
  printf '%s %s\n' "$1" "$2" >> "$status_log"
}
queue_request() {
  echo "unexpected queue acknowledgement" >&2
  return 1
}
index_youtuber_for_queue_target() { return 75; }

if process_queue_once > "$test_dir/retry.out" 2> "$test_dir/retry.err"; then
  echo "retryable indexing gaps must fail the pass" >&2
  exit 1
fi
[ -f "$status_log" ] || {
  cat "$test_dir/retry.out" "$test_dir/retry.err" >&2
  echo "retryable request did not reach status transitions" >&2
  exit 1
}
grep -Fx "request-1 running" "$status_log" >/dev/null
grep -Fx "request-1 queued" "$status_log" >/dev/null
retry_at="$(awk -F '\t' '$1 == "request-1" { print $2 }' "$QUEUE_RETRY_STATE_FILE")"
now="$(date +%s)"
[ "$retry_at" -ge $(( now + 21590 )) ] || {
  echo "retryable request did not receive the transcript cooldown" >&2
  exit 1
}
cooldown_pull="$({
  . "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"
  queue_request() {
    printf '{"requests":[{"id":"request-1","handle":"@example","status":"queued","target_backend":"raw"}]}'
  }
  pull_queue_tsv
})"
if [ -n "$cooldown_pull" ]; then
  echo "a request inside its retry cooldown must not be pulled" >&2
  exit 1
fi

pull_queue_tsv() {
  printf 'request-1\t@example\tworker\traw\n'
}

: > "$status_log"
index_youtuber_for_queue_target() { return 1; }
process_queue_once > "$test_dir/terminal.out" 2> "$test_dir/terminal.err"
grep -Fx "request-1 running" "$status_log" >/dev/null
grep -Fx "request-1 failed" "$status_log" >/dev/null
if [ -s "$QUEUE_RETRY_STATE_FILE" ]; then
  echo "terminal requests must not retain a retry deadline" >&2
  exit 1
fi

: > "$status_log"
set_queue_status() {
  printf '%s %s\n' "$1" "$2" >> "$status_log"
  [ "$2" != "failed" ]
}
if process_queue_once > "$test_dir/status-failure.out" 2> "$test_dir/status-failure.err"; then
  echo "a terminal status transport failure must fail the pass" >&2
  exit 1
fi
grep -Fx "request-1 failed" "$status_log" >/dev/null
test -s "$QUEUE_RETRY_STATE_FILE"

set_queue_status() {
  printf '%s %s\n' "$1" "$2" >> "$status_log"
}

: > "$status_log"
index_youtuber_for_queue_target() { return 0; }
queue_request() { return 0; }
process_queue_once > "$test_dir/recovered.out" 2> "$test_dir/recovered.err"
grep -F "acked request=request-1" "$test_dir/recovered.out" >/dev/null

: > "$status_log"
queue_request() { return 1; }
if process_queue_once > "$test_dir/ack.out" 2> "$test_dir/ack.err"; then
  echo "an acknowledgement failure must fail the pass" >&2
  exit 1
fi
grep -Fx "request-1 running" "$status_log" >/dev/null
grep -Fx "request-1 queued" "$status_log" >/dev/null
if grep -F "acked request=" "$test_dir/ack.out" >/dev/null; then
  echo "an unacknowledged request must not be reported as complete" >&2
  exit 1
fi

: > "$status_log"
: > "$publication_log"
publish_raw_generation() {
  printf 'failed\n' >> "$publication_log"
  return 1
}
queue_request() {
  echo "a request must not be acknowledged before publication" >&2
  return 1
}
if process_queue_once > "$test_dir/publication-failure.out" 2> "$test_dir/publication-failure.err"; then
  echo "a publication failure must fail the queue pass" >&2
  exit 1
fi
grep -Fx "request-1 queued" "$status_log" >/dev/null
if grep -F "acked request=" "$test_dir/publication-failure.out" >/dev/null; then
  echo "a request was acknowledged before publication" >&2
  exit 1
fi
[ -f "$RAW_PENDING_PUBLICATION_FILE" ] || {
  echo "publication failure must retain a durable retry marker" >&2
  exit 1
}

: > "$status_log"
: > "$publication_log"
: > "$QUEUE_RETRY_STATE_FILE"
pull_queue_tsv() {
  printf 'request-1\t@example\tworker\traw\n'
  printf 'request-2\t@example-two\tworker\traw\n'
}
publish_raw_generation() {
  printf 'publish\n' >> "$publication_log"
}
queue_request() { return 0; }
process_queue_once > "$test_dir/coalesced.out" 2> "$test_dir/coalesced.err"
[ ! -f "$RAW_PENDING_PUBLICATION_FILE" ] || {
  echo "successful publication must clear the durable retry marker" >&2
  exit 1
}
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "two successful queue requests must share one publication" >&2
  exit 1
}
[ "$(grep -c 'acked request=' "$test_dir/coalesced.out")" -eq 2 ] || {
  echo "both coalesced requests must be acknowledged after publication" >&2
  exit 1
}

# An upload notification is acked once indexed and leaves a routine marker
# instead of rebuilding the generation for every upload.
: > "$publication_log"
rm -f "$RAW_PENDING_PUBLICATION_FILE"
pull_queue_tsv() { printf 'upload-1-raw-example\t@example\tworker\traw\n'; }
process_queue_once > "$test_dir/upload.out" 2> "$test_dir/upload.err"
grep -F "acked request=upload-1-raw-example" "$test_dir/upload.out" >/dev/null || {
  echo "an indexed upload notification must be acknowledged at once" >&2
  exit 1
}
[ ! -s "$publication_log" ] || {
  echo "an upload notification must not publish a generation by itself" >&2
  exit 1
}
grep -q ' routine$' "$RAW_PENDING_PUBLICATION_FILE" || {
  echo "an upload notification must leave a routine publication marker" >&2
  exit 1
}
# Alongside an approved request, both share that request's publication.
pull_queue_tsv() {
  printf 'upload-2-raw-example\t@example\tworker\traw\n'
  printf 'request-3\t@example-two\tworker\traw\n'
}
process_queue_once > "$test_dir/upload-mixed.out" 2> "$test_dir/upload-mixed.err"
[ "$(wc -l < "$publication_log" | tr -d ' ')" -eq 1 ] || {
  echo "an approved request must still publish at once beside an upload" >&2
  exit 1
}
[ "$(grep -c 'acked request=' "$test_dir/upload-mixed.out")" -eq 2 ] || {
  echo "both the upload and the approved request must be acknowledged" >&2
  exit 1
}
[ ! -f "$RAW_PENDING_PUBLICATION_FILE" ] || {
  echo "the shared publication must clear the marker" >&2
  exit 1
}

# A daily catalog pass whose only failures are retryable per-video gaps
# (exit 75) completes and publishes; any other catalog failure fails it.
daily_list="$test_dir/daily-creators.txt"
printf '@example\n' > "$daily_list"
DAILY_UPDATE_LIST_FILE="$daily_list"
mkdir -p "$test_dir/api"
printf '%s\n' '{"channels":[{"handle":"@example"}]}' > "$test_dir/api/youtubers"
INDEX_QUEUE_API_BASE_URL="file://$test_dir"
RAW_SEARCH_API_URL=http://raw.invalid:8108
RAW_SEARCH_ADMIN_API_KEY=test-admin-key
BACKENDS=raw
daily_events="$test_dir/daily-events.log"
log_queue_event() { printf '%s\n' "$2" >> "$daily_events"; }
publication_log="$test_dir/daily-publications.log"
publish_raw_generation() { printf 'publish\n' >> "$publication_log"; }
for stub_status in 75 0; do
  : > "$daily_events"; : > "$publication_log"
  index_catalog_backend() { return "$stub_status"; }
  if ! index_daily_update_for_backends > /dev/null 2>&1; then
    echo "a daily pass with catalog status $stub_status must complete" >&2
    exit 1
  fi
  grep -Fx publish "$publication_log" >/dev/null || {
    echo "a completed daily pass must publish" >&2
    exit 1
  }
done
: > "$daily_events"
index_catalog_backend() { return 75; }
index_daily_update_for_backends > /dev/null 2>&1
grep -Fx daily_update_backend_gaps "$daily_events" >/dev/null || {
  echo "a gaps-only daily pass must be logged as completed with gaps" >&2
  exit 1
}
for stub_status in 1 69; do
  index_catalog_backend() { return "$stub_status"; }
  if index_daily_update_for_backends > /dev/null 2>&1; then
    echo "a daily pass with catalog status $stub_status must fail" >&2
    exit 1
  fi
done

# A bot challenge or rate limit (exit 69) keeps the queue request for retry.
: > "$QUEUE_RETRY_STATE_FILE"
blocked_status_log="$test_dir/blocked-status.log"
pull_queue_tsv() { printf 'request-blocked\t@example\tworker\traw\n'; }
set_queue_status() { printf '%s %s\n' "$1" "$2" >> "$blocked_status_log"; }
queue_request() { echo "unexpected queue acknowledgement" >&2; return 1; }
index_youtuber_for_queue_target() { return 69; }
process_queue_once > /dev/null 2>&1 || true
grep -Fx "request-blocked queued" "$blocked_status_log" >/dev/null || {
  echo "an upstream-blocked request must return to the queue" >&2
  exit 1
}


# As root, ytsearch runs as the unprivileged indexer user with a private
# cookie copy; fakes stand in for root-only commands.
mkdir -p "$test_dir/root-bin"
cat > "$test_dir/root-bin/id" <<'SH'
#!/usr/bin/env sh
case "$*" in
  -u) echo 0 ;;
  "-u ytsearch") echo 10002 ;;
  ytsearch) exit 0 ;;
  *) exit 1 ;;
esac
SH
cat > "$test_dir/root-bin/gosu" <<'SH'
#!/usr/bin/env sh
printf '%s\n' "$*" >> "$FAKE_PRIVILEGE_LOG"
shift
exec "$@"
SH
cat > "$test_dir/root-bin/chown" <<'SH'
#!/usr/bin/env sh
printf 'chown %s\n' "$*" >> "$FAKE_PRIVILEGE_LOG"
SH
chmod +x "$test_dir/root-bin/id" "$test_dir/root-bin/gosu" "$test_dir/root-bin/chown"
: > "$test_dir/privilege.log"
(
  PATH="$test_dir/root-bin:$test_dir/bin:$PATH"
  FAKE_PRIVILEGE_LOG="$test_dir/privilege.log"
  TRANSCRIPT_CACHE_DIR="$test_dir/volume/transcript-cache"
  RAW_PUBLICATION_WORK_DIRECTORY="$test_dir/volume/work"
  YT_DLP_COOKIES_FILE="$test_dir/yt-dlp-cookies.txt"
  export FAKE_PRIVILEGE_LOG RAW_PUBLICATION_WORK_DIRECTORY
  prepare_unprivileged_indexer > "$test_dir/privilege.out"
  test "$YT_DLP_COOKIES_FILE" = "$test_dir/volume/indexer-home/yt-dlp-cookies.txt"
  cmp -s "$YT_DLP_COOKIES_FILE" "$test_dir/yt-dlp-cookies.txt"
  test "$(stat -c %a "$YT_DLP_COOKIES_FILE")" = 600
  run_indexer ytsearch init-backend --backend raw > /dev/null
)
grep -Fq 'running ytsearch as ytsearch' "$test_dir/privilege.out"
grep -Fxq "ytsearch env HOME=$test_dir/volume/indexer-home ytsearch init-backend --backend raw" \
  "$test_dir/privilege.log"
grep -Fq "chown ytsearch:ytsearch $test_dir/volume/indexer-home/yt-dlp-cookies.txt" \
  "$test_dir/privilege.log"
for directory in transcript-cache work; do
  grep -Fq "$test_dir/volume/$directory" "$test_dir/privilege.log"
done
echo "cloud queue transport and retry behavior passed"
