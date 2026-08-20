#!/usr/bin/env bash
set -euo pipefail

: "${TYPESENSE_URL:?set TYPESENSE_URL to the same-corpus Typesense control}"
: "${TYPESENSE_API_KEY:?set TYPESENSE_API_KEY}"
: "${CANDIDATE_URL:?set CANDIDATE_URL}"
CANDIDATE_API_KEY="${CANDIDATE_API_KEY:-}"
CANDIDATE_NAME="${CANDIDATE_NAME:-raw}"
CASES_FILE="${CASES_FILE:-scripts/benchmark-search-cases.tsv}"
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
. "$SCRIPT_DIR/search-benchmark-cases.sh"
validate_search_benchmark_cases "$CASES_FILE"
OUTPUT="${OUTPUT:-.search-experiment/results/${CANDIDATE_NAME}-relevance.jsonl}"
mkdir -p "$(dirname "$OUTPUT")"
: > "$OUTPUT"

search_url() {
  local backend="$1"
  local query="$2"
  local handle="$3"
  local base_url="$4"
  local key="$5"
  local response="$6"
  local sort_by="$7"
  local args=(
    --fail --silent --show-error --get
    "${base_url%/}/collections/youtube_transcript_chunks/documents/search"
    --data-urlencode "q=$query"
    --data-urlencode 'query_by=text,title'
    --data-urlencode 'group_by=video_id'
    --data-urlencode 'group_limit=5'
    --data-urlencode 'offset=0'
    --data-urlencode 'limit=10'
  )
  if [[ -n "$handle" ]]; then
    args+=(--data-urlencode "filter_by=youtuber_handle:=$handle")
  fi
  if [[ -n "$sort_by" ]]; then
    args+=(--data-urlencode "sort_by=$sort_by")
  fi
  if [[ "$backend" == typesense ]]; then
    args+=(
      --data-urlencode 'query_by_weights=4,6'
      --data-urlencode 'text_match_type=sum_score'
      --data-urlencode 'prioritize_exact_match=true'
      --data-urlencode 'prioritize_token_position=true'
      --data-urlencode 'typo_tokens_threshold=5'
      --data-urlencode 'split_join_tokens=always'
    )
  fi
  if [[ -n "$key" ]]; then
    if [[ "$backend" == typesense ]]; then
      args+=(-H "X-TYPESENSE-API-KEY: $key")
    else
      args+=(-H "x-raw-search-key: $key")
    fi
  fi
  curl "${args[@]}" --output "$response"
}

while IFS=$'\t' read -r label query handle; do
  [[ -z "$label" || "$label" == \#* ]] && continue
  typesense_response="$(mktemp)"
  candidate_response="$(mktemp)"
  trap 'rm -f "$typesense_response" "$candidate_response"' EXIT
  search_url typesense "$query" "$handle" "$TYPESENSE_URL" "$TYPESENSE_API_KEY" "$typesense_response" ""
  search_url candidate "$query" "$handle" "$CANDIDATE_URL" "$CANDIDATE_API_KEY" "$candidate_response" ""

  jq -cn \
    --arg label "$label" \
    --arg query "$query" \
    --arg handle "$handle" \
    --arg candidate_name "$CANDIDATE_NAME" \
    --slurpfile typesense "$typesense_response" \
    --slurpfile candidate "$candidate_response" '
      def ids($response): [$response.grouped_hits[]?.group_key[0]];
      (ids($typesense[0])) as $oracle |
      (ids($candidate[0])) as $result |
      {
        label:$label,
        query:$query,
        handle:(if $handle == "" then null else $handle end),
        candidate_name:$candidate_name,
        typesense:$oracle,
        candidate:$result,
        top1_equal:(($oracle[0] // null) == ($result[0] // null)),
        overlap_at_10:([$result[] as $id | select($oracle | index($id))] | length),
        typesense_found:($typesense[0].found // 0),
        candidate_found:($candidate[0].found // 0)
      }
    ' >> "$OUTPUT"
  rm -f "$typesense_response" "$candidate_response"
  trap - EXIT
done < "$CASES_FILE"

jq -s '{cases:length,top1_agreement:(map(select(.top1_equal))|length),mean_overlap_at_10:(map(.overlap_at_10)|add/length),details:.}' "$OUTPUT"
