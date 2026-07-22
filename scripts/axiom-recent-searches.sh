#!/usr/bin/env sh
set -eu

# Show recent search text and aggregate usage without exposing client metadata.
# This intentionally omits IP addresses, user agents, countries, and request IDs.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

token="${AXIOM_PROD_API_KEY:-${AXIOM_TOKEN:-${AXIOM_API_KEY:-}}}"
dataset="${AXIOM_DATASET:-}"
window="${WINDOW:-7d}"
limit="${LIMIT:-200}"

if [ -z "$token" ] || [ -z "$dataset" ]; then
  echo "AXIOM_PROD_API_KEY (or an Axiom query-capable token) and AXIOM_DATASET are required." >&2
  exit 1
fi
case "$window" in
  *[!0-9mhdw]* | "")
    echo "WINDOW must be a duration such as 24h or 7d." >&2
    exit 1
    ;;
esac
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

base="${AXIOM_EDGE_URL:-${AXIOM_URL:-https://api.axiom.co}}"
base="${base%/}"
apl="['${dataset}'] | where _time > ago(${window}) | where event == \"search_query\" | summarize searches=count(), last_seen=max(_time) by query | sort by last_seen desc | limit ${limit}"
body="$(jq -n --arg apl "$apl" '{apl:$apl}')"

response="$(mktemp)"
trap 'rm -f "$response"' EXIT HUP INT TERM
status="$(curl -sS --max-time 30 -o "$response" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  -H 'Content-Type: application/json' \
  -d "$body" \
  "$base/v1/datasets/_apl?format=tabular")"

if [ "$status" -lt 200 ] || [ "$status" -ge 300 ]; then
  message="$(jq -r '.message // .error // "unknown Axiom query failure"' "$response" 2>/dev/null || true)"
  echo "Axiom query failed (HTTP $status): $message" >&2
  exit 1
fi

jq -r '
  (.columns // [] | map(.name // .)) as $columns
  | [$columns, (.rows // .data // [])[]] | @tsv
' "$response"
