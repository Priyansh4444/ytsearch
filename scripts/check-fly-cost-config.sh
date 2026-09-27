#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"

require_line() {
  file="$1"
  pattern="$2"
  message="$3"
  if ! grep -Eq "$pattern" "$file"; then
    echo "Fly cost guard failed: $message" >&2
    exit 1
  fi
}

require_line "$ROOT/fly.raw-search.toml" "memory = '2048mb'" "Raw must remain at 2 GiB"
require_line "$ROOT/fly.raw-search.toml" "cpu_kind = 'shared'" "Raw must use shared CPUs"
require_line "$ROOT/fly.raw-search.toml" "cpus = 4" "Raw must use four shared vCPUs (owner-approved 2026-09-27)"
require_line "$ROOT/fly.raw-search.toml" "RAW_SEARCH_BIND = '\[::\]:8108'" "Raw must listen on Fly private IPv6"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'INDEXER_MEMORY_MB:-2048' "recreated indexer must remain at 2 GiB"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'INDEXER_CPUS:-2' "recreated indexer must remain at two shared vCPUs"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'INDEXER_DATA_DIRECTORY:-/var/lib/ytsearch' "indexer volume must not hide baked Raw corpus files under /data"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'QUEUE_ONE_REQUEST_PER_PASS:-0' "queue requests must share one bounded publication batch"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'RAW_PUBLICATION_MIN_FREE_BYTES:-8000000000' "detached builds require eight GB of free volume space"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'IDLE_GAP_FILL_MAX_PASSES:-1' "on-demand indexer must run only one automatic gap pass"
require_line "$SCRIPT_DIR/fly-run-queue-indexers.sh" 'DAILY_UPDATE_ENABLED:-1' "on-demand indexer must run the daily catalog pass"
require_line "$SCRIPT_DIR/fly-run-queue-indexers.sh" 'DAILY_UPDATE_INITIAL_DELAY_SECONDS:-0' "first due daily pass must run on wake"
require_line "$ROOT/wrangler.jsonc" '17 \* \* \* \*' "hourly repair wake must remain configured"

echo "fly_cost_config=ok raw=shared4x/2GiB indexer=shared2x/2GiB/on-demand"
