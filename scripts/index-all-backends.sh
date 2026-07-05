#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
if [ "$#" -gt 0 ]; then
  shift
fi

MODE="${MODE:-update}"
JOBS="${JOBS:-4}"
if [ "${BACKENDS+x}" != "x" ]; then
  BACKENDS="meilisearch typesense-cloud typesense-fly"
fi
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

COLLECTION="${YTSEARCH_TRANSCRIPTS_INDEX:-${MEILI_TRANSCRIPTS_INDEX:-${TYPESENSE_COLLECTION:-$COLLECTION}}}"

if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi

run_index() {
  label="$1"
  backend="$2"
  shift 2

  echo "Indexing $label: mode=$MODE jobs=$JOBS list=$LIST_FILE"
  cargo run --release -p youtube_cli -- \
    index-youtubers "$LIST_FILE" \
    --backend "$backend" \
    --mode "$MODE" \
    --collection "$COLLECTION" \
    --jobs "$JOBS" \
    "$@"
}

run_refresh() {
  label="$1"
  backend="$2"

  echo "Refreshing profile counts for $label"
  cargo run --release -p youtube_cli -- \
    refresh-youtubers "$LIST_FILE" \
    --backend "$backend"
}

run_index_typesense_fly() {
  echo "Indexing typesense-fly: mode=$MODE jobs=$JOBS list=$LIST_FILE"
  TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  cargo run --release -p youtube_cli -- \
    index-youtubers "$LIST_FILE" \
    --backend typesense \
    --mode "$MODE" \
    --collection "$COLLECTION" \
    --jobs "$JOBS" \
    "$@"
}

run_refresh_typesense_fly() {
  echo "Refreshing profile counts for typesense-fly"
  TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  cargo run --release -p youtube_cli -- \
    refresh-youtubers "$LIST_FILE" \
    --backend typesense
}

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

if has_backend meilisearch; then
  run_index meilisearch meilisearch "$@"
  run_refresh meilisearch meilisearch
fi

if has_backend typesense-cloud; then
  run_index typesense-cloud typesense "$@"
  run_refresh typesense-cloud typesense
fi

if has_backend typesense-fly; then
  if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
    echo "Skipping typesense-fly: missing TYPESENSE_SELFHOST_API_KEY or TYPESENSE_SELFHOST_API_URL" >&2
  else
    run_index_typesense_fly "$@"
    run_refresh_typesense_fly
  fi
fi
