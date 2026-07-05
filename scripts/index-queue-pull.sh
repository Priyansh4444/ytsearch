#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUEUE_URL="${INDEX_QUEUE_API_BASE_URL:-}"
LIMIT="${LIMIT:-100}"

if [ -z "$QUEUE_URL" ]; then
  echo "Missing required environment variable: INDEX_QUEUE_API_BASE_URL" >&2
  exit 1
fi

if [ -z "${YTSEARCH_INDEX_QUEUE_TOKEN:-}" ]; then
  echo "Missing required environment variable: YTSEARCH_INDEX_QUEUE_TOKEN" >&2
  exit 1
fi

curl -sS \
  -H "Authorization: Bearer $YTSEARCH_INDEX_QUEUE_TOKEN" \
  "$QUEUE_URL/api/index-requests?limit=$LIMIT"
