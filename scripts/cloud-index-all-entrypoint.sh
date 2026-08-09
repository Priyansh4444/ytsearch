#!/usr/bin/env sh
set -eu

LIST_FILE="${LIST_FILE:-/tmp/indexed-channels.txt}"
BACKENDS="${BACKENDS:-raw}"
MODE="${MODE:-update}"
JOBS="${JOBS:-2}"
RAW_JOBS="${RAW_JOBS:-3}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"
VIDEO_TIMEOUT_SECONDS="${VIDEO_TIMEOUT_SECONDS:-300}"
EXISTING_TAIL="${EXISTING_TAIL:-0}"
TRANSCRIPT_CACHE_DIR="${TRANSCRIPT_CACHE_DIR:-/data/transcript-cache}"
mkdir -p "$TRANSCRIPT_CACHE_DIR"

if [ -n "${YOUTUBE_API_KEY_FILE:-}" ] && [ -f "$YOUTUBE_API_KEY_FILE" ]; then
  YOUTUBE_API_KEY="$(cat "$YOUTUBE_API_KEY_FILE")"
  export YOUTUBE_API_KEY
fi

REWIPE_ACK_ARG=""
if [ "$MODE" = "rewipe" ]; then
  if [ "${ALLOW_DESTRUCTIVE_REWIPE:-0}" != "1" ]; then
    echo "cloud-indexer: refusing destructive rewipe without ALLOW_DESTRUCTIVE_REWIPE=1" >&2
    exit 1
  fi
  REWIPE_ACK_ARG="--allow-destructive-rewipe"
fi

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

index_backend() {
  label="$1"
  backend="$2"
  jobs="$3"
  shift 3

  echo "cloud-indexer: indexing $label mode=$MODE jobs=$jobs list=$LIST_FILE"
  ytsearch index-youtubers "$LIST_FILE" \
    --backend "$backend" \
    --mode "$MODE" \
    --collection "$COLLECTION" \
    --jobs "$jobs" \
    --video-timeout-seconds "$VIDEO_TIMEOUT_SECONDS" \
    --stop-after-existing "$EXISTING_TAIL" \
    --transcript-cache-dir "$TRANSCRIPT_CACHE_DIR" \
    $REWIPE_ACK_ARG \
    "$@"
}

if has_backend raw; then
  if [ -z "${RAW_SEARCH_API_URL:-}" ] || [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ]; then
    echo "cloud-indexer: Raw unavailable; missing RAW_SEARCH_API_URL or RAW_SEARCH_ADMIN_API_KEY" >&2
    exit 1
  else
    index_backend raw raw "${RAW_JOBS:-$JOBS}"
  fi
fi

echo "cloud-indexer: done"
