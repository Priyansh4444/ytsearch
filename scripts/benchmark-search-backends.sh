#!/usr/bin/env sh
set -eu

# Retains the historical command name while benchmarking the sole active API.
SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

API_BASE_URL="${API_BASE_URL:-${RAW_SEARCH_API_BASE_URL:-}}"
QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
RUNS="${RUNS:-10}"
RESULTS_FILE="${RESULTS_FILE:-}"
CASE_LABEL="${CASE_LABEL:-default}"
BACKEND_LABEL="${BACKEND_LABEL:-raw}"
ACCEPT_ENCODING="${ACCEPT_ENCODING:-gzip}"
CURL_CONNECT_TIMEOUT="${CURL_CONNECT_TIMEOUT:-3}"
CURL_MAX_TIME="${CURL_MAX_TIME:-5}"
PAGE="${PAGE:-1}"
PER_PAGE="${PER_PAGE:-10}"
SORT="${SORT:-}"
VIDEO_ID="${VIDEO_ID:-}"
QUIET="${QUIET:-0}"

if [ -z "$API_BASE_URL" ]; then
  echo "API_BASE_URL or RAW_SEARCH_API_BASE_URL is required" >&2
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
  decoded_file="$response_file.decoded"
  trap 'rm -f "$response_file" "$decoded_file"' EXIT INT TERM
  curl_exit=0
  timing="$(curl -sS --connect-timeout "$CURL_CONNECT_TIMEOUT" --max-time "$CURL_MAX_TIME" \
    -H "Accept-Encoding: $ACCEPT_ENCODING" -o "$response_file" \
    -w '%{http_code}\t%{time_total}\t%{size_download}' "$url")" || curl_exit=$?
  http="$(printf '%s' "$timing" | cut -f1)"
  total="$(printf '%s' "$timing" | cut -f2)"
  bytes="$(printf '%s' "$timing" | cut -f3)"
  if [ -z "$total" ]; then
    total="$CURL_MAX_TIME"
  fi
  if [ -z "$bytes" ]; then
    bytes=0
  fi
  if [ "$curl_exit" -eq 0 ] && [ "$ACCEPT_ENCODING" = "gzip" ]; then
    gzip -dc "$response_file" > "$decoded_file" 2>/dev/null || cp "$response_file" "$decoded_file"
  elif [ "$curl_exit" -eq 0 ]; then
    cp "$response_file" "$decoded_file"
  else
    : > "$decoded_file"
  fi
  valid_response=0
  if [ "$curl_exit" -eq 0 ] && [ "$http" = "200" ] && jq -e '
    (.videos | type == "array") and
    (.total_videos | type == "number") and
    ((.total_moments // .total_found) | type == "number")
  ' "$decoded_file" >/dev/null 2>&1; then
    valid_response=1
    first_result_ms="$(jq -r '.time_to_first_result_ms // 0' "$decoded_file")"
    total_videos="$(jq -r '.total_videos // 0' "$decoded_file")"
    total_moments="$(jq -r '.total_moments // .total_found // 0' "$decoded_file")"
  else
    first_result_ms=0
    total_videos=0
    total_moments=0
  fi
  row="case=$CASE_LABEL backend=$BACKEND_LABEL encoding=$ACCEPT_ENCODING http=${http:-000} curl_exit=$curl_exit valid_response=$valid_response total=${total}s bytes=$bytes first_result_ms=$first_result_ms total_videos=$total_videos total_moments=$total_moments"
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
