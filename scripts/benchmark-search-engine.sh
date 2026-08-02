#!/usr/bin/env bash
set -euo pipefail

: "${SEARCH_URL:?set SEARCH_URL to the engine base URL}"
SEARCH_API_KEY="${SEARCH_API_KEY:-}"
BACKEND_KIND="${BACKEND_KIND:-raw}"
USE_CACHE="${USE_CACHE:-0}"
CAPTURE_RESOURCES="${CAPTURE_RESOURCES:-1}"
RUNS="${RUNS:-50}"
WARMUP_RUNS="${WARMUP_RUNS:-10}"
OUTPUT="${OUTPUT:-.search-experiment/results/${BACKEND_KIND}-direct.jsonl}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
QUERIES_FILE="${QUERIES_FILE:-$SCRIPT_DIR/benchmark-search-queries.txt}"
mkdir -p "$(dirname "$OUTPUT")"
: > "$OUTPUT"

mapfile -t queries < <(sed -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' "$QUERIES_FILE")
if (( ${#queries[@]} == 0 )); then
  printf 'no benchmark queries found in %s\n' "$QUERIES_FILE" >&2
  exit 1
fi

request() {
  local query="$1"
  local output="$2"
  local write_out="${3:-}"
  local args=(
    --fail --silent --show-error
    --get "${SEARCH_URL%/}/collections/youtube_transcript_chunks/documents/search"
    --data-urlencode "q=$query"
    --data-urlencode 'query_by=text,title'
    --data-urlencode 'group_by=video_id'
    --data-urlencode 'group_limit=5'
    --data-urlencode 'offset=0'
    --data-urlencode 'limit=11'
    --output "$output"
  )
  if [[ -n "$SEARCH_API_KEY" ]]; then
    args+=(-H "X-TYPESENSE-API-KEY: $SEARCH_API_KEY")
  fi
  if [[ "$BACKEND_KIND" == typesense ]]; then
    args+=(
      --data-urlencode 'query_by_weights=4,6'
      --data-urlencode 'text_match_type=sum_score'
      --data-urlencode 'prioritize_exact_match=true'
      --data-urlencode 'prioritize_token_position=true'
      --data-urlencode 'typo_tokens_threshold=5'
      --data-urlencode 'split_join_tokens=always'
    )
  fi
  if [[ "$USE_CACHE" == 1 ]]; then
    args+=(--data-urlencode 'use_cache=true' --data-urlencode 'cache_ttl=60')
  fi
  if [[ -n "$write_out" ]]; then
    args+=(--write-out "$write_out")
  fi
  curl "${args[@]}"
}

for query in "${queries[@]}"; do
  warmup=1
  while (( warmup <= WARMUP_RUNS )); do
    request "$query" /dev/null
    ((warmup += 1))
  done
done

if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_before="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
fi
for query in "${queries[@]}"; do
  run=1
  while (( run <= RUNS )); do
    timing="$(request "$query" /dev/null '%{time_total}')"
    jq -cn --arg query "$query" --argjson run "$run" --argjson seconds "$timing" \
      '{query:$query,run:$run,seconds:$seconds}' >> "$OUTPUT"
    ((run += 1))
  done
done
if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_after="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
  jq -n --argjson before "$metrics_before" --argjson after "$metrics_after" \
    '{before:$before,after:$after,cpu_ticks:($after.cpu_ticks-$before.cpu_ticks),search_nanos:($after.search_nanos-$before.search_nanos),searches:($after.searches-$before.searches)}' \
    > "${OUTPUT%.jsonl}-resources.json"
fi

jq -s 'group_by(.query) | map({query:.[0].query,runs:length,min_ms:(map(.seconds*1000)|min),median_ms:(map(.seconds*1000)|sort|.[length/2]),p95_ms:(map(.seconds*1000)|sort|.[(length*0.95|floor)]),max_ms:(map(.seconds*1000)|max)})' "$OUTPUT"
