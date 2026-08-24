#!/usr/bin/env sh
set -eu

# Safe Axiom entry point for operators and coding agents. This script never
# prints tokens or raw event/query data.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/load-env.sh"

usage() {
  cat <<'EOF'
Usage: scripts/axiom-agent.sh COMMAND

Commands:
  check       Check configured token authentication/permissions without printing data
  mcp-status  Show the Codex Axiom MCP configuration
  mcp-login   Start the official hosted Axiom OAuth flow
  report      Print the aggregate, privacy-safe search latency report
  recent      Print bounded recent query counts without client metadata
  refresh     Append new timestamped queries to query-log.txt
EOF
}

check_token() {
  label="$1"
  token="$2"
  if [ -z "$token" ]; then
    printf '%s\tmissing\n' "$label"
    return
  fi

  response="$(mktemp)"
  list_status="$(curl -sS --max-time 20 -o "$response" -w '%{http_code}' \
    -H "Authorization: Bearer $token" \
    "${AXIOM_URL:-https://api.axiom.co}/v1/datasets" || printf '000')"
  case "$list_status" in
    2??) result="authenticated" ;;
    401) result="authentication rejected" ;;
    403) result="accepted; dataset-list not allowed" ;;
    000) result="network request failed" ;;
    *) result="unexpected HTTP $list_status" ;;
  esac

  if [ -n "${AXIOM_DATASET:-}" ]; then
    # take 0 is invalid APL; limit 1 is enough for a permission probe.
    apl="['${AXIOM_DATASET}'] | limit 1"
    body="$(jq -n --arg apl "$apl" '{apl:$apl, startTime:"now-1h", endTime:"now"}')"
    query_status="$(curl -sS --max-time 20 -o "$response" -w '%{http_code}' \
      -H "Authorization: Bearer $token" \
      -H 'Content-Type: application/json' \
      -d "$body" \
      "${AXIOM_URL:-https://api.axiom.co}/v1/datasets/_apl?format=tabular" || printf '000')"
    case "$query_status" in
      2??) result="$result; query allowed" ;;
      403) result="$result; query denied" ;;
      *) result="$result; query check HTTP $query_status" ;;
    esac
  fi
  rm -f "$response"
  printf '%s\t%s\n' "$label" "$result"
}

case "${1:-}" in
  check)
    # Ingest role (API / indexer). Query role (operator scripts). Never mix.
    check_token "AXIOM_API_KEY (ingest)" "${AXIOM_API_KEY:-${AXIOM_TOKEN:-}}"
    check_token "AXIOM_API_QUERY (query)" "${AXIOM_API_QUERY:-${AXIOM_PROD_API_KEY:-}}"
    ;;
  mcp-status)
    if ! command -v codex >/dev/null 2>&1; then
      echo "codex is required." >&2
      exit 1
    fi
    codex mcp get axiom
    ;;
  mcp-login)
    if ! command -v codex >/dev/null 2>&1; then
      echo "codex is required." >&2
      exit 1
    fi
    if ! codex mcp get axiom >/dev/null 2>&1; then
      codex mcp add axiom --url https://mcp.axiom.co/mcp
    fi
    exec codex mcp login axiom
    ;;
  report)
    # Report script resolves AXIOM_API_QUERY itself; do not export query into AXIOM_TOKEN.
    exec "$SCRIPT_DIR/axiom-search-report.sh"
    ;;
  recent)
    exec "$SCRIPT_DIR/axiom-recent-searches.sh"
    ;;
  refresh)
    exec "$SCRIPT_DIR/refresh-query-log"
    ;;
  -h|--help|help|"") usage ;;
  *) usage >&2; exit 2 ;;
esac
