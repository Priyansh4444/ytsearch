#!/usr/bin/env sh
set -eu

# Exercise latency and result cardinality across a representative query suite.
# The Rust fixtures own deterministic ranking assertions; this script measures
# the same kinds of queries against live data without baking in mutable titles.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
CASES_FILE="${CASES_FILE:-$SCRIPT_DIR/benchmark-search-cases.tsv}"
RESULTS_FILE="${RESULTS_FILE:-/tmp/ytsearch-benchmark-cases.$$.log}"
RUNS="${RUNS:-10}"

if [ ! -f "$CASES_FILE" ]; then
  echo "Benchmark case file not found: $CASES_FILE" >&2
  exit 1
fi

: > "$RESULTS_FILE"
while IFS="$(printf '\t')" read -r label query youtuber; do
  case "$label" in
    ''|\#*) continue ;;
  esac
  if [ -z "$query" ]; then
    echo "Invalid benchmark case with no query: $label" >&2
    exit 1
  fi
  echo "case=$label query=$query youtuber=$youtuber"
  CASE_LABEL="$label" QUERY="$query" YOUTUBER="$youtuber" RUNS="$RUNS" SUMMARY=0 \
    RESULTS_FILE="$RESULTS_FILE" "$SCRIPT_DIR/benchmark-search-backends.sh"
done < "$CASES_FILE"

echo "summary_file=$RESULTS_FILE"
"$SCRIPT_DIR/summarize-benchmark-results.sh" "$RESULTS_FILE"
