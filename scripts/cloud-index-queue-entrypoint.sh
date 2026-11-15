#!/usr/bin/env sh
set -eu

BACKENDS="${BACKENDS:-raw}"
QUEUE_TARGET_BACKEND="${QUEUE_TARGET_BACKEND:-}"
JOBS="${JOBS:-1}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"
VIDEO_TIMEOUT_SECONDS="${VIDEO_TIMEOUT_SECONDS:-600}"
TRANSCRIPT_CACHE_DIR="${TRANSCRIPT_CACHE_DIR:-/data/transcript-cache}"
TRANSCRIBE_MISSING="${TRANSCRIBE_MISSING:-0}"
TRANSCRIPTION_CACHE_DIR="${TRANSCRIPTION_CACHE_DIR:-$TRANSCRIPT_CACHE_DIR/.transcription}"
WHISPER_BINARY="${WHISPER_BINARY:-/usr/local/bin/whisper-cli}"
WHISPER_MODEL="${WHISPER_MODEL:-/usr/local/share/ytsearch-asr/ggml-base.en.bin}"
WHISPER_THREADS="${WHISPER_THREADS:-2}"
TRANSCRIPTION_BUDGET_SECONDS="${TRANSCRIPTION_BUDGET_SECONDS:-480}"
TRANSCRIPT_FAILURE_RETRY_SECONDS="${TRANSCRIPT_FAILURE_RETRY_SECONDS:-21600}"
QUEUE_RETRY_STATE_FILE="${QUEUE_RETRY_STATE_FILE:-/data/queue-retries.tsv}"
QUEUE_ACK_RETRY_SECONDS="${QUEUE_ACK_RETRY_SECONDS:-300}"
LIMIT="${LIMIT:-10}"
QUEUE_ONE_REQUEST_PER_PASS="${QUEUE_ONE_REQUEST_PER_PASS:-0}"
DAILY_UPDATE_LIST_FILE="${DAILY_UPDATE_LIST_FILE:-/tmp/daily-indexed-channels.txt}"
DAILY_UPDATE_STATE_FILE="${DAILY_UPDATE_STATE_FILE:-/data/daily-update.last-completed}"
QUEUE_PRIORITY_LIST_FILE="${QUEUE_PRIORITY_LIST_FILE:-$DAILY_UPDATE_LIST_FILE}"
MAX_VIDEOS_PER_PASS="${MAX_VIDEOS_PER_PASS:-0}"
QUEUE_POLL_INTERVAL_SECONDS="${QUEUE_POLL_INTERVAL_SECONDS:-60}"
QUEUE_LOOP="${QUEUE_LOOP:-0}"
DAILY_UPDATE_ENABLED="${DAILY_UPDATE_ENABLED:-0}"
DAILY_UPDATE_INTERVAL_SECONDS="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}"
# A failed daily pass is retried after this delay, doubling per consecutive
# failure up to the maximum. The retry time is kept on the volume, so a worker
# that exits and wakes again does not rerun a failing pass every hour.
DAILY_UPDATE_FAILURE_RETRY_SECONDS="${DAILY_UPDATE_FAILURE_RETRY_SECONDS:-3600}"
DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS="${DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS:-21600}"
DAILY_UPDATE_RETRY_FILE="${DAILY_UPDATE_RETRY_FILE:-${DAILY_UPDATE_STATE_FILE}.retry}"
DAILY_UPDATE_INITIAL_DELAY_SECONDS="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-86400}"
EXISTING_TAIL="${EXISTING_TAIL:-0}"
ACK_FAILED_REQUESTS="${ACK_FAILED_REQUESTS:-0}"
IDLE_EXIT_AFTER_SECONDS="${IDLE_EXIT_AFTER_SECONDS:-0}"
IDLE_GAP_FILL_ENABLED="${IDLE_GAP_FILL_ENABLED:-0}"
IDLE_GAP_FILL_STATE_FILE="${IDLE_GAP_FILL_STATE_FILE:-/data/idle-gap-fill.cursor}"
IDLE_GAP_FILL_MAX_PASSES="${IDLE_GAP_FILL_MAX_PASSES:-1}"
IDLE_GAP_FILL_BATCH_PASSES="${IDLE_GAP_FILL_BATCH_PASSES:-4}"
IDLE_GAP_FILL_MAX_NEW_VIDEOS="${IDLE_GAP_FILL_MAX_NEW_VIDEOS:-10}"
RAW_PENDING_PUBLICATION_FILE="${RAW_PENDING_PUBLICATION_FILE:-/data/raw-publication.pending}"
# A publication rebuilds the whole generation (about an hour on the worker), so
# routine gap repair runs and publishes at most once per interval; approved
# requests and the daily pass still publish at once. 0 keeps every batch.
IDLE_GAP_FILL_INTERVAL_SECONDS="${IDLE_GAP_FILL_INTERVAL_SECONDS:-0}"
ROUTINE_PUBLICATION_INTERVAL_SECONDS="${ROUTINE_PUBLICATION_INTERVAL_SECONDS:-0}"
IDLE_GAP_FILL_STARTED_FILE="${IDLE_GAP_FILL_STARTED_FILE:-/data/idle-gap-fill.last-started}"
RAW_LAST_PUBLICATION_FILE="${RAW_LAST_PUBLICATION_FILE:-/data/raw-publication.last-completed}"
CREDENTIAL_PREFLIGHT_MARKER="${CREDENTIAL_PREFLIGHT_MARKER:-/run/ytsearch-credential-preflight.ok}"

mkdir -p "$TRANSCRIPT_CACHE_DIR"

require_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "cloud-queue-indexer: missing required environment variable: $name" >&2
    exit 1
  fi
}

