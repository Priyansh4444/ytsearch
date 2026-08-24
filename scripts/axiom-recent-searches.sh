#!/usr/bin/env sh
set -eu

# Show recent search text and aggregate usage without exposing client metadata.
# This intentionally omits IP addresses, user agents, countries, and request IDs.
#
# Axiom tabular responses are column-oriented under tables[].fields/columns.
# The scan window is set via startTime/endTime (not only an APL where clause).

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/axiom-query-lib.sh"

window="${WINDOW:-7d}"
limit="${LIMIT:-200}"

axiom_require_query_config
axiom_validate_duration WINDOW "$window"
case "$limit" in
  *[!0-9]* | "")
    echo "LIMIT must be a positive integer." >&2
    exit 1
    ;;
esac
if [ "$limit" -lt 1 ] || [ "$limit" -gt 1000 ]; then
  echo "LIMIT must be between 1 and 1000." >&2
  exit 1
fi

apl="['${AXIOM_QUERY_DATASET}'] | where event == \"search_query\" | where isnotnull(query) and query != \"\" | summarize searches=count(), last_seen=max(_time) by query | sort by last_seen desc | limit ${limit}"

response="$(mktemp)"
trap 'rm -f "$response"' EXIT HUP INT TERM
axiom_query_to_file "$apl" "now-${window}" "$response"
axiom_print_first_table "$response"
