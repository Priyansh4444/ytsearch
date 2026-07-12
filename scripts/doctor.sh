#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
SNAPSHOT_FILE="${SNAPSHOT_FILE:-}"

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
  echo "NEXT run: TYPESENSE_FRONTEND_URL=https://ytsearch-755.pages.dev $SCRIPT_DIR/check-frontend-backends.sh"
  exit 1
fi

if [ "$engine" != "200" ]; then
  echo "FAIL Typesense health HTTP ${engine:-unknown}"
  echo "NEXT run: VERBOSE=1 $SCRIPT_DIR/check-typesense-fly.sh"
  exit 1
fi

if [ "$search" != "200" ]; then
  echo "FAIL search API HTTP ${search:-unknown}"
  echo "NEXT run: flyctl logs --app ytsearch-api-typesense-fly-staging --no-tail | tail -n 50"
  exit 1
fi

if [ "$worker" != "started" ]; then
  echo "FAIL queue worker state ${worker:-missing}"
  echo "NEXT run: $SCRIPT_DIR/check-queue-indexers.sh"
  exit 1
fi

case "$indexing" in
  running)
    echo "OK serving; indexing ${handle:-unknown} ${indexed:-?}/${public:-?}"
    ;;
  queued)
    echo "OK serving; queue waiting for ${handle:-unknown}"
    ;;
  *)
    echo "OK serving; indexer idle"
    ;;
esac
