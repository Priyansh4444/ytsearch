#!/usr/bin/env sh
set -eu

# Confirm indexed creator data and searchability from the public APIs.
# Usage:
#   scripts/check-indexed-channel-status.sh @vsauce @thePrimagen

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"
. "$(dirname "$0")/queue-api-url.sh"

INDEX_QUEUE_API_BASE_URL="$(normalize_queue_api_url "${INDEX_QUEUE_API_BASE_URL:-}")"

BACKENDS="${BACKENDS:-typesense-fly}"
QUERY="${QUERY:-the}"
PER_PAGE="${PER_PAGE:-3}"

if [ "$#" -eq 0 ]; then
  echo "Usage: scripts/check-indexed-channel-status.sh @handle [@handle ...]" >&2
  exit 1
fi

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

backend_url() {
  case "$1" in
    typesense-fly)
      required_env TYPESENSE_FLY_API_BASE_URL
      printf '%s' "$TYPESENSE_FLY_API_BASE_URL"
      ;;
    *)
      echo "Unknown backend: $1" >&2
      exit 1
      ;;
  esac
}

normalize_handle() {
  value="$1"
  case "$value" in
    @*) printf '%s' "$value" ;;
    *) printf '@%s' "$value" ;;
  esac
}

encode() {
  printf '%s' "$1" | jq -sRr @uri
}

require_command curl
require_command jq

printf 'backend\thandle\tindexed\tvideos\tchunks\tpublic_videos\tlatest_video\tsearch_http\tsearch_videos\tfirst_result\n'

for backend in $BACKENDS; do
  base_url="$(backend_url "$backend")"
  channels_file="/tmp/ytsearch-youtubers-$backend.json"
  curl -fsS "$base_url/api/youtubers" > "$channels_file"

  for raw_handle in "$@"; do
    handle="$(normalize_handle "$raw_handle")"
    channel_json="$(jq -c --arg handle "$handle" '
      .channels[]? | select((.handle | ascii_downcase) == ($handle | ascii_downcase))
    ' "$channels_file" | head -n 1)"

    if [ -z "$channel_json" ]; then
      videos=0
      chunks=0
      public_videos=""
      latest=""
      indexed="no"
    else
      videos="$(printf '%s' "$channel_json" | jq -r '.indexed_video_count // 0')"
      chunks="$(printf '%s' "$channel_json" | jq -r '.indexed_chunk_count // 0')"
      public_videos="$(printf '%s' "$channel_json" | jq -r '.public_video_count // ""')"
      latest="$(printf '%s' "$channel_json" | jq -r '.latest_video_id // ""')"
      indexed="yes"
    fi

    search_url="$base_url/api/search?q=$(encode "$QUERY")&youtuber=$(encode "$handle")&per_page=$PER_PAGE"
    search_body="/tmp/ytsearch-channel-search-$backend-$(printf '%s' "$handle" | tr -cd '[:alnum:]').json"
    search_http="$(curl -sS -o "$search_body" -w "%{http_code}" "$search_url" || true)"
    if [ "$search_http" = "200" ]; then
      search_videos="$(jq -r '.videos | length' "$search_body")"
      first_result="$(jq -r '.videos[0].title // ""' "$search_body")"
    else
      search_videos=0
      first_result=""
    fi

    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
      "$backend" "$handle" "$indexed" "$videos" "$chunks" "$public_videos" "$latest" \
      "$search_http" "$search_videos" "$first_result"
  done
done

if [ -n "${INDEX_QUEUE_API_BASE_URL:-}" ] && [ -n "${YTSEARCH_INDEX_QUEUE_TOKEN:-}" ]; then
  echo
  echo "queued_requests"
  printf 'id\thandle\tstatus\ttarget_backend\trequested_at\tnote\n'
  curl -fsS \
    -H "Authorization: Bearer $YTSEARCH_INDEX_QUEUE_TOKEN" \
    "$INDEX_QUEUE_API_BASE_URL/api/index-requests?limit=1000" |
    jq -r --argjson handles "$(printf '%s\n' "$@" | jq -R 'if startswith("@") then . else "@" + . end' | jq -s 'map(ascii_downcase)')" '
      .requests[]
      | select((.handle | ascii_downcase) as $handle | $handles | index($handle))
      | [.id, .handle, .status, (.target_backend // .source_backend), .requested_at, (.note // "")] | @tsv'
else
  echo
  echo "queued_requests skipped: set INDEX_QUEUE_API_BASE_URL and YTSEARCH_INDEX_QUEUE_TOKEN to inspect the private queue" >&2
fi
