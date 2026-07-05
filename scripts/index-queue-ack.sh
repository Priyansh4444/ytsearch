#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "Usage: scripts/index-queue-ack.sh REQUEST_ID" >&2
  exit 1
fi

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUEUE_URL="${INDEX_QUEUE_API_BASE_URL:-}"

if [ -z "$QUEUE_URL" ]; then
  echo "Missing required environment variable: INDEX_QUEUE_API_BASE_URL" >&2
  exit 1
fi

if [ -z "${YTSEARCH_INDEX_QUEUE_TOKEN:-}" ]; then
  echo "Missing required environment variable: YTSEARCH_INDEX_QUEUE_TOKEN" >&2
  exit 1
fi

curl -sS \
  -X DELETE \
  -H "Authorization: Bearer $YTSEARCH_INDEX_QUEUE_TOKEN" \
  "$QUEUE_URL/api/index-requests/$1"
