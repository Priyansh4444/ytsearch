#!/usr/bin/env sh
set -eu

# Incremental Typesense indexing: add only videos that are not already indexed.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ "$#" -lt 1 ]; then
  echo "Usage: scripts/index-youtuber-typesense-update.sh @handle [extra args]" >&2
  exit 1
fi

handle="$1"
shift

exec cargo run --release -p youtube_cli -- \
  index-youtuber "$handle" \
  --backend typesense \
  --mode update \
  "$@"
