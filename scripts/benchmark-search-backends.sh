#!/usr/bin/env sh
set -eu

# Retains the historical command name while benchmarking the sole active API.
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

API_BASE_URL="${API_BASE_URL:-${TYPESENSE_FLY_API_BASE_URL:-}}"
QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
RUNS="${RUNS:-10}"
RESULTS_FILE="${RESULTS_FILE:-}"
CASE_LABEL="${CASE_LABEL:-default}"
BACKEND_LABEL="${BACKEND_LABEL:-typesense}"
ACCEPT_ENCODING="${ACCEPT_ENCODING:-gzip}"
PAGE="${PAGE:-1}"
PER_PAGE="${PER_PAGE:-10}"
SORT="${SORT:-}"
VIDEO_ID="${VIDEO_ID:-}"
QUIET="${QUIET:-0}"

if [ -z "$API_BASE_URL" ]; then
  echo "API_BASE_URL or TYPESENSE_FLY_API_BASE_URL is required" >&2
  exit 1
fi

encode() { printf '%s' "$1" | jq -sRr @uri; }
url="${API_BASE_URL%/}/api/search?q=$(encode "$QUERY")&per_page=$PER_PAGE&page=$PAGE"
if [ -n "$YOUTUBER" ]; then
  url="$url&youtuber=$(encode "$YOUTUBER")"
fi
if [ -n "$VIDEO_ID" ]; then
  url="$url&video_id=$(encode "$VIDEO_ID")"
fi
if [ -n "$SORT" ]; then
  url="$url&sort=$(encode "$SORT")"
fi

run=1
while [ "$run" -le "$RUNS" ]; do
  response_file="$(mktemp)"
  trap 'rm -f "$response_file"' EXIT INT TERM
  timing="$(curl -sS -H "Accept-Encoding: $ACCEPT_ENCODING" -o "$response_file" -w '%{http_code}\t%{time_total}\t%{size_download}' "$url")"
  http="$(printf '%s' "$timing" | cut -f1)"
  total="$(printf '%s' "$timing" | cut -f2)"
  bytes="$(printf '%s' "$timing" | cut -f3)"
  decoded_file="$response_file.decoded"
  if [ "$ACCEPT_ENCODING" = "gzip" ]; then
    gzip -dc "$response_file" > "$decoded_file" 2>/dev/null || cp "$response_file" "$decoded_file"
  else
    cp "$response_file" "$decoded_file"
  fi
  if [ "$http" = "200" ] && jq -e . "$decoded_file" >/dev/null 2>&1; then
    first_result_ms="$(jq -r '.time_to_first_result_ms // 0' "$decoded_file")"
    total_videos="$(jq -r '.total_videos // 0' "$decoded_file")"
    total_moments="$(jq -r '.total_moments // .total_found // 0' "$decoded_file")"
  else
    first_result_ms=0
    total_videos=0
    total_moments=0
  fi
  row="case=$CASE_LABEL backend=$BACKEND_LABEL encoding=$ACCEPT_ENCODING http=$http total=${total}s bytes=$bytes first_result_ms=$first_result_ms total_videos=$total_videos total_moments=$total_moments"
  if [ "$QUIET" != "1" ]; then
    printf '%s\n' "$row"
  fi
  if [ -n "$RESULTS_FILE" ]; then
    printf '%s\n' "$row" >> "$RESULTS_FILE"
  fi
  rm -f "$response_file" "$decoded_file"
  trap - EXIT INT TERM
  run=$((run + 1))
done