validate_file_credentials() {
  rm -f "$CREDENTIAL_PREFLIGHT_MARKER"
  if [ ! -f "$YOUTUBE_API_KEY_FILE" ] || [ ! -r "$YOUTUBE_API_KEY_FILE" ]; then
    echo "cloud-queue-indexer: YouTube API key file is not readable" >&2
    exit 1
  fi
  youtube_api_key="$(tr -d '\r\n' < "$YOUTUBE_API_KEY_FILE")"
  if ! printf '%s' "$youtube_api_key" | grep -Eq '^AIza[A-Za-z0-9_-]{35}$'; then
    echo "cloud-queue-indexer: YouTube API key file has an invalid shape" >&2
    exit 1
  fi
  if [ ! -f "$YT_DLP_COOKIES_FILE" ] || [ ! -r "$YT_DLP_COOKIES_FILE" ]; then
    echo "cloud-queue-indexer: yt-dlp cookie file is not readable" >&2
    exit 1
  fi
  if ! grep -Eq '^# Netscape HTTP Cookie File' "$YT_DLP_COOKIES_FILE"; then
    echo "cloud-queue-indexer: yt-dlp cookie file is not Netscape format" >&2
    exit 1
  fi
  now="$(date +%s)"
  if ! awk -F '\t' -v now="$now" '
    NF == 7 {
      domain = $1
      sub(/^#HttpOnly_/, "", domain)
      expiry = $5
      name = $6
      value = $7
      valid_domain = domain ~ /(^|\.)youtube\.com$/ || domain ~ /(^|\.)google\.com$/
      valid_expiry = expiry == "0" || (expiry ~ /^[0-9]+$/ && expiry + 0 > now)
      valid_name = name ~ /^(SID|HSID|SSID|APISID|SAPISID|__Secure-1PAPISID|__Secure-3PAPISID|LOGIN_INFO)$/
      if (valid_domain && valid_expiry && valid_name && value != "") {
        found = 1
      }
    }
    END { exit(found ? 0 : 1) }
  ' "$YT_DLP_COOKIES_FILE"; then
    echo "cloud-queue-indexer: yt-dlp cookie file has no usable YouTube or Google auth cookie" >&2
    exit 1
  fi
  YOUTUBE_API_KEY="$youtube_api_key"
  export YOUTUBE_API_KEY
}

# As root, run ytsearch (and so yt-dlp, which parses untrusted YouTube data)
# as an unprivileged user; the entrypoint keeps root only for its own setup.
INDEXER_USER="${INDEXER_USER:-ytsearch}"
INDEXER_RUN_AS=""
prepare_unprivileged_indexer() {
  [ "$(id -u)" = 0 ] && id "$INDEXER_USER" >/dev/null 2>&1 && command -v gosu >/dev/null 2>&1 || return 0
  indexer_home="${INDEXER_HOME:-$(dirname "$TRANSCRIPT_CACHE_DIR")/indexer-home}"
  work_directory="${RAW_PUBLICATION_WORK_DIRECTORY:-/var/lib/ytsearch/raw-generation-work}"
  mkdir -p "$indexer_home" "$TRANSCRIPT_CACHE_DIR" "$work_directory"
  # Only files not yet owned are changed, so later starts are near-instant.
  indexer_uid="$(id -u "$INDEXER_USER")"
  for directory in "$indexer_home" "$TRANSCRIPT_CACHE_DIR" "$work_directory"; do
    find "$directory" ! -uid "$indexer_uid" -exec chown -h "$INDEXER_USER:$INDEXER_USER" {} +
  done
  # yt-dlp writes its cookie jar back, so it gets a private copy of the secret.
  cookies="$indexer_home/yt-dlp-cookies.txt"
  cp "$YT_DLP_COOKIES_FILE" "$cookies"
  chmod 600 "$cookies"
  chown "$INDEXER_USER:$INDEXER_USER" "$cookies"
  YT_DLP_COOKIES_FILE="$cookies"
  INDEXER_HOME="$indexer_home"
  export YT_DLP_COOKIES_FILE INDEXER_HOME
  INDEXER_RUN_AS="$INDEXER_USER"
  echo "cloud-queue-indexer: running ytsearch as $INDEXER_USER"
}

run_indexer() {
  if [ -n "$INDEXER_RUN_AS" ]; then
    gosu "$INDEXER_RUN_AS" env HOME="$INDEXER_HOME" "$@"
  else
    "$@"
  fi
}

# One argument boundary for approved requests, catalog refreshes and idle repair.
run_with_transcription() {
  if [ "$TRANSCRIBE_MISSING" = 1 ]; then
    run_indexer "$@" --transcribe-missing --transcription-backend whisper-cpp \
      --whisper-binary "$WHISPER_BINARY" --whisper-model "$WHISPER_MODEL" \
      --whisper-threads "$WHISPER_THREADS" \
      --transcription-cache-dir "$TRANSCRIPTION_CACHE_DIR" \
      --transcription-budget-seconds "$TRANSCRIPTION_BUDGET_SECONDS"
  else
    run_indexer "$@"
  fi
}

validate_transcription() {
  case "$TRANSCRIBE_MISSING" in
    0) return 0 ;;
    1) ;;
    *) echo 'cloud-queue-indexer: TRANSCRIBE_MISSING must be 0 or 1' >&2; exit 1 ;;
  esac
  for value in "$WHISPER_THREADS" "$TRANSCRIPTION_BUDGET_SECONDS"; do
    case "$value" in
      ''|*[!0-9]*|0) echo 'cloud-queue-indexer: transcription bounds must be positive integers' >&2; exit 1 ;;
    esac
  done
  if [ "$TRANSCRIPTION_BUDGET_SECONDS" -ge "$VIDEO_TIMEOUT_SECONDS" ]; then
    echo 'cloud-queue-indexer: transcription budget must be below the video timeout' >&2
    exit 1
  fi
  if [ ! -s "$WHISPER_MODEL" ] || [ ! -r "$WHISPER_MODEL" ] \
    || ! run_indexer "$WHISPER_BINARY" --help >/dev/null 2>&1; then
    echo 'cloud-queue-indexer: enabled Whisper runtime/model is unavailable' >&2
    exit 1
  fi
}

validate_update_schedule() {
  case "$DAILY_UPDATE_INTERVAL_SECONDS" in
    ''|*[!0-9]*)
      echo "cloud-queue-indexer: DAILY_UPDATE_INTERVAL_SECONDS must be an integer" >&2
      exit 1
      ;;
  esac
  case "$DAILY_UPDATE_FAILURE_RETRY_SECONDS" in
    ''|*[!0-9]*)
      echo "cloud-queue-indexer: DAILY_UPDATE_FAILURE_RETRY_SECONDS must be an integer" >&2
      exit 1
      ;;
  esac
  case "$DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS" in
    ''|*[!0-9]*)
      echo "cloud-queue-indexer: DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS must be an integer" >&2
      exit 1
      ;;
  esac
  if [ "$DAILY_UPDATE_ENABLED" = "1" ] && [ "$DAILY_UPDATE_INTERVAL_SECONDS" -lt 60 ]; then
    echo "cloud-queue-indexer: DAILY_UPDATE_INTERVAL_SECONDS must be at least 60" >&2
    exit 1
  fi
}

validate_idle_exit() {
  case "$IDLE_EXIT_AFTER_SECONDS" in
    ''|*[!0-9]*)
      echo "cloud-queue-indexer: IDLE_EXIT_AFTER_SECONDS must be an integer" >&2
      exit 1
      ;;
  esac
}

exit_if_idle_timeout_reached() {
  idle_now="$(date +%s)"
  if [ "$IDLE_EXIT_AFTER_SECONDS" -gt 0 ] && [ $(( idle_now - idle_since )) -ge "$IDLE_EXIT_AFTER_SECONDS" ]; then
    echo "cloud-queue-indexer: idle timeout reached; stopping machine"
    exit 0
  fi
}

