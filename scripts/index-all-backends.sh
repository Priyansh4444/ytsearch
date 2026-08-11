#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
if [ "$#" -gt 0 ]; then
  shift
fi

MODE="${MODE:-update}"
# The active engine publishes one immutable Raw generation. Keep local bulk runs
# serialized unless an operator has provisioned and measured separate ingest
# capacity.
JOBS="${JOBS:-1}"
CHANNEL_JOBS="${CHANNEL_JOBS:-1}"
if [ "${BACKENDS+x}" != "x" ]; then
  BACKENDS="raw"
fi
COLLECTION="${COLLECTION:-youtube_transcript_chunks}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

COLLECTION="${YTSEARCH_TRANSCRIPTS_INDEX:-${RAW_SEARCH_COLLECTION:-$COLLECTION}}"
REWIPE_ACK_ARG=""

if [ "$MODE" = "rewipe" ]; then
  if [ "${ALLOW_DESTRUCTIVE_REWIPE:-0}" != "1" ]; then
    echo "Refusing destructive rewipe. Verify a recoverable source, then set ALLOW_DESTRUCTIVE_REWIPE=1." >&2
    exit 1
  fi
  REWIPE_ACK_ARG="--allow-destructive-rewipe"
fi

if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi

run_index_raw() {
  echo "Indexing Raw: mode=$MODE jobs=$JOBS list=$LIST_FILE"
  cargo run --release -p youtube_cli -- \
    index-youtubers "$LIST_FILE" \
    --backend raw \
    --mode "$MODE" \
    --collection "$COLLECTION" \
    --jobs "$JOBS" \
    --channel-jobs "$CHANNEL_JOBS" \
    $REWIPE_ACK_ARG \
    "$@"
}

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

if has_backend raw; then
  if [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ] || [ -z "${RAW_SEARCH_API_URL:-}" ]; then
    echo "Raw indexing unavailable: missing RAW_SEARCH_ADMIN_API_KEY or RAW_SEARCH_API_URL" >&2
    exit 1
  else
    run_index_raw "$@"
  fi
fi
