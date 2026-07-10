#!/usr/bin/env sh
set -eu

BACKENDS="${BACKENDS:-meilisearch typesense-fly}"
QUEUE_TARGET_BACKEND="${QUEUE_TARGET_BACKEND:-}"
JOBS="${JOBS:-1}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"
VIDEO_TIMEOUT_SECONDS="${VIDEO_TIMEOUT_SECONDS:-300}"
TRANSCRIPT_CACHE_DIR="${TRANSCRIPT_CACHE_DIR:-/data/transcript-cache}"
LIMIT="${LIMIT:-10}"
QUEUE_ONE_REQUEST_PER_PASS="${QUEUE_ONE_REQUEST_PER_PASS:-0}"
QUEUE_PRIORITY_LIST_FILE="${QUEUE_PRIORITY_LIST_FILE:-$DAILY_UPDATE_LIST_FILE}"
MAX_VIDEOS_PER_PASS="${MAX_VIDEOS_PER_PASS:-0}"
MEILI_WAIT_FOR_TASKS="${MEILI_WAIT_FOR_TASKS:-1}"
QUEUE_POLL_INTERVAL_SECONDS="${QUEUE_POLL_INTERVAL_SECONDS:-60}"
QUEUE_LOOP="${QUEUE_LOOP:-0}"
DAILY_UPDATE_ENABLED="${DAILY_UPDATE_ENABLED:-0}"
DAILY_UPDATE_INTERVAL_SECONDS="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}"
DAILY_UPDATE_INITIAL_DELAY_SECONDS="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-86400}"
DAILY_UPDATE_LIST_FILE="${DAILY_UPDATE_LIST_FILE:-/tmp/daily-indexed-channels.txt}"
EXISTING_TAIL="${EXISTING_TAIL:-0}"
ACK_FAILED_REQUESTS="${ACK_FAILED_REQUESTS:-1}"

export MEILI_WAIT_FOR_TASKS
mkdir -p "$TRANSCRIPT_CACHE_DIR"

require_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "cloud-queue-indexer: missing required environment variable: $name" >&2
    exit 1
  fi
}

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

default_queue_target_backend() {
  if [ -n "$QUEUE_TARGET_BACKEND" ]; then
    printf '%s\n' "$QUEUE_TARGET_BACKEND"
    return 0
  fi
  set -- $BACKENDS
  if [ "$#" -eq 1 ]; then
    printf '%s\n' "$1"
  fi
}

validate_backend_scope() {
  target_backend="$(default_queue_target_backend)"
  case "$target_backend" in
    meilisearch|typesense-fly) ;;
    *)
      echo "cloud-queue-indexer: QUEUE_TARGET_BACKEND must select exactly one active backend" >&2
      exit 1
      ;;
  esac

  if [ "$BACKENDS" != "$target_backend" ]; then
    echo "cloud-queue-indexer: BACKENDS must equal QUEUE_TARGET_BACKEND ($target_backend), got: $BACKENDS" >&2
    exit 1
  fi
}

log_queue_event() {
  level="$1"
  event="$2"
  request_id="${3:-}"
  handle="${4:-}"
  source_backend="${5:-}"
  backends="${6:-}"
  status="${7:-}"

  python3 - "$level" "$event" "$request_id" "$handle" "$source_backend" "$backends" "$status" <<'PY'
import json
import os
import sys
import time
import urllib.error
import urllib.request

level, event, request_id, handle, source_backend, backends, status = sys.argv[1:]
payload = {
    "ts": int(time.time()),
    "component": "cloud-queue-indexer",
    "level": level,
    "event": event,
}
if request_id:
    payload["request_id"] = request_id
if handle:
    payload["handle"] = handle
if source_backend:
    payload["source_backend"] = source_backend
if backends:
    payload["backends"] = backends.split()
if status:
    payload["status"] = status
print(json.dumps(payload, separators=(",", ":")), file=sys.stderr)

token = os.environ.get("AXIOM_TOKEN") or os.environ.get("AXIOM_API_KEY")
dataset = os.environ.get("AXIOM_DATASET")
if token and dataset:
    base_url = (
        os.environ.get("AXIOM_URL")
        or os.environ.get("AXIOM_EDGE_URL")
        or os.environ.get("AXIOM_DOMAIN")
        or "https://api.axiom.co"
    )
    if not base_url.startswith(("http://", "https://")):
        base_url = "https://" + base_url
    url = f"{base_url.rstrip('/')}/v1/datasets/{dataset}/ingest"
    request = urllib.request.Request(
        url,
        data=json.dumps([payload]).encode(),
        method="POST",
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=10):
            pass
    except (OSError, urllib.error.HTTPError, urllib.error.URLError) as error:
        print(f"cloud-queue-indexer: axiom ingest failed: {error}", file=sys.stderr)
PY
}