validate_idle_gap_fill() {
  case "$IDLE_GAP_FILL_ENABLED" in
    0|1) ;;
    *)
      echo "cloud-queue-indexer: IDLE_GAP_FILL_ENABLED must be 0 or 1" >&2
      exit 1
      ;;
  esac
  case "$IDLE_GAP_FILL_MAX_PASSES" in
    ''|*[!0-9]*|0)
      echo "cloud-queue-indexer: IDLE_GAP_FILL_MAX_PASSES must be a positive integer" >&2
      exit 1
      ;;
  esac
  case "$IDLE_GAP_FILL_MAX_NEW_VIDEOS" in
    ''|*[!0-9]*|0)
      echo "cloud-queue-indexer: IDLE_GAP_FILL_MAX_NEW_VIDEOS must be a positive integer" >&2
      exit 1
      ;;
  esac
  case "$IDLE_GAP_FILL_BATCH_PASSES" in
    ''|*[!0-9]*|0)
      echo "cloud-queue-indexer: IDLE_GAP_FILL_BATCH_PASSES must be a positive integer" >&2
      exit 1
      ;;
  esac
  for interval in "$IDLE_GAP_FILL_INTERVAL_SECONDS" "$ROUTINE_PUBLICATION_INTERVAL_SECONDS"; do
    case "$interval" in
      ''|*[!0-9]*)
        echo "cloud-queue-indexer: gap fill and routine publication intervals must be non-negative integers" >&2
        exit 1
        ;;
    esac
  done
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
    raw) ;;
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

validate_raw_backend() {
  if ! run_indexer ytsearch init-backend --backend raw --collection "$COLLECTION"; then
    echo "cloud-queue-indexer: Raw reader compatibility preflight failed" >&2
    exit 1
  fi
}

log_queue_event() {
  log_level="$1"
  log_event="$2"
  log_request_id="${3:-}"
  log_handle="${4:-}"
  log_source_backend="${5:-}"
  log_backends="${6:-}"
  log_status="${7:-}"

  python3 - "$log_level" "$log_event" "$log_request_id" "$log_handle" "$log_source_backend" "$log_backends" "$log_status" <<'PY'
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

# Ingest only. Never use AXIOM_API_QUERY (operator query token).
token = os.environ.get("AXIOM_API_KEY") or os.environ.get("AXIOM_TOKEN")
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

set_queue_status() {
  request_id="$1"
  status="$2"
  python3 - "$request_id" "$status" <<'PY'
import json
import os
import sys
import urllib.request

request_id, status = sys.argv[1:]
request = urllib.request.Request(
    f"{os.environ['INDEX_QUEUE_API_BASE_URL'].rstrip('/')}/api/index-requests/{request_id}/status",
    data=json.dumps({"status": status}).encode(),
    method="PATCH",
    headers={
        "Authorization": f"Bearer {os.environ['YTSEARCH_INDEX_QUEUE_TOKEN']}",
        "Content-Type": "application/json",
    },
)
with urllib.request.urlopen(request, timeout=30):
    pass
PY
}

set_index_activity() {
  handle="${1:-}"
  attempt=1
  while [ "$attempt" -le 3 ]; do
    if HANDLE="$handle" python3 <<'PY'
import json
import os
import urllib.request

handle = os.environ.get("HANDLE", "")
request = urllib.request.Request(
    os.environ["INDEX_QUEUE_API_BASE_URL"].rstrip("/") + "/api/index-activity",
    data=json.dumps({"handle": handle or None, "target_backend": "raw"}).encode(),
    method="POST",
    headers={
        "Authorization": f"Bearer {os.environ['YTSEARCH_INDEX_QUEUE_TOKEN']}",
        "Content-Type": "application/json",
    },
)
with urllib.request.urlopen(request, timeout=30):
    pass
PY
    then
      return 0
    fi
    attempt=$((attempt + 1))
    sleep 2
  done
  return 1
}

pull_queue_tsv() {
  if ! queue_body="$(queue_request GET "/api/index-requests?limit=1000")"; then
    echo "cloud-queue-indexer: queue request failed" >&2
    return 1
  fi
  if [ -z "$queue_body" ]; then
    echo "cloud-queue-indexer: queue response was empty" >&2
    return 1
  fi
  QUEUE_BODY="$queue_body" QUEUE_PULL_LIMIT="$LIMIT" QUEUE_PRIORITY_LIST_FILE="$QUEUE_PRIORITY_LIST_FILE" QUEUE_TARGET_BACKEND="$(default_queue_target_backend)" QUEUE_RETRY_STATE_FILE="$QUEUE_RETRY_STATE_FILE" \
    python3 <<'PY'
import json
import os
import sys

raw = os.environ.get("QUEUE_BODY", "").strip()
if not raw:
    print("cloud-queue-indexer: queue response was empty", file=sys.stderr)
    sys.exit(1)
try:
    body = json.loads(raw)
except json.JSONDecodeError as error:
    print(f"cloud-queue-indexer: invalid queue response: {error}", file=sys.stderr)
    sys.exit(1)
if not isinstance(body, dict) or not isinstance(body.get("requests"), list):
    print("cloud-queue-indexer: queue response is missing a requests array", file=sys.stderr)
    sys.exit(1)
priority_file = os.environ.get("QUEUE_PRIORITY_LIST_FILE", "")
retry_file = os.environ.get("QUEUE_RETRY_STATE_FILE", "")
target_backend = os.environ.get("QUEUE_TARGET_BACKEND", "").strip()
priority_handles = set()
retry_after = {}
if retry_file and os.path.isfile(retry_file):
    with open(retry_file, encoding="utf-8") as handle:
        for raw_line in handle:
            request_id, separator, epoch = raw_line.strip().partition("\t")
            if separator and request_id and epoch.isdigit():
                retry_after[request_id] = int(epoch)
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
    if request.get("status") not in {"queued", "running"}:
        continue
    request_id = request.get("id", "")
    handle = request.get("handle", "")
    source_backend = request.get("source_backend", "")
    request_target_backend = request.get("target_backend") or ""
    if target_backend and request_target_backend != target_backend:
        continue
    if request_id and handle:
        if retry_after.get(request_id, 0) > int(__import__("time").time()):
            continue
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

remember_queue_retry() {
  request_id="$1"
  delay_seconds="$2"
  state_dir="$(dirname "$QUEUE_RETRY_STATE_FILE")"
  mkdir -p "$state_dir"
  tmp_state="${QUEUE_RETRY_STATE_FILE}.tmp.$$"
  retry_at=$(( $(date +%s) + delay_seconds ))
  if [ -f "$QUEUE_RETRY_STATE_FILE" ]; then
    awk -F '\t' -v id="$request_id" '$1 != id' "$QUEUE_RETRY_STATE_FILE" > "$tmp_state"
  else
    : > "$tmp_state"
  fi
  printf '%s\t%s\n' "$request_id" "$retry_at" >> "$tmp_state"
  mv "$tmp_state" "$QUEUE_RETRY_STATE_FILE"
}

forget_queue_retry() {
  request_id="$1"
  [ -f "$QUEUE_RETRY_STATE_FILE" ] || return 0
  tmp_state="${QUEUE_RETRY_STATE_FILE}.tmp.$$"
  awk -F '\t' -v id="$request_id" '$1 != id' "$QUEUE_RETRY_STATE_FILE" > "$tmp_state"
  mv "$tmp_state" "$QUEUE_RETRY_STATE_FILE"
}

pull_idle_gap_handle() {
  cursor=""
  if [ -f "$IDLE_GAP_FILL_STATE_FILE" ]; then
    cursor="$(sed -n '1p' "$IDLE_GAP_FILL_STATE_FILE" | tr '[:upper:]' '[:lower:]')"
  fi
  CURSOR="$cursor" python3 <<'PY'
import json
import os
import sys
import urllib.request

url = os.environ["INDEX_QUEUE_API_BASE_URL"].rstrip("/") + "/api/youtubers"
try:
    with urllib.request.urlopen(url, timeout=30) as response:
        body = json.load(response)
except (OSError, ValueError) as error:
    print(f"cloud-queue-indexer: gap catalog request failed: {error}", file=sys.stderr)
    sys.exit(0)

handles = sorted(
    {
        str(channel.get("handle") or "").strip().lower()
        for channel in body.get("channels", [])
        if int(channel.get("indexed_video_count") or 0)
        < int(channel.get("public_video_count") or 0)
    }
    - {""}
)
cursor = os.environ.get("CURSOR", "").strip().lower()
selected = next((handle for handle in handles if handle > cursor), None)
if selected is None and handles:
    selected = handles[0]
if selected:
    print(selected)
PY
}

remember_idle_gap_handle() {
  handle="$1"
  state_dir="$(dirname "$IDLE_GAP_FILL_STATE_FILE")"
  mkdir -p "$state_dir"
  tmp_state="${IDLE_GAP_FILL_STATE_FILE}.tmp.$$"
  printf '%s\n' "$handle" > "$tmp_state"
  mv "$tmp_state" "$IDLE_GAP_FILL_STATE_FILE"
}

next_daily_update_epoch() {
  now="$1"
  last_completed=""
  if [ -f "$DAILY_UPDATE_STATE_FILE" ]; then
    last_completed="$(sed -n '1p' "$DAILY_UPDATE_STATE_FILE")"
  fi
  case "$last_completed" in
    ''|*[!0-9]*) printf '%s\n' "$(( now + DAILY_UPDATE_INITIAL_DELAY_SECONDS ))" ;;
    *)
      if [ "$last_completed" -gt "$now" ]; then
        printf '%s\n' "$now"
      else
        due=$(( last_completed + DAILY_UPDATE_INTERVAL_SECONDS ))
        retry_at="$(daily_update_retry_field 1)"
        if [ -n "$retry_at" ] && [ "$retry_at" -gt "$due" ] \
          && [ "$retry_at" -le $(( now + DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS )) ]; then
          due="$retry_at"
        fi
        printf '%s\n' "$due"
      fi
      ;;
  esac
}

