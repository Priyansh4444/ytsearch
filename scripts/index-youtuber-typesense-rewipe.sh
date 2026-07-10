#!/usr/bin/env sh
set -eu

# Full Typesense reindex: wipe existing transcript chunks before importing.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ "$#" -lt 1 ]; then
  echo "Usage: scripts/index-youtuber-typesense-rewipe.sh @handle [extra args]" >&2
  exit 1
fi

if [ "${ALLOW_DESTRUCTIVE_REWIPE:-0}" != "1" ]; then
  echo "Refusing destructive rewipe. Verify a recoverable source, then set ALLOW_DESTRUCTIVE_REWIPE=1." >&2
  exit 1
fi

handle="$1"
shift

exec cargo run --release -p youtube_cli -- \
  index-youtuber "$handle" \
  --backend typesense \
  --mode rewipe \
  --allow-destructive-rewipe \
  "$@"
