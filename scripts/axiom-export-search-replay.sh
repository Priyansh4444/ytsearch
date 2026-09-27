#!/usr/bin/env sh
set -eu

# Export distinct production Raw searches as a private replay dataset.
#
# Writes one TSV row per (query, youtuber, sort, page) with its production
# count, median/max backend milliseconds, total found, and last-seen time.
# Query text never reaches stdout: the file is created mode 0600 at the path
# given, and must stay outside Git (for example under .search-experiment/).
# Replay it with scripts/replay-production-searches.sh.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/axiom-query-lib.sh"

output="${1:?usage: axiom-export-search-replay.sh <output.tsv>}"
window="${WINDOW:-90d}"
limit="${LIMIT:-5000}"

axiom_require_query_config
axiom_validate_duration WINDOW "$window"
case "$limit" in
  *[!0-9]* | "")
    echo "LIMIT must be a positive integer." >&2
    exit 1
    ;;
esac

apl="['${AXIOM_QUERY_DATASET}'] | where event == \"search_query\" and backend == \"raw\" | where isnotnull(query) and query != \"\" | summarize searches=count(), p50_backend_ms=percentile(backend_search_ms, 50), max_backend_ms=max(backend_search_ms), found=max(total_found), last_seen=max(_time) by query, youtuber, sort, page | sort by searches desc | limit ${limit}"

response="$(mktemp)"
trap 'rm -f "$response"' EXIT HUP INT TERM
axiom_query_to_file "$apl" "now-${window}" "$response"

umask 077
jq -r '
  (.tables // [])[0] as $table
  | select($table != null)
  | [range(0; ($table.columns[0] // []) | length)] as $rows
  | $rows[] as $row
  | [$table.columns[][$row] | if . == null then "" else tostring end
      | gsub("[\t\n\r]"; " ")]
  | @tsv
' "$response" > "$output"
echo "rows=$(wc -l < "$output") window=$window output=$output"
