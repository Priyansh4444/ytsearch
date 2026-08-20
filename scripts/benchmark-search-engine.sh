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
CASES_FILE="${CASES_FILE:-$SCRIPT_DIR/benchmark-search-cases.tsv}"
# shellcheck disable=SC1091
. "$SCRIPT_DIR/search-benchmark-cases.sh"
validate_search_benchmark_cases "$CASES_FILE"
mkdir -p "$(dirname "$OUTPUT")"
: > "$OUTPUT"

declare -a labels=()
declare -a queries=()
declare -a creators=()
while IFS=$'\t' read -r label query creator; do
  [[ -z "$label" || "$label" == \#* ]] && continue
  if [[ -z "$query" ]]; then
    printf 'benchmark case %s has no query\n' "$label" >&2
    exit 1
  fi
  labels+=("$label")
  queries+=("$query")
  creators+=("$creator")
done < "$CASES_FILE"
if (( ${#queries[@]} == 0 )); then
  printf 'no benchmark queries found in %s\n' "$CASES_FILE" >&2
  exit 1
fi

request() {
  local query="$1"
  local creator="$2"
  local output="$3"
  local write_out="${4:-}"
  local args=(
    --fail --silent --show-error
    --get "${SEARCH_URL%/}/collections/youtube_transcript_chunks/documents/search"
    --data-urlencode "q=$query"
    --data-urlencode 'query_by=text,title'
    --data-urlencode 'group_by=video_id'
    --data-urlencode 'group_limit=3'
    --data-urlencode 'offset=0'
    --data-urlencode 'limit=10'
    --output "$output"
  )
  if [[ -n "$SEARCH_API_KEY" ]]; then
    args+=(-H "x-raw-search-key: $SEARCH_API_KEY")
  fi
  if [[ -n "$creator" ]]; then
    args+=(--data-urlencode "filter_by=youtuber_handle:=$creator")
  fi
  args+=(
    --data-urlencode 'query_by_weights=4,6'
    --data-urlencode 'text_match_type=sum_score'
    --data-urlencode 'prioritize_exact_match=true'
    --data-urlencode 'prioritize_token_position=true'
    --data-urlencode 'typo_tokens_threshold=5'
    --data-urlencode 'split_join_tokens=always'
  )
  if [[ "$USE_CACHE" == 1 ]]; then
    args+=(--data-urlencode 'use_cache=true' --data-urlencode 'cache_ttl=60')
  fi
  if [[ -n "$write_out" ]]; then
    args+=(--write-out "$write_out")
  fi
  curl "${args[@]}"
}

for index in "${!queries[@]}"; do
  warmup=1
  while (( warmup <= WARMUP_RUNS )); do
    request "${queries[$index]}" "${creators[$index]}" /dev/null
    ((warmup += 1))
  done
done

if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_before="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
fi
for index in "${!queries[@]}"; do
  run=1
  while (( run <= RUNS )); do
    timing="$(request "${queries[$index]}" "${creators[$index]}" /dev/null '%{time_total}')"
    jq -cn --arg label "${labels[$index]}" --arg query "${queries[$index]}" \
      --arg creator "${creators[$index]}" --argjson run "$run" --argjson seconds "$timing" \
      '{label:$label,query:$query,creator:(if $creator == "" then null else $creator end),run:$run,seconds:$seconds}' >> "$OUTPUT"
    ((run += 1))
  done
done
if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_after="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
  jq -n --argjson before "$metrics_before" --argjson after "$metrics_after" \
    '{before:$before,after:$after,cpu_ticks:($after.cpu_ticks-$before.cpu_ticks),search_nanos:($after.search_nanos-$before.search_nanos),searches:($after.searches-$before.searches)}' \
    > "${OUTPUT%.jsonl}-resources.json"
fi

jq -s 'group_by(.label) | map({label:.[0].label,query:.[0].query,creator:.[0].creator,runs:length,min_ms:(map(.seconds*1000)|min),median_ms:(map(.seconds*1000)|sort|.[length/2]),p95_ms:(map(.seconds*1000)|sort|.[(length*0.95|floor)]),max_ms:(map(.seconds*1000)|max)})' "$OUTPUT"
