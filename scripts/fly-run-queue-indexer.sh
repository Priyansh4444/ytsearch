#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

raw_reader_target="$(jq -er '
  .resources.raw_reader as $reader |
  select($reader.machine_id | test("^[0-9a-f]{14}$")) |
  "http://" + $reader.machine_id + ".vm." + $reader.app + ".internal:8108"
' "$(dirname "$0")/../config/fly-production-resources.json")"
if [ -n "${RAW_SEARCH_API_URL:-}" ] && [ "$RAW_SEARCH_API_URL" != "$raw_reader_target" ]; then
  echo "RAW_SEARCH_API_URL must target the exact production reader Machine." >&2
  exit 1
fi

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
FLY_REGION="${FLY_REGION:-sjc}"
QUEUE_TARGET_BACKEND="${QUEUE_TARGET_BACKEND:-}"
ON_DEMAND="${ON_DEMAND:-0}"
VOLUME_SIZE_GB="${INDEXER_VOLUME_SIZE_GB:-10}"
DAILY_UPDATE_LIST_SOURCE="${DAILY_UPDATE_LIST_SOURCE:-scripts/indexed-channels.txt}"
INDEXER_DATA_DIRECTORY="${INDEXER_DATA_DIRECTORY:-/var/lib/ytsearch}"

if [ -z "${FLY_ORG:-}" ]; then
  echo "Set FLY_ORG to the Fly.io organization that should own the indexer." >&2
  exit 1
fi

case "$QUEUE_TARGET_BACKEND" in
  raw)
    EXPECTED_BACKENDS="raw"
    DEFAULT_MACHINE_NAME="ytsearch-queue-raw"
    # Fly volume names are limited to 30 characters. Keep this aligned with the
    # Raw worker volume created by the multi-indexer launcher.
    DEFAULT_VOLUME_NAME="ytsearch_indexer_data_raw"
    ;;
  *)
    echo "QUEUE_TARGET_BACKEND must be raw." >&2
    echo "Use scripts/fly-run-queue-indexers.sh to start the active Raw worker." >&2
    exit 1
    ;;
esac

BACKENDS="${BACKENDS:-$EXPECTED_BACKENDS}"
if [ "$BACKENDS" != "$EXPECTED_BACKENDS" ]; then
  echo "QUEUE_TARGET_BACKEND=$QUEUE_TARGET_BACKEND requires BACKENDS=$EXPECTED_BACKENDS, got: $BACKENDS" >&2
  exit 1
fi

VOLUME_NAME="${INDEXER_VOLUME_NAME:-$DEFAULT_VOLUME_NAME}"

flyctl apps create "$APP_NAME" --org "$FLY_ORG" >/dev/null 2>&1 || true

if ! flyctl volumes list --app "$APP_NAME" | grep -q "$VOLUME_NAME"; then
  flyctl volumes create "$VOLUME_NAME" \
    --app "$APP_NAME" \
    --region "$FLY_REGION" \
    --size "$VOLUME_SIZE_GB" \
    --yes
fi

QUEUE_DAEMON="${QUEUE_DAEMON:-0}"
if [ "$QUEUE_DAEMON" = "1" ]; then
  MACHINE_NAME="${INDEXER_MACHINE_NAME:-$DEFAULT_MACHINE_NAME}"
else
  MACHINE_NAME="${INDEXER_MACHINE_NAME:-${DEFAULT_MACHINE_NAME}-once-$(date +%Y%m%d%H%M%S)}"
fi

daily_update_enabled="${DAILY_UPDATE_ENABLED:-}"
if [ -z "$daily_update_enabled" ]; then
  if [ "$QUEUE_DAEMON" = "1" ] && [ "$ON_DEMAND" != "1" ]; then
    daily_update_enabled="1"
  else
    daily_update_enabled="0"
  fi
fi

daily_update_interval_seconds="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}"
daily_update_initial_delay_seconds="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-}"
if [ -z "$daily_update_initial_delay_seconds" ]; then
  # Start a complete update after deployment, then repeat on the normal cadence.
  daily_update_initial_delay_seconds="0"
fi
case "$daily_update_interval_seconds" in
  ''|*[!0-9]*)
    echo "DAILY_UPDATE_INTERVAL_SECONDS must be an integer" >&2
    exit 1
    ;;
esac
if [ "$daily_update_enabled" = "1" ] && [ "$daily_update_interval_seconds" -lt 60 ]; then
  echo "DAILY_UPDATE_INTERVAL_SECONDS must be at least 60" >&2
  exit 1
