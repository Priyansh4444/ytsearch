#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
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

# Volumes bill while idle, so never create one implicitly.
if ! flyctl volumes list --app "$APP_NAME" | grep -qw "$VOLUME_NAME"; then
  echo "Volume $VOLUME_NAME does not exist in $APP_NAME; create it or set INDEXER_VOLUME_NAME." >&2
  exit 1
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