queue_request() {
  method="$1"
  path="$2"
  python3 - "$method" "$path" <<'PY'
import os
import sys
import urllib.request

method, path = sys.argv[1], sys.argv[2]
base = os.environ["INDEX_QUEUE_API_BASE_URL"].rstrip("/")
token = os.environ["YTSEARCH_INDEX_QUEUE_TOKEN"]
request = urllib.request.Request(
    base + path,
    method=method,
    headers={"Authorization": f"Bearer {token}"},
)
with urllib.request.urlopen(request, timeout=30) as response:
    sys.stdout.write(response.read().decode())
PY
}

pull_queue_tsv() {
  queue_body="$(queue_request GET "/api/index-requests?limit=1000" 2>/dev/null || true)"
  QUEUE_BODY="$queue_body" QUEUE_PULL_LIMIT="$LIMIT" QUEUE_PRIORITY_LIST_FILE="$QUEUE_PRIORITY_LIST_FILE" QUEUE_TARGET_BACKEND="$(default_queue_target_backend)" \
    python3 <<'PY'
import json
import os
import sys

raw = os.environ.get("QUEUE_BODY", "").strip()
if not raw:
    sys.exit(0)
try:
    body = json.loads(raw)
except json.JSONDecodeError as error:
    print(f"cloud-queue-indexer: invalid queue response: {error}", file=sys.stderr)
    sys.exit(0)
priority_file = os.environ.get("QUEUE_PRIORITY_LIST_FILE", "")
target_backend = os.environ.get("QUEUE_TARGET_BACKEND", "").strip()
priority_handles = set()
if priority_file and os.path.isfile(priority_file):
    with open(priority_file, encoding="utf-8") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            handle_value = line.split("|", 1)[0].strip().lower()
            if not handle_value:
                continue
            if not handle_value.startswith("@"):
                handle_value = "@" + handle_value
            priority_handles.add(handle_value)

requests = []
for request in body.get("requests", []):
    if request.get("status") != "queued":
        continue
    request_id = request.get("id", "")
    handle = request.get("handle", "")
    source_backend = request.get("source_backend", "")
    request_target_backend = request.get("target_backend") or ""
    if target_backend and request_target_backend != target_backend:
        continue
    if request_id and handle:
        requests.append(
            {
                "id": request_id,
                "handle": handle,
                "source_backend": source_backend,
                "target_backend": request_target_backend,
                "requested_at": int(request.get("requested_at") or 0),
            }
        )

def normalize(handle: str) -> str:
    value = handle.strip().lower()
    return value if value.startswith("@") else "@" + value

requests.sort(
    key=lambda request: (
        0 if normalize(request["handle"]) in priority_handles else 1,
        request["requested_at"],
    )
)

pull_limit = int(os.environ.get("QUEUE_PULL_LIMIT", "10"))
for request in requests[:pull_limit]:
    print(
        "\t".join(
            [
                request["id"],
                request["handle"],
                request.get("source_backend", ""),
                request.get("target_backend", ""),
            ]
        )
    )
PY
}

index_one_backend() {
  label="$1"
  backend="$2"
  jobs="$3"
  handle="$4"
  shift 4

  extra_args=""
  if [ -n "${MAX_VIDEOS_PER_PASS:-}" ] && [ "$MAX_VIDEOS_PER_PASS" -gt 0 ] 2>/dev/null; then
    extra_args="--max-videos $MAX_VIDEOS_PER_PASS"
  fi

  echo "cloud-queue-indexer: indexing $handle backend=$label jobs=$jobs"
  # shellcheck disable=SC2086
  if ! ytsearch index-youtuber "$handle" \
    --backend "$backend" \
    --mode update \
    --collection "$COLLECTION" \
    --jobs "$jobs" \
    --video-timeout-seconds "$VIDEO_TIMEOUT_SECONDS" \
    --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
    $extra_args \
    "$@"; then
    return 1
  fi

  echo "cloud-queue-indexer: refreshing $handle backend=$label"
  if ! ytsearch refresh-youtuber-counts "$handle" \
    --backend "$backend" \
    --collection "$COLLECTION"; then
    echo "cloud-queue-indexer: refresh failed handle=$handle backend=$label (indexed data kept)" >&2
  fi
}