fi

restart_policy="no"
remove_arg="--rm"
if [ "$QUEUE_DAEMON" = "1" ]; then
  restart_policy="always"
  remove_arg=""
fi
if [ "$ON_DEMAND" = "1" ]; then
  restart_policy="no"
  remove_arg=""
fi

indexer_cpu_kind="${INDEXER_CPU_KIND:-shared}"
indexer_cpus="${INDEXER_CPUS:-2}"
idle_exit_after_seconds="${IDLE_EXIT_AFTER_SECONDS:-0}"
if [ "$ON_DEMAND" = "1" ]; then
  indexer_cpu_kind="${INDEXER_CPU_KIND:-shared}"
  indexer_cpus="${INDEXER_CPUS:-2}"
  idle_exit_after_seconds="${IDLE_EXIT_AFTER_SECONDS:-60}"
fi

cookie_secret_args=""
if flyctl secrets list --app "$APP_NAME" | grep -q 'YT_DLP_COOKIES_FILE_B64'; then
  cookie_secret_args="--file-secret /run/secrets/yt-dlp-cookies.txt=YT_DLP_COOKIES_FILE_B64 --env YT_DLP_COOKIES_FILE=/run/secrets/yt-dlp-cookies.txt"
fi

youtube_secret_args=""
if flyctl secrets list --app "$APP_NAME" | grep -q 'YOUTUBE_API_KEY_FILE_B64'; then
  youtube_secret_args="--file-secret /run/secrets/youtube-api-key.txt=YOUTUBE_API_KEY_FILE_B64 --env YOUTUBE_API_KEY_FILE=/run/secrets/youtube-api-key.txt"
fi

daily_update_file_args=""
if [ -f "$DAILY_UPDATE_LIST_SOURCE" ]; then
  daily_update_file_args="--file-local /tmp/daily-indexed-channels.txt=$DAILY_UPDATE_LIST_SOURCE --env DAILY_UPDATE_LIST_FILE=/tmp/daily-indexed-channels.txt --env QUEUE_PRIORITY_LIST_FILE=/tmp/daily-indexed-channels.txt"
fi

queue_limit="${LIMIT:-10}"
queue_one_request_per_pass="${QUEUE_ONE_REQUEST_PER_PASS:-0}"

echo "Starting queue indexer machine: app=$APP_NAME name=$MACHINE_NAME region=$FLY_REGION"
if [ "$QUEUE_DAEMON" = "1" ]; then
  echo "Queue daemon mode enabled; machine will poll until stopped."
  echo "Daily update enabled=$daily_update_enabled initial_delay=${daily_update_initial_delay_seconds}s interval=${daily_update_interval_seconds}s"
fi
if [ "$ON_DEMAND" = "1" ]; then
  echo "On-demand mode enabled; machine exits after an idle window and is restarted by the queue wake Worker."
fi
echo "Follow logs with: flyctl logs --app $APP_NAME"

build_args=""
image_ref="${INDEXER_IMAGE:-}"
if [ -n "$image_ref" ]; then
  echo "Using existing image: $image_ref"
elif [ "${INDEXER_BUILD_DEPOT:-1}" = "1" ]; then
  build_args="--build-depot"
fi

