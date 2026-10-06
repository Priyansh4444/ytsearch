#!/usr/bin/env sh
set -eu

# Print only aggregate search timings. Tokens and raw user queries never reach
# stdout, which keeps this suitable for operator and agent use.
#
# Axiom tabular responses are column-oriented under tables[].fields/columns.
# Pass an explicit startTime/endTime so the report covers the intended window.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/axiom-query-lib.sh"

window="${WINDOW:-7d}"

axiom_require_query_config
axiom_validate_duration WINDOW "$window"

apl="['${AXIOM_QUERY_DATASET}'] | where event == \"search_query\" | summarize requests=count(), avg_total_ms=round(avg(duration_ms), 1), p50_total_ms=round(percentile(duration_ms, 50), 1), p95_total_ms=round(percentile(duration_ms, 95), 1), avg_backend_ms=round(avg(backend_search_ms), 1), p95_backend_ms=round(percentile(backend_search_ms, 95), 1), avg_processing_ms=round(avg(result_processing_ms), 1), avg_backend_calls=round(avg(backend_search_calls), 2) by backend"

response="$(mktemp)"
trap 'rm -f "$response"' EXIT
trap 'exit 130' HUP INT TERM
axiom_query_to_file "$apl" "now-${window}" "$response"
axiom_print_first_table "$response"