index_youtuber_for_backends() {
  handle="$1"
  failed=0
  typesense_jobs="${TYPESENSE_JOBS:-$JOBS}"
  meili_jobs="${MEILI_JOBS:-$JOBS}"

  # Typesense first so long Meili jobs do not starve the Typesense frontend counts.
  if has_backend typesense-fly; then
    if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
      echo "cloud-queue-indexer: skipping typesense-fly; missing TYPESENSE_SELFHOST_API_URL or TYPESENSE_SELFHOST_API_KEY" >&2
    else
      export TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL"
      export TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
      export TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
      if ! index_one_backend typesense-fly typesense "$typesense_jobs" "$handle"; then
        failed=1
        echo "cloud-queue-indexer: backend failed handle=$handle backend=typesense-fly" >&2
      fi
    fi
  fi

  if has_backend meilisearch; then
    if ! index_one_backend meilisearch meilisearch "$meili_jobs" "$handle"; then
      failed=1
      echo "cloud-queue-indexer: backend failed handle=$handle backend=meilisearch" >&2
    fi
  fi

  return "$failed"
}

index_youtuber_for_queue_target() {
  handle="$1"
  target_backend="$2"

  if [ -z "$target_backend" ]; then
    index_youtuber_for_backends "$handle"
    return $?
  fi

  case "$target_backend" in
    meilisearch)
      BACKENDS="meilisearch" index_youtuber_for_backends "$handle"
      ;;
    typesense-fly)
      BACKENDS="typesense-fly" index_youtuber_for_backends "$handle"
      ;;
    *)
      echo "cloud-queue-indexer: unsupported target_backend=$target_backend request skipped" >&2
      return 1
      ;;
  esac
}

index_list_backend() {
  label="$1"
  backend="$2"
  jobs="$3"
  shift 3

  echo "cloud-queue-indexer: daily indexing backend=$label jobs=$jobs list=$DAILY_UPDATE_LIST_FILE"
  ytsearch index-youtubers "$DAILY_UPDATE_LIST_FILE" \
    --backend "$backend" \
    --mode update \
    --collection "$COLLECTION" \
    --jobs "$jobs" \
    --video-timeout-seconds "$VIDEO_TIMEOUT_SECONDS" \
    --stop-after-existing "$EXISTING_TAIL" \
    --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
    "$@"

  echo "cloud-queue-indexer: daily refreshing backend=$label"
  if ! ytsearch refresh-youtuber-counts-list "$DAILY_UPDATE_LIST_FILE" \
    --backend "$backend" \
    --collection "$COLLECTION"; then
    echo "cloud-queue-indexer: daily refresh failed backend=$label (indexed data kept)" >&2
  fi
}

index_daily_update_for_backends() {
  if [ ! -f "$DAILY_UPDATE_LIST_FILE" ]; then
    echo "cloud-queue-indexer: daily update skipped; list file not found: $DAILY_UPDATE_LIST_FILE" >&2
    return 0
  fi

  failed=0
  typesense_jobs="${TYPESENSE_JOBS:-$JOBS}"
  meili_jobs="${MEILI_JOBS:-$JOBS}"

  if has_backend typesense-fly; then
    if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
      echo "cloud-queue-indexer: skipping daily typesense-fly; missing TYPESENSE_SELFHOST_API_URL or TYPESENSE_SELFHOST_API_KEY" >&2
    else
      export TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL"
      export TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
      export TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
      if ! index_list_backend typesense-fly typesense "$typesense_jobs"; then
        failed=1
        log_queue_event error daily_update_backend_failed "" "" "" "typesense-fly" "failed"
      fi
    fi
  fi

  if has_backend meilisearch; then
    if ! index_list_backend meilisearch meilisearch "$meili_jobs"; then
      failed=1
      log_queue_event error daily_update_backend_failed "" "" "" "meilisearch" "failed"
    fi
  fi

  return "$failed"
}

