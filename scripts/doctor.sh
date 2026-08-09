#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
SNAPSHOT_FILE="${SNAPSHOT_FILE:-}"
STATE_FILE="${DOCTOR_STATE_FILE:-${TMPDIR:-/tmp}/ytsearch-doctor-state.json}"
STALL_MINUTES="${STALL_MINUTES:-30}"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT HUP INT TERM

if [ -n "$SNAPSHOT_FILE" ]; then
  cp "$SNAPSHOT_FILE" "$tmp"
else
  "$SCRIPT_DIR/agent-status.sh" > "$tmp" 2>/dev/null || true
fi

if ! jq -e 'type == "object"' "$tmp" >/dev/null 2>&1; then
  echo "FAIL diagnostics unavailable"
  echo "NEXT check local prerequisites: command -v curl jq flyctl"
  exit 1
fi

value() { jq -r "$1 // empty" "$tmp"; }
frontend="$(value '.frontend_http')"
engine="$(value '.engine_http')"
search="$(value '.search.http')"
worker="$(value '.worker.state')"
indexing="$(value '.indexing.status')"
handle="$(value '.indexing.handle')"
indexed="$(value '.indexing.indexed')"
public="$(value '.indexing.public')"

if [ "$frontend" != "200" ]; then
  echo "FAIL frontend HTTP ${frontend:-unknown}"
  echo "NEXT run: RAW_SEARCH_FRONTEND_URL=https://search.pronsh.dev $SCRIPT_DIR/check-frontend-backends.sh"
  exit 1
fi

if [ "$engine" != "200" ]; then
  echo "FAIL Raw search health HTTP ${engine:-unknown}"
  echo "NEXT run: $SCRIPT_DIR/check-raw-search-engine.sh"
  exit 1
fi

if [ "$search" != "200" ]; then
  echo "FAIL search API HTTP ${search:-unknown}"
  echo "NEXT run: flyctl logs --app ytsearch-api-raw-production --no-tail | tail -n 50"
  exit 1
fi

if { [ "$indexing" = "running" ] || [ "$indexing" = "queued" ]; } && [ "$worker" != "started" ]; then
  echo "FAIL queue worker state ${worker:-missing}"
  echo "NEXT run: $SCRIPT_DIR/check-queue-indexers.sh"
  exit 1
fi

case "$indexing" in
  running)
    now="$(date +%s)"
    if [ -f "$STATE_FILE" ] && jq -e --arg handle "$handle" --arg indexed "$indexed" \
      '.handle == $handle and (.indexed | tostring) == $indexed' "$STATE_FILE" >/dev/null 2>&1; then
      last_progress="$(jq -r '.last_progress // 0' "$STATE_FILE")"
      stalled_seconds=$((now - last_progress))
      if [ "$stalled_seconds" -ge $((STALL_MINUTES * 60)) ]; then
        echo "FAIL indexing stalled ${handle:-unknown} at ${indexed:-?}/${public:-?} for $((stalled_seconds / 60))m"
        echo "NEXT run: $SCRIPT_DIR/summarize-indexer-logs.sh"
        exit 1
      fi
    else
      jq -nc --arg handle "$handle" --arg indexed "$indexed" --argjson last_progress "$now" \
        '{handle:$handle,indexed:$indexed,last_progress:$last_progress}' > "$STATE_FILE"
    fi
    echo "OK serving; indexing ${handle:-unknown} ${indexed:-?}/${public:-?}"
    ;;
  queued)
    echo "OK serving; queue waiting for ${handle:-unknown}"
    ;;
  *)
    echo "OK serving; indexer idle"
    ;;
esac
