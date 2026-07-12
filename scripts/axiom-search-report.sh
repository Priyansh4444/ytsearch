#!/usr/bin/env sh
set -eu

# Print only aggregate search timings. Tokens and raw user queries never reach
# stdout, which keeps this suitable for operator and agent use.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

token="${AXIOM_TOKEN:-${AXIOM_API_KEY:-}}"
dataset="${AXIOM_DATASET:-}"
if [ -z "$token" ] || [ -z "$dataset" ]; then
  echo "AXIOM_TOKEN (or AXIOM_API_KEY) and AXIOM_DATASET are required." >&2
  exit 1
fi

base="${AXIOM_EDGE_URL:-${AXIOM_URL:-https://api.axiom.co}}"
base="${base%/}"
apl="['${dataset}'] | where event == \"search_query\" | summarize requests=count(), avg_total_ms=round(avg(duration_ms), 1), p50_total_ms=round(percentile(duration_ms, 50), 1), p95_total_ms=round(percentile(duration_ms, 95), 1), avg_backend_ms=round(avg(backend_search_ms), 1), p95_backend_ms=round(percentile(backend_search_ms, 95), 1), avg_processing_ms=round(avg(result_processing_ms), 1), avg_backend_calls=round(avg(backend_search_calls), 2) by backend"
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
