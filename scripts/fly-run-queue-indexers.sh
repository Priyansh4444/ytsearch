#!/usr/bin/env sh
set -eu

# Start the active Typesense persistent queue worker. Meilisearch was retired;
# its former machine and volume are intentionally gone.

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
legacy_count="$(printf '%s' "$machines" | jq '[.[] | select(.state == "started" and ((.config.env.BACKENDS // "") | contains("meilisearch")))] | length')"
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

run_worker typesense-fly "${TYPESENSE_INDEXER_MACHINE_NAME:-ytsearch-queue-typesense}" \
  "${TYPESENSE_INDEXER_VOLUME_NAME:-ytsearch_indexer_data_ts}"
echo "Active queue worker: backend=typesense-fly"