# daily_update_retry_field N prints field N (1 = retry epoch, 2 = consecutive
# failures) of the retry file, or nothing when it is missing or invalid.
daily_update_retry_field() {
  sed -n '1p' "$DAILY_UPDATE_RETRY_FILE" 2>/dev/null | awk -v n="$1" '
    $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { print $n }'
}

# Records a failed daily pass and prints when it may run again.
remember_daily_update_failure() {
  failures=$(( $(daily_update_retry_field 2 || true) + 1 ))
  delay="$DAILY_UPDATE_FAILURE_RETRY_SECONDS"
  step=1
  while [ "$step" -lt "$failures" ] && [ "$delay" -lt "$DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS" ]; do
    delay=$(( delay * 2 ))
    step=$(( step + 1 ))
  done
  [ "$delay" -le "$DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS" ] || delay="$DAILY_UPDATE_FAILURE_RETRY_MAX_SECONDS"
  retry_at=$(( $(date +%s) + delay ))
  remember_epoch "$DAILY_UPDATE_RETRY_FILE" "$retry_at $failures" || true
  printf '%s\n' "$retry_at"
}

remember_daily_update_completion() {
  completed_at="$1"
  state_dir="$(dirname "$DAILY_UPDATE_STATE_FILE")"
  mkdir -p "$state_dir"
  tmp_state="${DAILY_UPDATE_STATE_FILE}.tmp.$$"
  printf '%s\n' "$completed_at" > "$tmp_state"
  mv "$tmp_state" "$DAILY_UPDATE_STATE_FILE"
  rm -f "$DAILY_UPDATE_RETRY_FILE"
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
  if run_with_transcription ytsearch index-youtuber "$handle" \
    --backend "$backend" \
    --mode update \
    --collection "$COLLECTION" \
    --jobs "$jobs" \
    --video-timeout-seconds "$VIDEO_TIMEOUT_SECONDS" \
    --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
    --transcript-failure-retry-seconds "$TRANSCRIPT_FAILURE_RETRY_SECONDS" \
    $extra_args \
    "$@"; then
    return 0
  else
    return $?
  fi
}

index_youtuber_for_backends() (
  handle="$1"
  shift
  failed=0
  raw_jobs="${RAW_JOBS:-$JOBS}"

  if has_backend raw; then
    if [ -z "${RAW_SEARCH_API_URL:-}" ] || [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ]; then
      echo "cloud-queue-indexer: raw unavailable; missing RAW_SEARCH_API_URL or RAW_SEARCH_ADMIN_API_KEY" >&2
      failed=1
    else
      if index_one_backend raw raw "$raw_jobs" "$handle" "$@"; then
        :
      else
        failed=$?
        echo "cloud-queue-indexer: backend failed handle=$handle backend=raw" >&2
      fi
    fi
  fi

  return "$failed"
)

