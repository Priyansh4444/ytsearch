#!/usr/bin/env sh
set -eu

# Incremental Meilisearch indexing: add only videos that are not already indexed.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ "$#" -lt 1 ]; then
  echo "Usage: scripts/index-youtuber-meili-update.sh @handle [extra args]" >&2
  exit 1
fi

handle="$1"
shift

exec cargo run --release -p youtube_cli -- \
  index-youtuber "$handle" \
  --backend meilisearch \
  --mode update \
  "$@"
