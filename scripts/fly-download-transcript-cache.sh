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

if ! command -v jq >/dev/null 2>&1; then
  echo "This script needs jq to select the temporary export machine." >&2
  exit 1
fi

mkdir -p "$LOCAL_CACHE_DIR"

machine_name="ytsearch-cache-export-$(date +%Y%m%d%H%M%S)"
archive_remote="/tmp/transcript-cache.tar.gz"
archive_remote_tmp="${archive_remote}.tmp"
archive_ready="${archive_remote}.ready"
archive_local="/tmp/ytsearch-transcript-cache.$$.tar.gz"
extract_local="/tmp/ytsearch-transcript-cache-extract.$$"

cleanup() {
  rm -f "$archive_local"
  rm -rf "$extract_local"
  if [ -n "${machine_id:-}" ]; then
    flyctl machine destroy "$machine_id" --app "$APP_NAME" --force >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

flyctl machine run debian:bookworm-slim \
  --app "$APP_NAME" \
  --org "$FLY_ORG" \
  --region "$FLY_REGION" \
  --name "$machine_name" \
  --detach \
  --restart no \
  --volume "$VOLUME_NAME:/data" \
  --vm-cpu-kind shared \
  --vm-cpus 1 \
  --vm-memory 512 \
  -- /bin/sh -c "set -e; mkdir -p /data/transcript-cache; rm -f $archive_remote_tmp $archive_ready; tar -C /data/transcript-cache -czf $archive_remote_tmp .; mv $archive_remote_tmp $archive_remote; : >$archive_ready; sleep 300"

machine_id="$(flyctl machine list --app "$APP_NAME" --json | jq -r ".[] | select(.name == \"$machine_name\") | .id" | head -n 1)"
if [ -z "$machine_id" ]; then
  echo "Could not find export machine $machine_name" >&2
  exit 1
fi

flyctl ssh console --app "$APP_NAME" --machine "$machine_id" --command \
  "sh -lc 'attempt=0; while [ ! -f $archive_ready ]; do attempt=\$((attempt + 1)); [ \"\$attempt\" -lt 180 ] || exit 1; sleep 1; done'"

flyctl ssh sftp get "$archive_remote" "$archive_local" --app "$APP_NAME" --machine "$machine_id"
gzip -t "$archive_local"
mkdir -p "$extract_local"
tar -C "$extract_local" -xzf "$archive_local"
cp -Rn "$extract_local/." "$LOCAL_CACHE_DIR/"
find "$LOCAL_CACHE_DIR" -type f | wc -l
