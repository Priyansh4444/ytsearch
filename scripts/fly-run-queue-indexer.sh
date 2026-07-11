#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"
QUEUE_TARGET_BACKEND="${QUEUE_TARGET_BACKEND:-}"
VOLUME_SIZE_GB="${INDEXER_VOLUME_SIZE_GB:-10}"
DAILY_UPDATE_LIST_SOURCE="${DAILY_UPDATE_LIST_SOURCE:-scripts/indexed-channels.txt}"

case "$QUEUE_TARGET_BACKEND" in
  meilisearch)
    echo "Meilisearch is retired; no queue machine may target it." >&2
    exit 1
    ;;
  typesense-fly)
    EXPECTED_BACKENDS="typesense-fly"
    DEFAULT_MACHINE_NAME="ytsearch-queue-typesense"
    # Fly volume names are limited to 30 characters. Keep this aligned with the
    # Typesense worker volume created by the multi-indexer launcher.
    DEFAULT_VOLUME_NAME="ytsearch_indexer_data_ts"
    ;;
  *)
    echo "QUEUE_TARGET_BACKEND must be either meilisearch or typesense-fly." >&2
    echo "Use scripts/fly-run-queue-indexers.sh to start the active Typesense worker." >&2
    exit 1
    ;;
esac

BACKENDS="${BACKENDS:-$EXPECTED_BACKENDS}"
if [ "$BACKENDS" != "$EXPECTED_BACKENDS" ]; then
  echo "QUEUE_TARGET_BACKEND=$QUEUE_TARGET_BACKEND requires BACKENDS=$EXPECTED_BACKENDS, got: $BACKENDS" >&2
  exit 1
fi

VOLUME_NAME="${INDEXER_VOLUME_NAME:-$DEFAULT_VOLUME_NAME}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

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
  if [ "$QUEUE_DAEMON" = "1" ]; then
    daily_update_enabled="1"
  else
    daily_update_enabled="0"
  fi
fi

daily_update_interval_seconds="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}"
daily_update_initial_delay_seconds="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-}"
if [ -z "$daily_update_initial_delay_seconds" ]; then
  # Defer the first bulk daily pass so queue catch-up is not blocked on daemon restart.
  daily_update_initial_delay_seconds="86400"
fi

restart_policy="no"
remove_arg="--rm"
if [ "$QUEUE_DAEMON" = "1" ]; then
  restart_policy="always"
  remove_arg=""
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
if [ "$QUEUE_DAEMON" = "1" ]; then
  if [ "$queue_limit" = "10" ] && [ -z "${LIMIT+x}" ]; then
    queue_limit="1"
  fi
  if [ -z "${QUEUE_ONE_REQUEST_PER_PASS+x}" ]; then
    queue_one_request_per_pass="1"
  fi
fi

echo "Starting queue indexer machine: app=$APP_NAME name=$MACHINE_NAME region=$FLY_REGION"
if [ "$QUEUE_DAEMON" = "1" ]; then
  echo "Queue daemon mode enabled; machine will poll until stopped."
  echo "Daily update enabled=$daily_update_enabled initial_delay=${daily_update_initial_delay_seconds}s interval=${daily_update_interval_seconds}s"
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
  --vm-cpu-kind "${INDEXER_CPU_KIND:-shared}" \
  --vm-cpus "${INDEXER_CPUS:-2}" \
  --vm-memory "${INDEXER_MEMORY_MB:-4096}" \
  --rootfs-size "${INDEXER_ROOTFS_SIZE_GB:-10}" \
  --swap-size "${INDEXER_SWAP_MB:-1024}" \
  --volume "$VOLUME_NAME:/data" \
  --file-local "/usr/local/bin/cloud-index-queue=$(dirname "$0")/cloud-index-queue-entrypoint.sh" \
  $cookie_secret_args \
  $youtube_secret_args \
  $daily_update_file_args \
  --env "BACKENDS=$BACKENDS" \
  --env "QUEUE_TARGET_BACKEND=$QUEUE_TARGET_BACKEND" \
  --env "JOBS=${JOBS:-2}" \
  --env "TYPESENSE_JOBS=${TYPESENSE_JOBS:-3}" \
  --env "MEILI_JOBS=${MEILI_JOBS:-2}" \
  --env "LIMIT=$queue_limit" \
  --env "QUEUE_ONE_REQUEST_PER_PASS=$queue_one_request_per_pass" \
  --env "MAX_VIDEOS_PER_PASS=${MAX_VIDEOS_PER_PASS:-0}" \
  --env "QUEUE_LOOP=${QUEUE_LOOP:-$QUEUE_DAEMON}" \
  --env "QUEUE_POLL_INTERVAL_SECONDS=${QUEUE_POLL_INTERVAL_SECONDS:-30}" \
  --env "ACK_FAILED_REQUESTS=${ACK_FAILED_REQUESTS:-1}" \
  --env "DAILY_UPDATE_ENABLED=$daily_update_enabled" \
  --env "DAILY_UPDATE_INTERVAL_SECONDS=$daily_update_interval_seconds" \
  --env "DAILY_UPDATE_INITIAL_DELAY_SECONDS=$daily_update_initial_delay_seconds" \
  --env "EXISTING_TAIL=${EXISTING_TAIL:-0}" \
  --env "VIDEO_TIMEOUT_SECONDS=${VIDEO_TIMEOUT_SECONDS:-300}" \
  --env "MEILI_WAIT_FOR_TASKS=${MEILI_WAIT_FOR_TASKS:-1}" \
  --env "RUST_LOG=${RUST_LOG:-youtube_cli=info,youtube_sources=info,youtube_ytdlp=warn}" \
  ${image_ref:-.} \
  /bin/sh /usr/local/bin/cloud-index-queue "$@"
