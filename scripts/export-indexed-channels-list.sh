#!/usr/bin/env sh
set -eu

# Export indexed creators to a plain-text list file.
# Format per line:
#   @handle | Creator Name | 123 indexed

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

OUTPUT_FILE="${1:-indexed-channels.txt}"
API_BASE_URL="${API_BASE_URL:-${TYPESENSE_FLY_API_BASE_URL:-}}"

if [ -z "$API_BASE_URL" ]; then
  echo "Missing required environment variable: API_BASE_URL or TYPESENSE_FLY_API_BASE_URL" >&2
  exit 1
fi

curl -sS "$API_BASE_URL/api/youtubers" |
  jq -r '.channels[]
    | select(.indexed_video_count > 0 and .indexed_chunk_count > 0)
    | "\(.handle) | \(.name) | \(.indexed_video_count) indexed"' \
  > "$OUTPUT_FILE"

echo "Wrote $OUTPUT_FILE"
