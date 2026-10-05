#!/usr/bin/env sh
set -eu

# Update every creator in the list (default scripts/indexed-channels.txt) in
# Raw. One CLI process coalesces every creator's durable mutations into one
# verified Raw generation. Existing video IDs and classified failure records
# make the command safe to resume after a Machine or network interruption.
# Keep JOBS=1 CHANNEL_JOBS=1 unless separate ingest capacity has been measured:
# one Raw writer publishes at a time.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
LIST_FILE="${1:-$SCRIPT_DIR/indexed-channels.txt}"
if [ "$#" -gt 0 ]; then
  shift
fi
JOBS="${JOBS:-1}"
CHANNEL_JOBS="${CHANNEL_JOBS:-1}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"

# shellcheck disable=SC1091
. "$SCRIPT_DIR/load-env.sh"

COLLECTION="${YTSEARCH_TRANSCRIPTS_INDEX:-${RAW_SEARCH_COLLECTION:-$COLLECTION}}"
if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi
if [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ] || [ -z "${RAW_SEARCH_API_URL:-}" ]; then
  echo "Raw indexing unavailable: missing RAW_SEARCH_ADMIN_API_KEY or RAW_SEARCH_API_URL" >&2
  exit 1
fi
if [ -n "${MAX_VIDEOS:-}" ]; then
  set -- --max-videos "$MAX_VIDEOS" "$@"
fi

echo "Indexing Raw: mode=update jobs=$JOBS list=$LIST_FILE"
exec cargo run --release -p youtube_cli -- \
  index-youtubers "$LIST_FILE" \
  --backend raw \
  --mode update \
  --collection "$COLLECTION" \
  --jobs "$JOBS" \
  --channel-jobs "$CHANNEL_JOBS" \
  --stop-after-existing "${STOP_AFTER_EXISTING:-${EXISTING_TAIL:-0}}" \
  "$@"
