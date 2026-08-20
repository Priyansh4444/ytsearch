#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPOSITORY_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)

cd "$REPOSITORY_ROOT"

if [ "${SKIP_LOCAL_RELEASE_CHECKS:-0}" != 1 ]; then
  "$SCRIPT_DIR/check-all.sh"
fi

"$SCRIPT_DIR/check-frontend-backends.sh"
"$SCRIPT_DIR/check-production-search-contract.sh"
"$SCRIPT_DIR/check-staging-machines.sh"
"$SCRIPT_DIR/check-queue-indexers.sh"
"$SCRIPT_DIR/check-raw-search-engine.sh"
"$SCRIPT_DIR/check-fly-cost-config.sh"
bun run perf:bundle

printf '%s\n' 'search release verification passed'
