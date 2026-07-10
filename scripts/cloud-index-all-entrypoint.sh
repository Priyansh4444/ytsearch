#!/usr/bin/env sh
set -eu

LIST_FILE="${LIST_FILE:-/tmp/indexed-channels.txt}"
BACKENDS="${BACKENDS:-meilisearch typesense-fly}"
MODE="${MODE:-update}"
JOBS="${JOBS:-2}"
TYPESENSE_JOBS="${TYPESENSE_JOBS:-3}"
MEILI_JOBS="${MEILI_JOBS:-2}"
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"
VIDEO_TIMEOUT_SECONDS="${VIDEO_TIMEOUT_SECONDS:-300}"
EXISTING_TAIL="${EXISTING_TAIL:-0}"
TRANSCRIPT_CACHE_DIR="${TRANSCRIPT_CACHE_DIR:-/data/transcript-cache}"
MEILI_WAIT_FOR_TASKS="${MEILI_WAIT_FOR_TASKS:-1}"

export MEILI_WAIT_FOR_TASKS
mkdir -p "$TRANSCRIPT_CACHE_DIR"

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

  echo "cloud-indexer: refreshing counts for $label"
  ytsearch refresh-youtuber-counts-list "$LIST_FILE" \
    --backend "$backend" \
    --collection "$COLLECTION"
}

if has_backend typesense-fly; then
  if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
    echo "cloud-indexer: skipping typesense-fly; missing TYPESENSE_SELFHOST_API_URL or TYPESENSE_SELFHOST_API_KEY" >&2
  else
    export TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL"
    export TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
    export TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY"
    index_backend typesense-fly typesense "${TYPESENSE_JOBS:-$JOBS}"
  fi
fi

if has_backend meilisearch; then
  index_backend meilisearch meilisearch "${MEILI_JOBS:-$JOBS}"
fi

echo "cloud-indexer: done"
