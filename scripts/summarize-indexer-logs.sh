#!/usr/bin/env sh
set -eu

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
LINES="${LINES:-25}"
ERROR_LINES="${ERROR_LINES:-3}"
VERBOSE="${VERBOSE:-0}"

command -v flyctl >/dev/null 2>&1 || {
  echo "missing_command=flyctl" >&2
  exit 1
}

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT HUP INT TERM
flyctl logs --app "$APP_NAME" --no-tail 2>/dev/null > "$tmp" || true

strip_and_redact() {
  sed -E \
    -e 's/\x1B\[[0-9;]*[[:alpha:]]//g' \
    -e 's/(Bearer|token|api[_-]?key|secret)[=:][^[:space:]]+/\1=[redacted]/Ig' \
    -e 's/[A-Za-z0-9_-]{32,}/[redacted]/g'
}

if [ "$VERBOSE" = "1" ]; then
  strip_and_redact < "$tmp" | tail -n "$LINES"
  exit 0
fi

clean="$(mktemp)"
trap 'rm -f "$tmp" "$clean"' EXIT HUP INT TERM
strip_and_redact < "$tmp" > "$clean"

latest_done="$(rg -i 'Done [0-9]+/[0-9]+:' "$clean" | tail -n 1 || true)"
latest_active="$(rg -i 'Indexing video [0-9]+/[0-9]+:|processing request=' "$clean" | tail -n 1 || true)"
latest_summary="$(rg -i 'Index summary:|pass complete|acked request=' "$clean" | tail -n 1 || true)"

[ -z "$latest_done" ] || printf 'latest_done\t%s\n' "$latest_done"
[ -z "$latest_active" ] || printf 'active\t%s\n' "$latest_active"
[ -z "$latest_summary" ] || printf 'summary\t%s\n' "$latest_summary"

errors="$(rg -i '\[error\]|panic|killed|out of memory|write to disk|health check.*failed|YouTuber\(s\) failed|backend failed|skipped .* error|publication failed' "$clean" | tail -n "$ERROR_LINES" || true)"
if [ -n "$errors" ]; then
  printf '%s\n' "$errors" | cut -c1-400 | sed 's/^/error\t/'
fi
