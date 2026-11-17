#!/usr/bin/env bash
set -euo pipefail

: "${SEARCH_URL:?set SEARCH_URL to the engine base URL}"
SEARCH_API_KEY="${SEARCH_API_KEY:-}"
BACKEND_KIND="${BACKEND_KIND:-raw}"
USE_CACHE="${USE_CACHE:-0}"
CAPTURE_RESOURCES="${CAPTURE_RESOURCES:-1}"
RUNS="${RUNS:-50}"
WARMUP_RUNS="${WARMUP_RUNS:-10}"
CURL_CONNECT_TIMEOUT="${CURL_CONNECT_TIMEOUT:-3}"
CURL_MAX_TIME="${CURL_MAX_TIME:-5}"
OUTPUT="${OUTPUT:-.search-experiment/results/${BACKEND_KIND}-direct.jsonl}"
APPEND_OUTPUT="${APPEND_OUTPUT:-0}"
STOP_FILE="${STOP_FILE:-}"
PRINT_SUMMARY="${PRINT_SUMMARY:-1}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CASES_FILE="${CASES_FILE:-$SCRIPT_DIR/benchmark-search-cases.tsv}"
CONTRACT_FILE="${CONTRACT_FILE:-$SCRIPT_DIR/benchmark-search-contract.json}"
# shellcheck disable=SC1091
. "$SCRIPT_DIR/search-benchmark-cases.sh"
validate_search_benchmark_cases "$CASES_FILE"
validate_search_benchmark_contract "$CONTRACT_FILE"
mkdir -p "$(dirname "$OUTPUT")"
if [[ "$APPEND_OUTPUT" != 1 ]]; then
  : > "$OUTPUT"
fi
BENCHMARK_TMP="$(mktemp -d)"
trap 'rm -rf "$BENCHMARK_TMP"' EXIT
trap 'exit 130' INT TERM
chmod 700 "$BENCHMARK_TMP"
SEARCH_HEADER_FILE=""
if [[ -n "$SEARCH_API_KEY" ]]; then
  if [[ "$(printf '%s' "$SEARCH_API_KEY" | tr -d '\r\n')" != "$SEARCH_API_KEY" ]]; then
    printf 'SEARCH_API_KEY must be a single-line value\n' >&2
    exit 1
  fi
  SEARCH_HEADER_FILE="$BENCHMARK_TMP/search.header"
  printf 'x-raw-search-key: %s\n' "$SEARCH_API_KEY" > "$SEARCH_HEADER_FILE"
  chmod 600 "$SEARCH_HEADER_FILE"
fi

