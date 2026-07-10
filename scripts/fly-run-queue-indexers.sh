#!/usr/bin/env sh
set -eu

# Start the two persistent queue workers. Each worker consumes and acknowledges
# only rows for its backend, which keeps the JSONL queue safe without a lock.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-staging}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_command flyctl
require_command jq

machines="$(flyctl machine list --app "$APP_NAME" --json)"
legacy_count="$(printf '%s' "$machines" | jq '[
  .[]
  | select(
      (.state == "started")
      and ((.config.env.BACKENDS // "") | contains("meilisearch"))
      and ((.config.env.BACKENDS // "") | contains("typesense-fly"))
      and ((.config.env.QUEUE_TARGET_BACKEND // "") == "")
    )
] | length')"
if [ "$legacy_count" -ne 0 ]; then
  echo "Refusing to start split workers while $legacy_count legacy shared worker(s) are running." >&2
  echo "Confirm they are idle, stop them, then rerun this command." >&2
  exit 1
fi

run_worker() {
  backend="$1"
  machine_name="$2"
  volume_name="$3"

  started="$(printf '%s' "$machines" | jq --arg backend "$backend" '[.[] | select(.state == "started" and .config.env.QUEUE_TARGET_BACKEND == $backend)] | length')"
  if [ "$started" -eq 1 ]; then
    echo "Queue worker already started: backend=$backend"
    return 0
  fi
  if [ "$started" -gt 1 ]; then
    echo "Refusing to start $backend: found $started started workers." >&2
    exit 1
  fi

  QUEUE_DAEMON=1 \
  BACKENDS="$backend" \
  QUEUE_TARGET_BACKEND="$backend" \
  INDEXER_MACHINE_NAME="$machine_name" \
  INDEXER_VOLUME_NAME="$volume_name" \
  JOBS="${JOBS:-1}" \
  QUEUE_POLL_INTERVAL_SECONDS="${QUEUE_POLL_INTERVAL_SECONDS:-30}" \
  DAILY_UPDATE_ENABLED="${DAILY_UPDATE_ENABLED:-1}" \
  DAILY_UPDATE_INITIAL_DELAY_SECONDS="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-86400}" \
  DAILY_UPDATE_INTERVAL_SECONDS="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}" \
  "$SCRIPT_DIR/fly-run-queue-indexer.sh"
}

run_worker meilisearch "${MEILI_INDEXER_MACHINE_NAME:-ytsearch-queue-meili}" \
  "${MEILI_INDEXER_VOLUME_NAME:-ytsearch_indexer_data_meili}"
run_worker typesense-fly "${TYPESENSE_INDEXER_MACHINE_NAME:-ytsearch-queue-typesense}" \
  "${TYPESENSE_INDEXER_VOLUME_NAME:-ytsearch_indexer_data_ts}"

"$SCRIPT_DIR/check-queue-indexers.sh"