require_env INDEX_QUEUE_API_BASE_URL
require_env YTSEARCH_INDEX_QUEUE_TOKEN
validate_backend_scope

process_queue_once() {
  processed=0
  failed=0
  failed_acked=0
  tmp_queue="/tmp/ytsearch-queue.tsv"
  pull_queue_tsv > "$tmp_queue"

  if [ ! -s "$tmp_queue" ]; then
    echo "cloud-queue-indexer: queue is empty"
    log_queue_event info queue_empty "" "" "" "$BACKENDS" "idle"
    return 0
  fi

  queue_depth="$(wc -l < "$tmp_queue" | tr -d ' ')"
  log_queue_event info queue_poll "" "" "" "$BACKENDS" "depth=$queue_depth"

  while IFS="$(printf '\t')" read -r request_id handle source_backend target_backend; do
    [ -n "$request_id" ] || continue
    if [ -n "$target_backend" ]; then
      active_backends="$target_backend"
    else
      active_backends="$BACKENDS"
    fi
    echo "cloud-queue-indexer: processing request=$request_id handle=$handle source_backend=$source_backend target_backend=$target_backend"
    log_queue_event info index_request_started "$request_id" "$handle" "$source_backend" "$active_backends" "running"
    if index_youtuber_for_queue_target "$handle" "$target_backend"; then
      queue_request DELETE "/api/index-requests/$request_id" >/dev/null
      echo "cloud-queue-indexer: acked request=$request_id handle=$handle"
      log_queue_event info index_request_completed "$request_id" "$handle" "$source_backend" "$active_backends" "acked"
      processed=$((processed + 1))
    else
      failed=$((failed + 1))
      log_queue_event error index_request_failed "$request_id" "$handle" "$source_backend" "$active_backends" "failed"
      if [ "$ACK_FAILED_REQUESTS" = "1" ]; then
        if queue_request DELETE "/api/index-requests/$request_id" >/dev/null; then
          failed_acked=$((failed_acked + 1))
          log_queue_event warn index_request_failed_acked "$request_id" "$handle" "$source_backend" "$active_backends" "deleted"
        else
          log_queue_event error index_request_failed_ack_error "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
        fi
      else
        log_queue_event warn index_request_failed_left_queued "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
      fi
    fi
    if [ "$QUEUE_ONE_REQUEST_PER_PASS" = "1" ]; then
      echo "cloud-queue-indexer: one-request-per-pass enabled; deferring remaining queue items"
      break
    fi
  done < "$tmp_queue"

  echo "cloud-queue-indexer: pass complete processed=$processed failed=$failed failed_acked=$failed_acked"
}

if [ "$QUEUE_LOOP" = "1" ]; then
  echo "cloud-queue-indexer: daemon mode enabled poll_interval=${QUEUE_POLL_INTERVAL_SECONDS}s"
  next_daily_update=$(( $(date +%s) + DAILY_UPDATE_INITIAL_DELAY_SECONDS ))
  if [ "$DAILY_UPDATE_ENABLED" = "1" ]; then
    echo "cloud-queue-indexer: daily update enabled initial_delay=${DAILY_UPDATE_INITIAL_DELAY_SECONDS}s interval=${DAILY_UPDATE_INTERVAL_SECONDS}s next_after_epoch=$next_daily_update"
  else
    echo "cloud-queue-indexer: daily update disabled"
  fi
  while :; do
    if [ "$DAILY_UPDATE_ENABLED" = "1" ]; then
      now="$(date +%s)"
      if [ "$now" -ge "$next_daily_update" ]; then
        echo "cloud-queue-indexer: running scheduled daily update before queue pass"
        if index_daily_update_for_backends; then
          next_daily_update=$(( now + DAILY_UPDATE_INTERVAL_SECONDS ))
          echo "cloud-queue-indexer: next daily update after epoch=$next_daily_update"
        else
          next_daily_update=$(( now + QUEUE_POLL_INTERVAL_SECONDS ))
          echo "cloud-queue-indexer: daily update failed; retry_after=${QUEUE_POLL_INTERVAL_SECONDS}s" >&2
        fi
      fi
    fi
    process_queue_once || echo "cloud-queue-indexer: queue pass failed; continuing after poll interval" >&2
    sleep "$QUEUE_POLL_INTERVAL_SECONDS"
  done
fi

process_queue_once