index_youtuber_for_queue_target() {
  handle="$1"
  target_backend="$2"
  shift 2

  if [ -z "$target_backend" ]; then
    index_youtuber_for_backends "$handle" "$@"
    return $?
  fi

  case "$target_backend" in
    raw)
      if [ -z "${RAW_SEARCH_API_URL:-}" ] || [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ]; then
        echo "cloud-queue-indexer: raw unavailable; missing RAW_SEARCH_API_URL or RAW_SEARCH_ADMIN_API_KEY" >&2
        return 1
      fi
      index_one_backend raw raw "${RAW_JOBS:-$JOBS}" "$handle" "$@"
      ;;
    *)
      echo "cloud-queue-indexer: unsupported target_backend=$target_backend request skipped" >&2
      return 1
      ;;
  esac
}

publish_raw_generation() {
  echo "cloud-queue-indexer: publishing one coalesced Raw generation"
  run_indexer ytsearch publish --backend raw --collection "$COLLECTION"
}

# mark_raw_publication_pending [urgent|routine]: routine work never downgrades
# an urgent marker, and a marker without a kind (older workers) is urgent.
mark_raw_publication_pending() {
  pending_kind="${1:-urgent}"
  if [ "$pending_kind" = routine ] && [ -f "$RAW_PENDING_PUBLICATION_FILE" ]; then
    case "$(sed -n '1p' "$RAW_PENDING_PUBLICATION_FILE")" in
      *" routine") ;;
      *) pending_kind=urgent ;;
    esac
  fi
  state_dir="$(dirname "$RAW_PENDING_PUBLICATION_FILE")"
  mkdir -p "$state_dir" || return 1
  pending_tmp="${RAW_PENDING_PUBLICATION_FILE}.tmp.$$"
  printf '%s %s\n' "$(date +%s)" "$pending_kind" > "$pending_tmp" || return 1
  if ! mv "$pending_tmp" "$RAW_PENDING_PUBLICATION_FILE"; then
    rm -f "$pending_tmp"
    return 1
  fi
}

publish_pending_raw_generation() {
  [ -f "$RAW_PENDING_PUBLICATION_FILE" ] || return 0
  if ! publish_raw_generation; then
    return 1
  fi
  rm -f "$RAW_PENDING_PUBLICATION_FILE"
  remember_epoch "$RAW_LAST_PUBLICATION_FILE" "$(date +%s)" || true
}

# remember_epoch FILE EPOCH writes the timestamp atomically.
remember_epoch() {
  mkdir -p "$(dirname "$1")" || return 1
  printf '%s\n' "$2" > "$1.tmp.$$" && mv "$1.tmp.$$" "$1"
}

# interval_elapsed SECONDS FILE: true when the interval is 0 or the epoch in
# FILE is missing, invalid or at least SECONDS old.
interval_elapsed() {
  [ "$1" -gt 0 ] || return 0
  last="$(sed -n '1p' "$2" 2>/dev/null || true)"
  case "$last" in
    ''|*[!0-9]*) return 0 ;;
  esac
  [ $(( $(date +%s) - last )) -ge "$1" ]
}

# True when a marker exists and is urgent, or routine and the interval since
# the last publication has passed.
pending_publication_due() {
  [ -f "$RAW_PENDING_PUBLICATION_FILE" ] || return 1
  case "$(sed -n '1p' "$RAW_PENDING_PUBLICATION_FILE")" in
    *" routine") interval_elapsed "$ROUTINE_PUBLICATION_INTERVAL_SECONDS" "$RAW_LAST_PUBLICATION_FILE" ;;
    *) return 0 ;;
  esac
}

index_catalog_backend() {
  label="$1"
  backend="$2"
  jobs="$3"
  shift 3

  extra_args=""
  if [ -n "${MAX_VIDEOS_PER_PASS:-}" ] && [ "$MAX_VIDEOS_PER_PASS" -gt 0 ] 2>/dev/null; then
    extra_args="--max-videos $MAX_VIDEOS_PER_PASS"
  fi

  set_index_activity "catalog:$label" || true
  echo "cloud-queue-indexer: indexing catalog backend=$label jobs=$jobs list=$DAILY_UPDATE_LIST_FILE"
  # One process owns one catalog run ID, one complete set of creator manifests,
  # and one deferred publication boundary. Per-creator processes cannot prove
  # that their discovery results describe the same catalog pass.
  # shellcheck disable=SC2086
  run_with_transcription ytsearch index-youtubers "$DAILY_UPDATE_LIST_FILE" \
    --backend "$backend" \
    --mode update \
    --collection "$COLLECTION" \
    --jobs "$jobs" \
    --channel-jobs 1 \
    --video-timeout-seconds "$VIDEO_TIMEOUT_SECONDS" \
    --stop-after-existing "$EXISTING_TAIL" \
    --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
    --transcript-failure-retry-seconds "$TRANSCRIPT_FAILURE_RETRY_SECONDS" \
    --defer-publish \
    $extra_args \
    "$@"
}

# Build one approved refresh catalog. Canonical lines retain their labels and
# spelling; directory-only handles extend it even when their count gap is zero.
build_daily_update_catalog() {
  python3 - "$1" "$2" <<'PYTHON'
import json
import os
import pathlib
import re
import sys
import urllib.request

# The output is ordered canonical lines followed by unseen directory handles;
# membership uses the same ASCII identity as youtube_ingest::stable_youtuber_id.
canonical_lines: list[str] = []
seen_ids: set[str] = set()
directory_handles: list[str] = []


def creator_identity(handle: object) -> str:
    if not isinstance(handle, str) or not handle.startswith("@"):
        raise ValueError("creator handle must be a string beginning with @")
    source = handle[1:]
    if not source or any(not (char.isalnum() or char in "_.-") for char in source):
        raise ValueError("creator handle contains invalid characters")
    identity = re.sub(r"[^A-Za-z0-9]+", "-", source).lower().strip("-")
    if not identity:
        raise ValueError("creator handle has no stable identity")
    return identity


try:
    source, destination = map(pathlib.Path, sys.argv[1:])
    for line in source.read_text(encoding="utf-8").splitlines(keepends=True):
        entry = line.strip()
        if not entry or entry.startswith("#"):
            canonical_lines.append(line)
            continue
        identity = creator_identity(entry.split("|", 1)[0].strip())
        if identity not in seen_ids:
            seen_ids.add(identity)
            canonical_lines.append(line)
    if not seen_ids:
        raise ValueError("canonical creator list has no usable entries")

    url = os.environ["INDEX_QUEUE_API_BASE_URL"].rstrip("/") + "/api/youtubers"
    with urllib.request.urlopen(url, timeout=30) as response:
        body = json.load(response)
    if not isinstance(body, dict) or not isinstance(body.get("channels"), list):
        raise ValueError("approved directory must contain a channels array")
    for channel in body["channels"]:
        if not isinstance(channel, dict):
            raise ValueError("approved directory contains a non-object channel")
        handle = channel.get("handle")
        identity = creator_identity(handle)
        if identity not in seen_ids:
            seen_ids.add(identity)
            directory_handles.append(handle)

    # Validate the entire directory before writing any catalog content. A failed
    # read must not silently turn a partial canonical pass into a full sweep.
    catalog = "".join(canonical_lines)
    if catalog and not catalog.endswith("\n"):
        catalog += "\n"
    catalog += "".join(handle + "\n" for handle in directory_handles)
    destination.write_text(catalog, encoding="utf-8")
except (OSError, ValueError, KeyError) as error:
    print(f"cloud-queue-indexer: daily approved catalog failed: {error}", file=sys.stderr)
    sys.exit(1)
PYTHON
}

