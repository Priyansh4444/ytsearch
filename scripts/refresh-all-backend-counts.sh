#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
BACKENDS="${BACKENDS:-raw}"

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

if has_backend raw; then
  if [ -z "${RAW_SEARCH_ADMIN_API_KEY:-}" ] || [ -z "${RAW_SEARCH_API_URL:-}" ]; then
    echo "Raw count refresh unavailable: missing RAW_SEARCH_ADMIN_API_KEY or RAW_SEARCH_API_URL" >&2
    exit 1
  else
    cargo run --release -p youtube_cli -- \
      refresh-youtuber-counts-list "$LIST_FILE" --backend raw
  fi
fi
