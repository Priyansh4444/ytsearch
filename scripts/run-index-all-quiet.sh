#!/usr/bin/env sh
set -eu

LIST_FILE="${1:-scripts/indexed-channels.txt}"
LOG_FILE="${LOG_FILE:-/tmp/ytsearch-index-all.log}"

mkdir -p "$(dirname "$LOG_FILE")"

if "$(dirname "$0")/index-all-raw.sh" "$LIST_FILE" > "$LOG_FILE" 2>&1; then
  printf 'Indexing completed successfully. Log: %s\n' "$LOG_FILE"
else
  status="$?"
  printf 'Indexing failed with exit code %s. Last 120 log lines:\n' "$status" >&2
  tail -n 120 "$LOG_FILE" >&2
  exit "$status"
fi
