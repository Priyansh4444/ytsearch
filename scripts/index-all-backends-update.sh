#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
JOBS="${JOBS:-4}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

echo "Updating Meilisearch index"
cargo run --release -p youtube_cli -- \
  index-youtubers "$LIST_FILE" \
  --backend meilisearch \
  --mode update \
  --jobs "$JOBS"

echo "Updating Typesense Cloud index"
cargo run --release -p youtube_cli -- \
  index-youtubers "$LIST_FILE" \
  --backend typesense \
  --mode update \
  --jobs "$JOBS"

if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
  echo "Skipping self-hosted Typesense: missing TYPESENSE_SELFHOST_API_KEY" >&2
  exit 1
fi
if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
  echo "Missing required environment variable: TYPESENSE_SELFHOST_API_URL" >&2
  exit 1
fi

echo "Updating self-hosted Typesense on Fly index"
TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- \
  index-youtubers "$LIST_FILE" \
  --backend typesense \
  --mode update \
  --jobs "$JOBS"

echo "Refreshing profile counts in all three indexes"
cargo run --release -p youtube_cli -- \
  refresh-youtubers "$LIST_FILE" \
  --backend meilisearch

cargo run --release -p youtube_cli -- \
  refresh-youtubers "$LIST_FILE" \
  --backend typesense

TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
cargo run --release -p youtube_cli -- \
  refresh-youtubers "$LIST_FILE" \
  --backend typesense