# shellcheck disable=SC2086
exec flyctl machine run \
  --app "$APP_NAME" \
  --org "$FLY_ORG" \
  --region "$FLY_REGION" \
  --name "$MACHINE_NAME" \
  --detach \
  $remove_arg \
  --restart "$restart_policy" \
  $build_args \
  --vm-cpu-kind "$indexer_cpu_kind" \
  --vm-cpus "$indexer_cpus" \
  --vm-memory "${INDEXER_MEMORY_MB:-2048}" \
  --rootfs-size "${INDEXER_ROOTFS_SIZE_GB:-10}" \
  --swap-size "${INDEXER_SWAP_MB:-512}" \
  --volume "$VOLUME_NAME:$INDEXER_DATA_DIRECTORY" \
  --file-local "/usr/local/bin/cloud-index-queue=$(dirname "$0")/cloud-index-queue-entrypoint.sh" \
  $cookie_secret_args \
  $youtube_secret_args \
  $daily_update_file_args \
  --env "YTSEARCH_MACHINE_ROLE=queue-indexer" \
  --env "BACKENDS=$BACKENDS" \
  --env "QUEUE_TARGET_BACKEND=$QUEUE_TARGET_BACKEND" \
  --env "RAW_SEARCH_API_URL=$raw_reader_target" \
  --env "RAW_PUBLICATION_TIMEOUT_SECONDS=${RAW_PUBLICATION_TIMEOUT_SECONDS:-3600}" \
  --env "RAW_PUBLICATION_WORK_DIRECTORY=${RAW_PUBLICATION_WORK_DIRECTORY:-$INDEXER_DATA_DIRECTORY/raw-generation-work}" \
  --env "RAW_PUBLICATION_MIN_FREE_BYTES=${RAW_PUBLICATION_MIN_FREE_BYTES:-8000000000}" \
  --env "RAW_PENDING_PUBLICATION_FILE=${RAW_PENDING_PUBLICATION_FILE:-$INDEXER_DATA_DIRECTORY/raw-publication.pending}" \
  --env "JOBS=${JOBS:-1}" \
  --env "RAW_JOBS=${RAW_JOBS:-1}" \
  --env "LIMIT=$queue_limit" \
  --env "QUEUE_ONE_REQUEST_PER_PASS=$queue_one_request_per_pass" \
  --env "MAX_VIDEOS_PER_PASS=${MAX_VIDEOS_PER_PASS:-0}" \
  --env "QUEUE_LOOP=${QUEUE_LOOP:-$QUEUE_DAEMON}" \
  --env "QUEUE_POLL_INTERVAL_SECONDS=${QUEUE_POLL_INTERVAL_SECONDS:-30}" \
  --env "TRANSCRIPT_CACHE_DIR=$INDEXER_DATA_DIRECTORY/transcript-cache" \
  --env "QUEUE_RETRY_STATE_FILE=${QUEUE_RETRY_STATE_FILE:-$INDEXER_DATA_DIRECTORY/queue-retries.tsv}" \
  --env "QUEUE_ACK_RETRY_SECONDS=${QUEUE_ACK_RETRY_SECONDS:-300}" \
  --env "IDLE_EXIT_AFTER_SECONDS=$idle_exit_after_seconds" \
  --env "IDLE_GAP_FILL_ENABLED=${IDLE_GAP_FILL_ENABLED:-0}" \
  --env "IDLE_GAP_FILL_STATE_FILE=${IDLE_GAP_FILL_STATE_FILE:-$INDEXER_DATA_DIRECTORY/idle-gap-fill.cursor}" \
  --env "IDLE_GAP_FILL_MAX_PASSES=${IDLE_GAP_FILL_MAX_PASSES:-1}" \
  --env "IDLE_GAP_FILL_BATCH_PASSES=${IDLE_GAP_FILL_BATCH_PASSES:-4}" \
  --env "IDLE_GAP_FILL_MAX_NEW_VIDEOS=${IDLE_GAP_FILL_MAX_NEW_VIDEOS:-10}" \
  --env "ACK_FAILED_REQUESTS=${ACK_FAILED_REQUESTS:-0}" \
  --env "DAILY_UPDATE_ENABLED=$daily_update_enabled" \
  --env "DAILY_UPDATE_STATE_FILE=${DAILY_UPDATE_STATE_FILE:-$INDEXER_DATA_DIRECTORY/daily-update.last-completed}" \
  --env "DAILY_UPDATE_INTERVAL_SECONDS=$daily_update_interval_seconds" \
  --env "DAILY_UPDATE_INITIAL_DELAY_SECONDS=$daily_update_initial_delay_seconds" \
  --env "EXISTING_TAIL=${EXISTING_TAIL:-0}" \
  --env "VIDEO_TIMEOUT_SECONDS=${VIDEO_TIMEOUT_SECONDS:-600}" \
  --env "TRANSCRIPT_FAILURE_RETRY_SECONDS=${TRANSCRIPT_FAILURE_RETRY_SECONDS:-21600}" \
  --env "YT_DLP_CAPTION_FETCH_ATTEMPTS=${YT_DLP_CAPTION_FETCH_ATTEMPTS:-6}" \
  --env "YT_DLP_CAPTION_MIN_INTERVAL_MS=${YT_DLP_CAPTION_MIN_INTERVAL_MS:-1500}" \
  --env "YT_DLP_RATE_LIMIT_RETRY_BASE_SECONDS=${YT_DLP_RATE_LIMIT_RETRY_BASE_SECONDS:-5}" \
  --env "RUST_LOG=${RUST_LOG:-youtube_cli=info,youtube_sources=info,youtube_ytdlp=warn}" \
  ${image_ref:-.} \
  /bin/sh /usr/local/bin/cloud-index-queue "$@"