# Isolate the temporary list and its cleanup from the queue daemon's state and
# traps, including failures before indexing or during deferred publication.
index_daily_update_for_backends() (
  if [ ! -f "$DAILY_UPDATE_LIST_FILE" ]; then
    echo "cloud-queue-indexer: daily update failed; list file not found: $DAILY_UPDATE_LIST_FILE" >&2
    return 1
  fi
  if ! awk -F '|' '
    { gsub(/[[:space:]]/, "", $1); if ($1 != "" && $1 !~ /^#/) found = 1 }
    END { exit !found }
  ' "$DAILY_UPDATE_LIST_FILE"; then
    echo "cloud-queue-indexer: daily update failed; creator list has no usable entries" >&2
    return 1
  fi

  daily_catalog_file="$(mktemp "${TMPDIR:-/tmp}/ytsearch-daily-catalog.XXXXXX")" || return 1
  trap 'rm -f "$daily_catalog_file"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  if ! build_daily_update_catalog "$DAILY_UPDATE_LIST_FILE" "$daily_catalog_file"; then
    log_queue_event error daily_update_catalog_failed "" "" "" "raw" "failed"
    return 1
  fi
  # The unprivileged indexer needs to read this public-only creator list.
  chmod 644 "$daily_catalog_file" || return 1
  DAILY_UPDATE_LIST_FILE="$daily_catalog_file"

  failed=0
  raw_jobs="${RAW_JOBS:-$JOBS}"

  if has_backend raw; then
    if [ -z "${RAW_SEARCH_API_URL:-}" ] || [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ]; then
      echo "cloud-queue-indexer: daily Raw update unavailable; missing RAW_SEARCH_API_URL or RAW_SEARCH_ADMIN_API_KEY" >&2
      failed=1
    else
      # Routine until the pass completes: a failed pass that is retried
      # later must not rebuild the whole generation on every attempt.
      if ! mark_raw_publication_pending routine; then
        log_queue_event error daily_update_marker_failed "" "" "" "raw" "failed"
        return 1
      fi
      catalog_status=0
      index_catalog_backend raw raw "$raw_jobs" || catalog_status=$?
      echo "cloud-queue-indexer: daily catalog pass exit=$catalog_status"
      if [ "$catalog_status" -eq 0 ] || [ "$catalog_status" -eq 75 ]; then
        mark_raw_publication_pending urgent || failed=1
      fi
      if [ "$catalog_status" -eq 75 ]; then
        # Every creator was attempted; only per-video gaps with a scheduled
        # retry remain (for example captions not yet published on a new
        # upload). The failure ledger retries them on later passes.
        log_queue_event warn daily_update_backend_gaps "" "" "" "raw" "completed_with_gaps"
      elif [ "$catalog_status" -ne 0 ]; then
        failed=1
        log_queue_event error daily_update_backend_failed "" "" "" "raw" "failed"
      fi
      if pending_publication_due && ! publish_pending_raw_generation; then
        failed=1
        log_queue_event error daily_update_publication_failed "" "" "" "raw" "failed"
      fi
    fi
  fi

  return "$failed"
)

process_idle_gap_once() {
  IDLE_GAP_HAD_WORK=0
  handle="$(pull_idle_gap_handle)"
  if [ -z "$handle" ]; then
    echo "cloud-queue-indexer: no indexed/public video gaps found"
    return 0
  fi

  IDLE_GAP_HAD_WORK=1
  echo "cloud-queue-indexer: idle gap fill handle=$handle"
  log_queue_event info idle_gap_fill_started "" "$handle" "" "$BACKENDS" "running"
  set_index_activity "$handle" || true
  if ! mark_raw_publication_pending routine; then
    IDLE_GAP_HAD_WORK=0
    set_index_activity "" || true
    log_queue_event error idle_gap_marker_failed "" "$handle" "" "$BACKENDS" "failed"
    return 1
  fi
  status=0
  index_youtuber_for_backends "$handle" \
    --max-new-videos "$IDLE_GAP_FILL_MAX_NEW_VIDEOS" \
    --defer-publish || status=$?
  if [ "$status" -eq 0 ]; then
    log_queue_event info idle_gap_fill_completed "" "$handle" "" "$BACKENDS" "completed"
  else
    log_queue_event error idle_gap_fill_failed "" "$handle" "" "$BACKENDS" "failed"
  fi
  remember_idle_gap_handle "$handle"
  set_index_activity "" || true
  return "$status"
}

process_idle_gap_batch() {
  maximum_passes="$1"
  IDLE_GAP_HAD_WORK=0
  IDLE_GAP_BATCH_PASSES=0
  batch_failed=0

  while [ "$IDLE_GAP_BATCH_PASSES" -lt "$maximum_passes" ]; do
    IDLE_GAP_HAD_WORK=0
    if ! process_idle_gap_once; then
      batch_failed=1
    fi
    if [ "$IDLE_GAP_HAD_WORK" != "1" ]; then
      break
    fi
    IDLE_GAP_BATCH_PASSES=$((IDLE_GAP_BATCH_PASSES + 1))
    if [ "$batch_failed" -eq 1 ]; then
      break
    fi
    if [ "$IDLE_GAP_BATCH_PASSES" -lt "$maximum_passes" ]; then
      waiting_queue="$(pull_queue_tsv)" || break
      if [ -n "$waiting_queue" ]; then
        echo 'cloud-queue-indexer: yielding gap repair to queued requests'
        break
      fi
    fi
  done

  if [ "$IDLE_GAP_BATCH_PASSES" -gt 0 ]; then
    IDLE_GAP_HAD_WORK=1
    if ! pending_publication_due; then
      log_queue_event info idle_gap_batch_publication_deferred "" "" "" "$BACKENDS" "passes=$IDLE_GAP_BATCH_PASSES"
    elif ! publish_pending_raw_generation; then
      batch_failed=1
      log_queue_event error idle_gap_batch_publication_failed "" "" "" "$BACKENDS" "failed"
    else
      log_queue_event info idle_gap_batch_published "" "" "" "$BACKENDS" "passes=$IDLE_GAP_BATCH_PASSES"
    fi
  fi

  [ "$batch_failed" -eq 0 ]
}

if [ "${CLOUD_INDEX_QUEUE_LIBRARY_ONLY:-0}" != "1" ]; then
  require_env INDEX_QUEUE_API_BASE_URL
  require_env YTSEARCH_INDEX_QUEUE_TOKEN
  require_env RAW_SEARCH_API_URL
  require_env RAW_SEARCH_ADMIN_API_KEY
  require_env YOUTUBE_API_KEY_FILE
  require_env YT_DLP_COOKIES_FILE
  require_env CREDENTIAL_PREFLIGHT_GENERATION
  if ! printf '%s' "$CREDENTIAL_PREFLIGHT_GENERATION" | grep -Eq '^[A-Za-z0-9._-]{1,128}$'; then
    echo "cloud-queue-indexer: credential preflight generation is invalid" >&2
    exit 1
  fi
  rm -f "$CREDENTIAL_PREFLIGHT_MARKER"
  validate_backend_scope
  validate_raw_backend
  validate_file_credentials
  prepare_unprivileged_indexer
  validate_transcription
  validate_update_schedule
  validate_idle_exit
  validate_idle_gap_fill
  : > "$CREDENTIAL_PREFLIGHT_MARKER"
  echo "cloud-queue-indexer: credential preflight passed generation=$CREDENTIAL_PREFLIGHT_GENERATION"
fi

if [ "${CLOUD_INDEX_QUEUE_PREFLIGHT_ONLY:-0}" = "1" ]; then
  exit 0
fi

# ack_queue_request ID HANDLE SOURCE BACKENDS removes a completed request, or
# puts it back in the queue for a later pass when the API refuses.
ack_queue_request() {
  if queue_request DELETE "/api/index-requests/$1" >/dev/null; then
    echo "cloud-queue-indexer: acked request=$1 handle=$2"
    forget_queue_retry "$1"
    log_queue_event info index_request_completed "$1" "$2" "$3" "$4" "acked"
    processed=$((processed + 1))
  else
    failed=$((failed + 1))
    unresolved=$((unresolved + 1))
    remember_queue_retry "$1" "$QUEUE_ACK_RETRY_SECONDS"
    set_queue_status "$1" queued || true
    log_queue_event error index_request_ack_failed "$1" "$2" "$3" "$4" "queued"
  fi
}

process_queue_once() {
  QUEUE_HAD_WORK=0
  processed=0
  failed=0
  failed_acked=0
  unresolved=0
  tmp_queue="/tmp/ytsearch-queue.tsv"
  tmp_success="/tmp/ytsearch-queue-success.tsv"
  : > "$tmp_success"
  publication_needed=0
  if ! pull_queue_tsv > "$tmp_queue"; then
    echo "cloud-queue-indexer: queue pull failed" >&2
    log_queue_event error queue_pull_failed "" "" "" "$BACKENDS" "retrying"
    return 1
  fi

  if [ ! -s "$tmp_queue" ]; then
    echo "cloud-queue-indexer: queue is empty"
    log_queue_event info queue_empty "" "" "" "$BACKENDS" "idle"
    return 0
  fi

  queue_depth="$(wc -l < "$tmp_queue" | tr -d ' ')"
  log_queue_event info queue_poll "" "" "" "$BACKENDS" "depth=$queue_depth"

  while IFS="$(printf '\t')" read -r request_id handle source_backend target_backend; do
    [ -n "$request_id" ] || continue
    QUEUE_HAD_WORK=1
    if [ -n "$target_backend" ]; then
      active_backends="$target_backend"
    else
      active_backends="$BACKENDS"
    fi
    echo "cloud-queue-indexer: processing request=$request_id handle=$handle source_backend=$source_backend target_backend=$target_backend"
    set_index_activity "$handle" || true
    if ! set_queue_status "$request_id" running; then
      echo "cloud-queue-indexer: could not claim request=$request_id" >&2
      failed=$((failed + 1))
      unresolved=$((unresolved + 1))
      remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
      continue
    fi
    log_queue_event info index_request_started "$request_id" "$handle" "$source_backend" "$active_backends" "running"
    # Upload notifications (`upload-` IDs from the API's WebSub callback) are
    # routine: their writes are durable once indexed, so the request is acked
    # at once and the generation is rebuilt with the next routine publication
    # instead of once per upload.
    case "$request_id" in
      upload-*) request_publication=routine ;;
      *) request_publication=urgent ;;
    esac
    if ! mark_raw_publication_pending "$request_publication"; then
      failed=$((failed + 1))
      unresolved=$((unresolved + 1))
      remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
      set_queue_status "$request_id" queued || true
      log_queue_event error publication_marker_failed "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
      break
    fi
    index_status=0
    index_youtuber_for_queue_target "$handle" "$target_backend" --defer-publish || index_status=$?
    [ "$request_publication" = routine ] || publication_needed=1
    if [ "$index_status" -eq 0 ] && [ "$request_publication" = routine ]; then
      ack_queue_request "$request_id" "$handle" "$source_backend" "$active_backends"
    elif [ "$index_status" -eq 0 ]; then
      printf '%s\t%s\t%s\t%s\n' "$request_id" "$handle" "$source_backend" "$active_backends" >> "$tmp_success"
    else
      failed=$((failed + 1))
      log_queue_event error index_request_failed "$request_id" "$handle" "$source_backend" "$active_backends" "failed"
      if [ "$ACK_FAILED_REQUESTS" = "1" ]; then
        if queue_request DELETE "/api/index-requests/$request_id" >/dev/null; then
          failed_acked=$((failed_acked + 1))
          forget_queue_retry "$request_id"
          log_queue_event warn index_request_failed_acked "$request_id" "$handle" "$source_backend" "$active_backends" "deleted"
        else
          unresolved=$((unresolved + 1))
          remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
          set_queue_status "$request_id" queued || true
          log_queue_event error index_request_failed_ack_error "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
        fi
      elif [ "$index_status" -eq 70 ]; then
        # A temporary backend connection/service failure needs a short retry,
        # independent of the longer YouTube/transcript cooldown.
        unresolved=$((unresolved + 1))
        remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
        set_queue_status "$request_id" queued || true
        log_queue_event warn index_request_backend_unavailable "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
      elif [ "$index_status" -eq 75 ] || [ "$index_status" -eq 69 ]; then
        # 75: only retryable per-video gaps remain. 69: a bot challenge or rate
        # limit stopped the run. Either way put the request back into the
        # runnable queue so a later pass can complete it.
        unresolved=$((unresolved + 1))
        remember_queue_retry "$request_id" "$TRANSCRIPT_FAILURE_RETRY_SECONDS"
        set_queue_status "$request_id" queued || true
        log_queue_event warn index_request_failed_retained "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
      else
        if set_queue_status "$request_id" failed; then
          forget_queue_retry "$request_id"
          log_queue_event error index_request_terminal_failure "$request_id" "$handle" "$source_backend" "$active_backends" "failed"
        else
          unresolved=$((unresolved + 1))
          remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
          log_queue_event error index_request_terminal_status_failed "$request_id" "$handle" "$source_backend" "$active_backends" "retrying"
        fi
      fi
    fi
    if [ "$QUEUE_ONE_REQUEST_PER_PASS" = "1" ]; then
      echo "cloud-queue-indexer: one-request-per-pass enabled; deferring remaining queue items"
      break
    fi
  done < "$tmp_queue"

  if [ "$publication_needed" -eq 1 ]; then
    if publish_pending_raw_generation; then
      while IFS="$(printf '\t')" read -r request_id handle source_backend active_backends; do
        [ -n "$request_id" ] || continue
        ack_queue_request "$request_id" "$handle" "$source_backend" "$active_backends"
      done < "$tmp_success"
    else
      failed=$((failed + 1))
      unresolved=$((unresolved + 1))
      while IFS="$(printf '\t')" read -r request_id handle source_backend active_backends; do
        [ -n "$request_id" ] || continue
        remember_queue_retry "$request_id" "$QUEUE_ACK_RETRY_SECONDS"
        set_queue_status "$request_id" queued || true
        log_queue_event error index_request_publication_failed "$request_id" "$handle" "$source_backend" "$active_backends" "queued"
      done < "$tmp_success"
    fi
  fi

  set_index_activity "" || true

  echo "cloud-queue-indexer: pass complete processed=$processed failed=$failed failed_acked=$failed_acked"
  [ "$unresolved" -eq 0 ]
}

