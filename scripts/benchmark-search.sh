#!/usr/bin/env sh
set -eu

API_BASE_URL="${API_BASE_URL:-http://127.0.0.1:8787}"
QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-}"
RUNS="${RUNS:-20}"
PER_PAGE="${PER_PAGE:-10}"

url="$API_BASE_URL/api/search?q=$(printf '%s' "$QUERY" | jq -sRr @uri)&per_page=$PER_PAGE"
if [ -n "$YOUTUBER" ]; then
  url="$url&youtuber=$(printf '%s' "$YOUTUBER" | jq -sRr @uri)"
fi

echo "Benchmarking $url ($RUNS runs)"
i=1
while [ "$i" -le "$RUNS" ]; do
  curl -sS -o /tmp/ytsearch-benchmark-response.json \
    -w "run=$i http=%{http_code} dns=%{time_namelookup}s connect=%{time_connect}s tls=%{time_appconnect}s ttfb=%{time_starttransfer}s total=%{time_total}s\n" \
    "$url"
  i=$((i + 1))
done
