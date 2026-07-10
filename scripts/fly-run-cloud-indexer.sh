#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"
VOLUME_NAME="${INDEXER_VOLUME_NAME:-ytsearch_indexer_data}"
VOLUME_SIZE_GB="${INDEXER_VOLUME_SIZE_GB:-10}"
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

flyctl apps create "$APP_NAME" --org "$FLY_ORG" >/dev/null 2>&1 || true

if ! flyctl volumes list --app "$APP_NAME" | grep -q "$VOLUME_NAME"; then
  flyctl volumes create "$VOLUME_NAME" \
    --app "$APP_NAME" \
    --region "$FLY_REGION" \
    --size "$VOLUME_SIZE_GB" \
    --yes
fi

MACHINE_NAME="${INDEXER_MACHINE_NAME:-ytsearch-index-$(date +%Y%m%d%H%M%S)}"

cookie_secret_args=""
if flyctl secrets list --app "$APP_NAME" | grep -q 'YT_DLP_COOKIES_FILE_B64'; then
  cookie_secret_args="--file-secret /run/secrets/yt-dlp-cookies.txt=YT_DLP_COOKIES_FILE_B64 --env YT_DLP_COOKIES_FILE=/run/secrets/yt-dlp-cookies.txt"
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
  --env "LIST_FILE=/tmp/indexed-channels.txt" \
  --env "TRANSCRIPT_CACHE_DIR=$transcript_cache_dir" \
  --env "BACKENDS=${BACKENDS:-meilisearch typesense-fly}" \
  --env "MODE=${MODE:-update}" \
  --env "JOBS=${JOBS:-3}" \
  --env "TYPESENSE_JOBS=${TYPESENSE_JOBS:-3}" \
  --env "MEILI_JOBS=${MEILI_JOBS:-2}" \
  --env "EXISTING_TAIL=${EXISTING_TAIL:-0}" \
  --env "VIDEO_TIMEOUT_SECONDS=${VIDEO_TIMEOUT_SECONDS:-300}" \
  --env "MEILI_WAIT_FOR_TASKS=${MEILI_WAIT_FOR_TASKS:-1}" \
  --env "RUST_LOG=${RUST_LOG:-youtube_cli=info,youtube_sources=info,youtube_ytdlp=warn}" \
  ${image_ref:-.} \
  /bin/sh /usr/local/bin/cloud-index-all "$@"
