#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"

if [ "$#" -gt 0 ]; then
  shift
fi

if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
FLY_REGION="${FLY_REGION:-sjc}"
VOLUME_NAME="${INDEXER_VOLUME_NAME:-ytsearch_indexer_data}"

if [ -z "${FLY_ORG:-}" ]; then
  echo "Set FLY_ORG to the Fly.io organization that should own the indexer." >&2
  exit 1
fi

# Volumes bill while idle, so never create one implicitly; ONE_OFF_NO_VOLUME=1
# runs with an ephemeral transcript cache instead.
if [ "${ONE_OFF_NO_VOLUME:-0}" != "1" ] \
  && ! flyctl volumes list --app "$APP_NAME" | grep -qw "$VOLUME_NAME"; then
  echo "Volume $VOLUME_NAME does not exist in $APP_NAME; set INDEXER_VOLUME_NAME or ONE_OFF_NO_VOLUME=1." >&2
  exit 1
fi

MACHINE_NAME="${INDEXER_MACHINE_NAME:-ytsearch-index-$(date +%Y%m%d%H%M%S)}"

cookie_secret_args=""
if flyctl secrets list --app "$APP_NAME" | grep -q 'YT_DLP_COOKIES_FILE_B64'; then
  cookie_secret_args="--file-secret /run/secrets/yt-dlp-cookies.txt=YT_DLP_COOKIES_FILE_B64 --env YT_DLP_COOKIES_FILE=/run/secrets/yt-dlp-cookies.txt"
fi

youtube_secret_args=""
if flyctl secrets list --app "$APP_NAME" | grep -q 'YOUTUBE_API_KEY_FILE_B64'; then
  youtube_secret_args="--file-secret /run/secrets/youtube-api-key.txt=YOUTUBE_API_KEY_FILE_B64 --env YOUTUBE_API_KEY_FILE=/run/secrets/youtube-api-key.txt"
fi

echo "Starting cloud indexer machine: app=$APP_NAME name=$MACHINE_NAME region=$FLY_REGION"
echo "Follow logs with: flyctl logs --app $APP_NAME"

build_args=""
image_ref="${INDEXER_IMAGE:-}"
if [ -n "$image_ref" ]; then
  echo "Using existing image: $image_ref"
elif [ "${INDEXER_BUILD_DEPOT:-1}" = "1" ]; then
  build_args="--build-depot"
fi

volume_args="--volume $VOLUME_NAME:/data"
transcript_cache_dir="/data/transcript-cache"
if [ "${ONE_OFF_NO_VOLUME:-0}" = "1" ]; then
  volume_args=""
  transcript_cache_dir="/tmp/transcript-cache"
  echo "One-off mode without Fly volume; transcript cache is ephemeral at $transcript_cache_dir"
fi

# shellcheck disable=SC2086
exec flyctl machine run \
  --app "$APP_NAME" \
  --org "$FLY_ORG" \
  --region "$FLY_REGION" \
  --name "$MACHINE_NAME" \
  --detach \
  --rm \
  --restart no \
  $build_args \
  --vm-cpu-kind "${INDEXER_CPU_KIND:-shared}" \
  --vm-cpus "${INDEXER_CPUS:-2}" \
  --vm-memory "${INDEXER_MEMORY_MB:-4096}" \
  --rootfs-size "${INDEXER_ROOTFS_SIZE_GB:-10}" \
  --swap-size "${INDEXER_SWAP_MB:-1024}" \
  $volume_args \
  --file-local "/tmp/indexed-channels.txt=$LIST_FILE" \
  --file-local "/usr/local/bin/cloud-index-all=$(dirname "$0")/cloud-index-all-entrypoint.sh" \
  $cookie_secret_args \
  $youtube_secret_args \
  --env "LIST_FILE=/tmp/indexed-channels.txt" \
  --env "TRANSCRIPT_CACHE_DIR=$transcript_cache_dir" \
  --env "BACKENDS=${BACKENDS:-raw}" \
  --env "RAW_SEARCH_API_URL=${RAW_SEARCH_API_URL:-http://ytsearch-raw-production.internal:8108}" \
  --env "MODE=${MODE:-update}" \
  --env "JOBS=${JOBS:-1}" \
  --env "RAW_JOBS=${RAW_JOBS:-1}" \
  --env "CHANNEL_JOBS=${CHANNEL_JOBS:-1}" \
  --env "EXISTING_TAIL=${EXISTING_TAIL:-0}" \
  --env "VIDEO_TIMEOUT_SECONDS=${VIDEO_TIMEOUT_SECONDS:-300}" \
  --env "RUST_LOG=${RUST_LOG:-youtube_cli=info,youtube_sources=info,youtube_ytdlp=warn}" \
  ${image_ref:-.} \
  /bin/sh /usr/local/bin/cloud-index-all "$@"