declare -a labels=()
declare -a queries=()
declare -a creators=()
declare -a sample_prefixes=()
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
# Encode text in one process per sweep, outside continuous request intervals.
mapfile -t sample_prefixes < <(jq -Rrc '
  select((test("^[[:space:]]*$") | not) and (startswith("#") | not)) | split("\t") |
  {label:.[0],query:.[1],creator:((.[2] // "") | if . == "" then null else . end)} |
  tojson | .[0:-1]
' "$CASES_FILE")
[[ "${#sample_prefixes[@]}" == "${#queries[@]}" ]]
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
    --connect-timeout "$CURL_CONNECT_TIMEOUT"
    --max-time "$CURL_MAX_TIME"
    --get "${SEARCH_URL%/}/collections/youtube_transcript_chunks/documents/search"
    --data-urlencode "q=$query"
    --data-urlencode 'query_by=text,title'
    --data-urlencode 'group_by=video_id'
    --data-urlencode 'group_limit=3'
    --data-urlencode 'offset=0'
    --data-urlencode 'limit=10'
    --output "$output"
    --dump-header "$BENCHMARK_TMP/response.headers"
  )
  if [[ -n "$SEARCH_HEADER_FILE" ]]; then
    args+=(--header "@$SEARCH_HEADER_FILE")
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
    request "${queries[$index]}" "${creators[$index]}" /dev/null >/dev/null || true
    ((warmup += 1))
  done
done

if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_before="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
fi
for index in "${!queries[@]}"; do
  run=1
  while (( run <= RUNS )); do
    if [[ -n "$STOP_FILE" && -e "$STOP_FILE" ]]; then
      break 2
    fi
    response_file="$BENCHMARK_TMP/response.json"
    started_ms="$(date +%s%3N)"
    curl_exit=0
    timing="$(request "${queries[$index]}" "${creators[$index]}" "$response_file" \
      '%{http_code}\t%{time_total}\t%{size_download}\t%{time_connect}\t%{time_starttransfer}')" || curl_exit=$?
    read -r http seconds bytes connect_seconds first_byte_seconds <<< "$timing"
    # curl reports 000 when no HTTP response arrives; JSON requires numeric 0.
    http=$((10#${http:-0}))
    [[ -n "$seconds" ]] || seconds="$CURL_MAX_TIME"
    [[ -n "$bytes" ]] || bytes=0
    connect_seconds="${connect_seconds:-null}"
    first_byte_seconds="${first_byte_seconds:-null}"
    server_ms=null
    response_valid=false
    contract_valid=false
    if (( curl_exit == 0 )) && [[ "$http" == 200 ]] && jq -e \
      --arg creator "${creators[$index]}" '
      (.found | type == "number") and .found > 0 and
      (.grouped_hits | type == "array") and (.grouped_hits | length) > 0 and
      (if $creator == "" then true else
        all(.grouped_hits[].hits[].document.youtuber_handle;
          ascii_downcase == ($creator | ascii_downcase))
      end)
    ' "$response_file" >/dev/null 2>&1; then
      response_valid=true
      server_ms="$(awk '
        tolower($1) == "server-timing:" {
          sub(/\r$/, "", $2)
          sub(/^raw;dur=/, "", $2)
          if ($2 ~ /^[0-9]+([.][0-9]+)?$/) { print $2; exit }
        }
      ' "$BENCHMARK_TMP/response.headers" 2>/dev/null || true)"
      server_ms="${server_ms:-null}"
    fi
    if [[ "$response_valid" == true ]] && jq -e \
      --arg case_label "${labels[$index]}" --slurpfile contract "$CONTRACT_FILE" '
      (($contract[0].cases[$case_label] // null) as $rule |
        if $rule == null then true else
          .grouped_hits[0].group_key[0] == $rule.top_video_id and
          any(.grouped_hits[0].hits[].document.youtuber_handle;
            ascii_downcase == ($rule.top_handle | ascii_downcase)) and
          (([.grouped_hits[0].hits[].highlights[]?.snippet] | join(" ") | ascii_downcase) as $snippets |
            all($rule.required_highlight_tokens[]; . as $token | $snippets | contains($token)))
        end)
    ' "$response_file" >/dev/null 2>&1; then
      contract_valid=true
    fi
    ended_ms="$(date +%s%3N)"
    request_ok=false
    ok=false
    if (( curl_exit == 0 )) && [[ "$http" == 200 && "$response_valid" == true ]]; then
      request_ok=true
      [[ "$contract_valid" != true ]] || ok=true
    fi
    # All interpolated values here are curl numeric metrics or shell-owned
    # numbers/booleans. Text has already been JSON-encoded in sample_prefixes.
    printf '%s,"run":%s,"started_ms":%s,"ended_ms":%s,"seconds":%s,"http":%s,"curl_exit":%s,"bytes":%s,"response_valid":%s,"contract_valid":%s,"valid":%s,"request_ok":%s,"ok":%s,"connect_seconds":%s,"first_byte_seconds":%s,"server_ms":%s}\n' \
      "${sample_prefixes[$index]}" "$run" "$started_ms" "$ended_ms" "$seconds" \
      "${http:-0}" "$curl_exit" "$bytes" "$response_valid" "$contract_valid" \
      "$ok" "$request_ok" "$ok" "$connect_seconds" "$first_byte_seconds" \
      "$server_ms" >> "$OUTPUT"
    ((run += 1))
  done
done
if [[ "$CAPTURE_RESOURCES" == 1 ]]; then
  metrics_after="$(curl --fail --silent --show-error "${SEARCH_URL%/}/metrics")"
  jq -n --argjson before "$metrics_before" --argjson after "$metrics_after" \
    '{before:$before,after:$after,cpu_ticks:($after.cpu_ticks-$before.cpu_ticks),search_nanos:($after.search_nanos-$before.search_nanos),queue_wait_nanos:(($after.queue_wait_nanos//0)-($before.queue_wait_nanos//0)),backend_nanos:(($after.backend_nanos//0)-($before.backend_nanos//0)),searches:($after.searches-$before.searches)}' \
    > "${OUTPUT%.jsonl}-resources.json"
fi

if [[ "$PRINT_SUMMARY" != 1 ]]; then exit 0; fi

jq -s '
  group_by(.label) |
  map(. as $rows | [$rows[] | select(.ok)] as $ok | {
    label:$rows[0].label,
    query:$rows[0].query,
    creator:$rows[0].creator,
    runs:($rows | length),
    errors:(($rows | length) - ($ok | length)),
    min_ms:(if ($ok | length) == 0 then null else ($ok | map(.seconds*1000) | min) end),
    median_ms:(if ($ok | length) == 0 then null else ($ok | map(.seconds*1000) | sort | .[length/2]) end),
    p95_ms:(if ($ok | length) == 0 then null else ($ok | map(.seconds*1000) | sort | .[((length*0.95|ceil)-1)]) end),
    max_ms:(if ($ok | length) == 0 then null else ($ok | map(.seconds*1000) | max) end)
  })
' "$OUTPUT"
