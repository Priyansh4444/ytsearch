#!/usr/bin/env sh
set -eu

# Check that exactly one on-demand Raw queue worker exists. A stopped
# machine is healthy while the queue is idle; Cloudflare starts it on approval.

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
EXPECTED_RAW_SEARCH_API_URL="${EXPECTED_RAW_SEARCH_API_URL:-http://ytsearch-raw-production.internal:8108}"
EXPECTED_CREATOR_LIST_FILE="${EXPECTED_CREATOR_LIST_FILE:-$SCRIPT_DIR/indexed-channels.txt}"
EXPECTED_ENTRYPOINT_FILE="${EXPECTED_ENTRYPOINT_FILE:-$SCRIPT_DIR/cloud-index-queue-entrypoint.sh}"
REQUIRED_STATES="${REQUIRED_QUEUE_WORKER_STATES:-}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required." >&2
  exit 1
fi

if [ -z "${MACHINES_JSON_FILE:-}" ] && ! command -v flyctl >/dev/null 2>&1; then
  echo "flyctl is required when MACHINES_JSON_FILE is not provided." >&2
  exit 1
fi

if [ -n "${MACHINES_JSON_FILE:-}" ]; then
  machines="$(cat "$MACHINES_JSON_FILE")"
else
  machines="$(flyctl machine list --app "$APP_NAME" --json)"
fi
printf 'backend\tmachine\tstate\trestart\n'
printf '%s' "$machines" | jq -r '
  .[]
  | select((.config.env.YTSEARCH_MACHINE_ROLE // "") == "queue-indexer")
  | [
      .config.env.QUEUE_TARGET_BACKEND,
      .name,
      .state,
      (.config.restart.policy // "")
    ]
  | @tsv
'

count="$(printf '%s' "$machines" | jq '[.[] | select(
  .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
)] | length')"
if [ "$count" -ne 1 ]; then
  echo "Expected exactly one configured raw queue worker; found $count." >&2
  exit 1
fi

if ! printf '%s' "$machines" | jq -e \
  --arg raw_url "$EXPECTED_RAW_SEARCH_API_URL" \
  --arg required_states "$REQUIRED_STATES" '
  first(.[] | select(
    .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
    and .config.env.QUEUE_TARGET_BACKEND == "raw"
  ))
  | (.state == "created" or .state == "started" or .state == "starting" or .state == "stopped")
    and ($required_states == "" or (.state as $state | ($required_states | split(",") | index($state)) != null))
    and (.config.env.QUEUE_LOOP == "1")
    and (.config.env.DAILY_UPDATE_ENABLED == "1")
    and (.config.env.DAILY_UPDATE_INTERVAL_SECONDS == "86400")
    and (.config.env.DAILY_UPDATE_INITIAL_DELAY_SECONDS == "0")
    and (.config.env.DAILY_UPDATE_STATE_FILE == "/var/lib/ytsearch/daily-update.last-completed")
    and ((.config.env.IDLE_EXIT_AFTER_SECONDS | tonumber?) // 0) > 0
    and ((.config.env.RAW_PUBLICATION_TIMEOUT_SECONDS | tonumber?) // 0) >= 2700
    and (.config.env.RAW_SEARCH_API_URL == $raw_url)
    and (.config.env.YOUTUBE_API_KEY_FILE == "/run/secrets/youtube-api-key.txt")
    and (.config.env.YT_DLP_COOKIES_FILE == "/run/secrets/yt-dlp-cookies.txt")
    and (.config.env.CREDENTIAL_PREFLIGHT_MARKER == "/run/ytsearch-credential-preflight.ok")
    and ((.config.env.CREDENTIAL_PREFLIGHT_GENERATION // "") | test("^[A-Za-z0-9._-]{1,128}$"))
    and (.config.env.QUEUE_ONE_REQUEST_PER_PASS == "0")
    and (((.config.env.LIMIT | tonumber?) // 0) >= 100)
    and (.config.env.IDLE_GAP_FILL_ENABLED == "1")
    and (((.config.env.IDLE_GAP_FILL_MAX_PASSES | tonumber?) // 0) > 0)
    and (((.config.env.IDLE_GAP_FILL_BATCH_PASSES | tonumber?) // 0) > 0)
    and (((.config.env.IDLE_GAP_FILL_BATCH_PASSES | tonumber?) // 0) <= 8)
    and (((.config.env.IDLE_GAP_FILL_MAX_NEW_VIDEOS | tonumber?) // 0) > 0)
    and (.config.env.RAW_PENDING_PUBLICATION_FILE == "/var/lib/ytsearch/raw-publication.pending")
    and ((.config.files // []) | length) == 4
    and (([.config.files[] | select(.guest_path == "/run/secrets/youtube-api-key.txt")] | length) == 1)
    and (([.config.files[] | select(
      .guest_path == "/run/secrets/youtube-api-key.txt"
      and .secret_name == "YOUTUBE_API_KEY_FILE_B64"
      and (has("raw_value") | not)
    )] | length) == 1)
    and (([.config.files[] | select(.guest_path == "/run/secrets/yt-dlp-cookies.txt")] | length) == 1)
    and (([.config.files[] | select(
      .guest_path == "/run/secrets/yt-dlp-cookies.txt"
      and .secret_name == "YT_DLP_COOKIES_FILE_B64"
      and (has("raw_value") | not)
    )] | length) == 1)
    and (.config.init.entrypoint == ["/bin/sh"])
    and (.config.init.cmd == ["/usr/local/bin/cloud-index-queue"])
    and (((.config.init.exec // []) | length) == 0)
    and ((.config.restart.policy // "no") == "no")
' >/dev/null; then
  echo "The raw worker does not satisfy the on-demand queue invariant." >&2
  exit 1
fi

verify_embedded_file() {
  guest_path="$1"
  expected_file="$2"
  encoded="$(printf '%s' "$machines" | jq -er --arg guest_path "$guest_path" '
    first(.[] | select(
      .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
      and .config.env.QUEUE_TARGET_BACKEND == "raw"
    ))
    | first(.config.files[] | select(.guest_path == $guest_path))
    | .raw_value
  ')" || {
    echo "The raw worker is missing embedded file $guest_path." >&2
    exit 1
  }
  actual="$(mktemp)"
  trap 'rm -f "$actual"' EXIT HUP INT TERM
  if ! printf '%s' "$encoded" | base64 -d > "$actual" 2>/dev/null \
    || ! cmp -s "$expected_file" "$actual"; then
    echo "The raw worker has stale embedded file $guest_path." >&2
    exit 1
  fi
  rm -f "$actual"
  trap - EXIT HUP INT TERM
}

verify_embedded_file /usr/local/bin/cloud-index-queue "$EXPECTED_ENTRYPOINT_FILE"
verify_embedded_file /tmp/daily-indexed-channels.txt "$EXPECTED_CREATOR_LIST_FILE"
