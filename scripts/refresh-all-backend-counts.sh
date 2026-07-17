#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
BACKENDS="${BACKENDS:-typesense-fly}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi

has_backend() {
  backend="$1"
  case " $BACKENDS " in
    *" $backend "*) return 0 ;;
    *) return 1 ;;
  esac
}

if has_backend typesense-fly; then
  if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
    echo "Skipping typesense-fly: missing TYPESENSE_SELFHOST_API_KEY or TYPESENSE_SELFHOST_API_URL" >&2
  else
    TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
    TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
    TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
    cargo run --release -p youtube_cli -- \
      refresh-youtuber-counts-list "$LIST_FILE" --backend typesense
  fi
fi
