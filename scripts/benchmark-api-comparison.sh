#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
CASES_FILE="${CASES_FILE:-$SCRIPT_DIR/benchmark-search-cases.tsv}"
BEFORE_API_BASE_URL="${BEFORE_API_BASE_URL:?set BEFORE_API_BASE_URL}"
AFTER_API_BASE_URL="${AFTER_API_BASE_URL:?set AFTER_API_BASE_URL}"
RUNS="${RUNS:-30}"
WARMUP_RUNS="${WARMUP_RUNS:-3}"
RESULTS_FILE="${RESULTS_FILE:-/tmp/ytsearch-api-comparison.$$.log}"
VIDEO_ID="${VIDEO_ID:-}"

: > "$RESULTS_FILE"

run_case() {
  label="$1"
  query="$2"
  youtuber="$3"
  video_id="$4"
  page="$5"
  per_page="$6"
  sort="$7"

  for base_url in "$BEFORE_API_BASE_URL" "$AFTER_API_BASE_URL"; do
    API_BASE_URL="$base_url" QUERY="$query" YOUTUBER="$youtuber" VIDEO_ID="$video_id" \
      PAGE="$page" PER_PAGE="$per_page" SORT="$sort" CASE_LABEL="$label" \
      BACKEND_LABEL=warmup RESULTS_FILE="" RUNS="$WARMUP_RUNS" QUIET=1 \
      "$SCRIPT_DIR/benchmark-search-backends.sh"
  done

  run=1
  while [ "$run" -le "$RUNS" ]; do
    if [ $((run % 2)) -eq 1 ]; then
      order="before after"
    else
      order="after before"
    fi
    for variant in $order; do
      if [ "$variant" = "before" ]; then
        base_url="$BEFORE_API_BASE_URL"
      else
        base_url="$AFTER_API_BASE_URL"
      fi
      API_BASE_URL="$base_url" QUERY="$query" YOUTUBER="$youtuber" VIDEO_ID="$video_id" \
        PAGE="$page" PER_PAGE="$per_page" SORT="$sort" CASE_LABEL="$label" \
        BACKEND_LABEL="$variant" RESULTS_FILE="$RESULTS_FILE" RUNS=1 QUIET=1 \
        "$SCRIPT_DIR/benchmark-search-backends.sh"
    done
    run=$((run + 1))
  done
}

while IFS="$(printf '\t')" read -r label query youtuber; do
  case "$label" in
    ''|\#*) continue ;;
  esac
  run_case "$label" "$query" "$youtuber" "" 1 10 ""
done < "$CASES_FILE"

run_case page_two "linux exploit" "" "" 2 10 ""
run_case newest "linux exploit" "" "" 1 10 published_desc
if [ -n "$VIDEO_ID" ]; then
  run_case video_moments "the" "" "$VIDEO_ID" 1 100 ""
fi

echo "results_file=$RESULTS_FILE"
"$SCRIPT_DIR/summarize-benchmark-results.sh" "$RESULTS_FILE"
