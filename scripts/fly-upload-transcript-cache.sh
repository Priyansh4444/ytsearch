#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer}"
FLY_REGION="${FLY_REGION:-sjc}"
VOLUME_NAME="${INDEXER_VOLUME_NAME:-ytsearch_indexer_data}"
LOCAL_CACHE_DIR="${1:-data/transcript-cache}"

if [ -z "${FLY_ORG:-}" ]; then
  echo "Set FLY_ORG to the Fly.io organization that owns the indexer app." >&2
  exit 1
fi

if [ ! -d "$LOCAL_CACHE_DIR" ]; then
  echo "Transcript cache directory not found: $LOCAL_CACHE_DIR" >&2
  exit 1
fi

archive="/tmp/ytsearch-transcript-cache.$$.tar.gz"
cleanup() {
  rm -f "$archive"
}
trap cleanup EXIT

tar -C "$LOCAL_CACHE_DIR" -czf "$archive" .

if ! flyctl volumes list --app "$APP_NAME" | grep -q "$VOLUME_NAME"; then
  flyctl volumes create "$VOLUME_NAME" \
    --app "$APP_NAME" \
    --region "$FLY_REGION" \
    --size "${INDEXER_VOLUME_SIZE_GB:-10}" \
    --yes
fi

flyctl machine run debian:bookworm-slim \
  --app "$APP_NAME" \
  --org "$FLY_ORG" \
  --region "$FLY_REGION" \
  --rm \
  --restart no \
  --volume "$VOLUME_NAME:/data" \
  --file-local "/tmp/transcript-cache.tar.gz=$archive" \
  --vm-cpu-kind shared \
  --vm-cpus 1 \
  --vm-memory 512 \
  -- /bin/sh -c 'mkdir -p /data/transcript-cache && tar -xzf /tmp/transcript-cache.tar.gz -C /data/transcript-cache && find /data/transcript-cache -type f | wc -l'
