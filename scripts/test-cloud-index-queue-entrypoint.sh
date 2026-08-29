#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT

CLOUD_INDEX_QUEUE_LIBRARY_ONLY=1
INDEX_QUEUE_API_BASE_URL="https://queue.invalid"
YTSEARCH_INDEX_QUEUE_TOKEN="test-token"
TRANSCRIPT_CACHE_DIR="$test_dir/cache"
QUEUE_RETRY_STATE_FILE="$test_dir/retries.tsv"
RAW_PENDING_PUBLICATION_FILE="$test_dir/raw-publication.pending"
TRANSCRIPT_FAILURE_RETRY_SECONDS=21600
export CLOUD_INDEX_QUEUE_LIBRARY_ONLY INDEX_QUEUE_API_BASE_URL YTSEARCH_INDEX_QUEUE_TOKEN TRANSCRIPT_CACHE_DIR QUEUE_RETRY_STATE_FILE RAW_PENDING_PUBLICATION_FILE TRANSCRIPT_FAILURE_RETRY_SECONDS
. "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"

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

echo "cloud queue transport and retry behavior passed"
