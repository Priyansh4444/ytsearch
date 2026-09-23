#!/usr/bin/env sh
set -eu

# Start or create the single on-demand Raw queue worker.

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"

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
      stopped|created)
        if [ -z "${INDEXER_IMAGE:-}" ]; then
          echo "Set INDEXER_IMAGE to the verified current indexer image before reconciling the stopped worker." >&2
          exit 1
        fi
        APPLY_QUEUE_WORKER_RECONCILIATION=1 \
        START_QUEUE_WORKER_AFTER_RECONCILIATION=1 \
        INDEXER_IMAGE="$INDEXER_IMAGE" \
          "$SCRIPT_DIR/reconcile-raw-queue-worker.sh"
        ;;
      *) echo "Refusing to start $backend worker in state: $state" >&2; exit 1 ;;
    esac
    if [ "$state" = "started" ] || [ "$state" = "starting" ]; then
      flyctl machine wait --app "$APP_NAME" --state started --wait-timeout 30s "$machine_id"
      REQUIRED_QUEUE_WORKER_STATES=started "$SCRIPT_DIR/check-queue-indexers.sh"
    fi
    return 0
  fi

  ON_DEMAND="${ON_DEMAND:-1}" \
  QUEUE_DAEMON=1 \
  BACKENDS="$backend" \
  QUEUE_TARGET_BACKEND="$backend" \
  INDEXER_MACHINE_NAME="$machine_name" \
  INDEXER_VOLUME_NAME="$volume_name" \
  JOBS="${JOBS:-1}" \
  RAW_JOBS="${RAW_JOBS:-1}" \
  QUEUE_POLL_INTERVAL_SECONDS="${QUEUE_POLL_INTERVAL_SECONDS:-30}" \
  IDLE_GAP_FILL_ENABLED="${IDLE_GAP_FILL_ENABLED:-1}" \
  IDLE_GAP_FILL_MAX_PASSES="${IDLE_GAP_FILL_MAX_PASSES:-1}" \
  IDLE_GAP_FILL_MAX_NEW_VIDEOS="${IDLE_GAP_FILL_MAX_NEW_VIDEOS:-10}" \
  DAILY_UPDATE_ENABLED="${DAILY_UPDATE_ENABLED:-1}" \
  DAILY_UPDATE_INITIAL_DELAY_SECONDS="${DAILY_UPDATE_INITIAL_DELAY_SECONDS:-0}" \
  DAILY_UPDATE_INTERVAL_SECONDS="${DAILY_UPDATE_INTERVAL_SECONDS:-86400}" \
  "$SCRIPT_DIR/fly-run-queue-indexer.sh"
}

run_worker raw "${RAW_INDEXER_MACHINE_NAME:-ytsearch-queue-raw}" \
  "${RAW_INDEXER_VOLUME_NAME:-ytsearch_indexer_data_raw}"
echo "Active queue worker: backend=raw"
