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
require_line "$ROOT/fly.raw-search.toml" "cpus = 2" "Raw must remain at two shared vCPUs"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'INDEXER_MEMORY_MB:-2048' "recreated indexer must remain at 2 GiB"
require_line "$SCRIPT_DIR/fly-run-queue-indexer.sh" 'INDEXER_CPUS:-2' "recreated indexer must remain at two shared vCPUs"
require_line "$SCRIPT_DIR/fly-run-queue-indexers.sh" 'DAILY_UPDATE_ENABLED:-0' "on-demand indexer must not become a daily always-running worker"

echo "fly_cost_config=ok raw=shared2x/2GiB indexer=shared2x/2GiB/on-demand"