if [ "${CLOUD_INDEX_QUEUE_LIBRARY_ONLY:-0}" = "1" ]; then
  return 0 2>/dev/null || exit 0
fi

if [ "$QUEUE_LOOP" = "1" ]; then
  echo "cloud-queue-indexer: daemon mode enabled poll_interval=${QUEUE_POLL_INTERVAL_SECONDS}s"
  idle_since="$(date +%s)"
  idle_gap_passes=0
  now="$(date +%s)"
  next_daily_update="$(next_daily_update_epoch "$now")"
  if [ "$DAILY_UPDATE_ENABLED" = "1" ]; then
    echo "cloud-queue-indexer: daily update enabled initial_delay=${DAILY_UPDATE_INITIAL_DELAY_SECONDS}s interval=${DAILY_UPDATE_INTERVAL_SECONDS}s next_after_epoch=$next_daily_update"
  else
    echo "cloud-queue-indexer: daily update disabled"
  fi
  while :; do
    if pending_publication_due \
      && ! publish_pending_raw_generation; then
      echo "cloud-queue-indexer: pending Raw publication failed; retrying before more work" >&2
      log_queue_event error pending_publication_retry_failed "" "" "" "$BACKENDS" "failed"
      exit_if_idle_timeout_reached
      sleep "$QUEUE_POLL_INTERVAL_SECONDS"
      continue
    fi
    process_queue_once || echo "cloud-queue-indexer: queue pass failed; continuing after poll interval" >&2
    if [ "$QUEUE_HAD_WORK" = "1" ]; then
      idle_since="$(date +%s)"
    fi
    DAILY_UPDATE_HAD_WORK=0
    if [ "$DAILY_UPDATE_ENABLED" = "1" ] && [ "$QUEUE_HAD_WORK" = "0" ]; then
      now="$(date +%s)"
      if [ "$now" -ge "$next_daily_update" ]; then
        DAILY_UPDATE_HAD_WORK=1
        echo "cloud-queue-indexer: running scheduled daily update before gap repair"
        log_queue_event info daily_update_started "" "" "" "$BACKENDS" "running"
        if index_daily_update_for_backends; then
          log_queue_event info daily_update_completed "" "" "" "$BACKENDS" "completed"
          completed_at="$(date +%s)"
          remember_daily_update_completion "$completed_at"
          next_daily_update=$(( completed_at + DAILY_UPDATE_INTERVAL_SECONDS ))
          idle_since="$completed_at"
          echo "cloud-queue-indexer: next daily update after epoch=$next_daily_update"
        else
          log_queue_event error daily_update_failed "" "" "" "$BACKENDS" "failed"
          next_daily_update="$(remember_daily_update_failure)"
          echo "cloud-queue-indexer: daily update failed; retrying after epoch=$next_daily_update"
        fi
        set_index_activity "" || true
      fi
    fi
    IDLE_GAP_HAD_WORK=0
    IDLE_GAP_BATCH_PASSES=0
    if [ "$idle_gap_passes" -ge "$IDLE_GAP_FILL_MAX_PASSES" ] \
      && interval_elapsed "$IDLE_GAP_FILL_INTERVAL_SECONDS" "$IDLE_GAP_FILL_STARTED_FILE"; then
      idle_gap_passes=0
    fi
    if [ "$IDLE_GAP_FILL_ENABLED" = "1" ] && [ "$QUEUE_HAD_WORK" = "0" ] && [ "$DAILY_UPDATE_HAD_WORK" = "0" ] && [ "$idle_gap_passes" -lt "$IDLE_GAP_FILL_MAX_PASSES" ] \
      && { [ "$idle_gap_passes" -gt 0 ] || interval_elapsed "$IDLE_GAP_FILL_INTERVAL_SECONDS" "$IDLE_GAP_FILL_STARTED_FILE"; }; then
      if [ "$idle_gap_passes" -eq 0 ]; then
        remember_epoch "$IDLE_GAP_FILL_STARTED_FILE" "$(date +%s)" || true
      fi
      remaining_gap_passes=$((IDLE_GAP_FILL_MAX_PASSES - idle_gap_passes))
      gap_batch_passes="$IDLE_GAP_FILL_BATCH_PASSES"
      if [ "$remaining_gap_passes" -lt "$gap_batch_passes" ]; then
        gap_batch_passes="$remaining_gap_passes"
      fi
      process_idle_gap_batch "$gap_batch_passes" || echo "cloud-queue-indexer: idle gap batch failed; continuing after poll interval" >&2
      if [ "$IDLE_GAP_HAD_WORK" = "1" ]; then
        idle_gap_passes=$((idle_gap_passes + IDLE_GAP_BATCH_PASSES))
        idle_since="$(date +%s)"
      fi
    fi
    exit_if_idle_timeout_reached
    sleep "$QUEUE_POLL_INTERVAL_SECONDS"
  done
fi

if pending_publication_due; then
  publish_pending_raw_generation
fi
process_queue_once
