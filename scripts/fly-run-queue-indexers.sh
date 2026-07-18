#!/usr/bin/env sh
set -eu

# Start or create the single on-demand Typesense queue worker.

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

run_worker() {
  backend="$1"
  machine_name="$2"
  volume_name="$3"

  configured="$(printf '%s' "$machines" | jq --arg backend "$backend" '[.[] | select(.config.env.QUEUE_TARGET_BACKEND == $backend)] | length')"
  if [ "$configured" -gt 1 ]; then
    echo "Refusing to start $backend: found $configured configured workers." >&2
    exit 1
  fi
  if [ "$configured" -eq 1 ]; then
    machine_id="$(printf '%s' "$machines" | jq -r --arg backend "$backend" 'first(.[] | select(.config.env.QUEUE_TARGET_BACKEND == $backend)) | .id')"
    state="$(printf '%s' "$machines" | jq -r --arg backend "$backend" 'first(.[] | select(.config.env.QUEUE_TARGET_BACKEND == $backend)) | .state')"
    case "$state" in
      started|starting) echo "Queue worker already active: backend=$backend state=$state" ;;
      stopped) flyctl machine start --app "$APP_NAME" "$machine_id" ;;
      *) echo "Refusing to start $backend worker in state: $state" >&2; exit 1 ;;
    esac
    return 0
  fi

  ON_DEMAND="${ON_DEMAND:-1}" \
  QUEUE_DAEMON=1 \
  BACKENDS="$backend" \
  QUEUE_TARGET_BACKEND="$backend" \
  INDEXER_MACHINE_NAME="$machine_name" \
  INDEXER_VOLUME_NAME="$volume_name" \
  JOBS="${JOBS:-1}" \
  TYPESENSE_JOBS="${TYPESENSE_JOBS:-1}" \
  QUEUE_POLL_INTERVAL_SECONDS="${QUEUE_POLL_INTERVAL_SECONDS:-30}" \
  DAILY_UPDATE_ENABLED="${DAILY_UPDATE_ENABLED:-1}" \
  DAILY_UPDATE_INITIAL_DELAY_SECONDS="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-0}" \
  DAILY_UPDATE_INTERVAL_SECONDS="${DAILY_UPDATE_INTERVAL_SECONDS:-300}" \
  "$SCRIPT_DIR/fly-run-queue-indexer.sh"
}

run_worker typesense-fly "${TYPESENSE_INDEXER_MACHINE_NAME:-ytsearch-queue-typesense}" \
  "${TYPESENSE_INDEXER_VOLUME_NAME:-ytsearch_indexer_data_ts}"
echo "Active queue worker: backend=typesense-fly"
