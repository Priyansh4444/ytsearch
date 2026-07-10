#!/usr/bin/env sh
set -eu

API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8787}"
QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
RUNS="${RUNS:-20}"
PER_PAGE="${PER_PAGE:-10}"
CURL_MAX_TIME="${CURL_MAX_TIME:-15}"
RESPONSE_FILE="${RESPONSE_FILE:-/tmp/ytsearch-benchmark-response.json}"

url="$API_BASE_URL/api/search?q=$(printf '%s' "$QUERY" | jq -sRr @uri)&per_page=$PER_PAGE"
if [ -n "$YOUTUBER" ]; then
  url="$url&youtuber=$(printf '%s' "$YOUTUBER" | jq -sRr @uri)"
fi

echo "Benchmarking $url ($RUNS runs)"
i=1
while [ "$i" -le "$RUNS" ]; do
  timing="$(curl -sS -o "$RESPONSE_FILE" \
    --max-time "$CURL_MAX_TIME" \
    -w "run=$i http=%{http_code} dns=%{time_namelookup}s connect=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s" \
    "$url" || true)"
  result_metrics="$(jq -r '
    if type == "object" then
      "first_result_ms=\(.time_to_first_result_ms // "") total_videos=\(.total_videos // "") total_moments=\(.total_moments // "") returned_videos=\(.videos | length)"
    else empty end
  ' "$RESPONSE_FILE" 2>/dev/null || true)"
  printf '%s %s\n' "$timing" "$result_metrics"
  i=$((i + 1))
done
