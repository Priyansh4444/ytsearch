#!/usr/bin/env sh
set -eu

HANDLE="${1:-}"
BATCH_SIZE="${BATCH_SIZE:-500}"
REPLACE="${REPLACE:-0}"

if [ -z "$HANDLE" ]; then
  echo "Usage: $0 @handle" >&2
  exit 1
fi

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ] || [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
  echo "Missing TYPESENSE_SELFHOST_API_URL or TYPESENSE_SELFHOST_API_KEY" >&2
  exit 1
fi

replace_arg=""
if [ "$REPLACE" = "1" ]; then
  replace_arg="--replace"
fi

# shellcheck disable=SC2086
TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- \
  copy-youtuber-chunks "$HANDLE" \
  --source-backend meilisearch \
  --target-backend typesense \
  --batch-size "$BATCH_SIZE" \
  $replace_arg
