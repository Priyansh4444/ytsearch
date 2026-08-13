#!/usr/bin/env sh
set -eu

# Check that exactly one on-demand Raw queue worker exists. A stopped
# machine is healthy while the queue is idle; Cloudflare starts it on approval.

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required." >&2
  exit 1
fi

if [ -z "${MACHINES_JSON_FILE:-}" ] && ! command -v flyctl >/dev/null 2>&1; then
  echo "flyctl is required when MACHINES_JSON_FILE is not provided." >&2
  exit 1
fi

if [ -n "${MACHINES_JSON_FILE:-}" ]; then
  machines="$(cat "$MACHINES_JSON_FILE")"
else
  machines="$(flyctl machine list --app "$APP_NAME" --json)"
fi
printf 'backend\tmachine\tstate\trestart\n'
printf '%s' "$machines" | jq -r '
  .[]
  | select((.config.env.YTSEARCH_MACHINE_ROLE // "") == "queue-indexer")
  | [
      .config.env.QUEUE_TARGET_BACKEND,
      .name,
      .state,
      (.config.restart.policy // "")
    ]
  | @tsv
'

count="$(printf '%s' "$machines" | jq '[.[] | select(
  .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
)] | length')"
if [ "$count" -ne 1 ]; then
  echo "Expected exactly one configured raw queue worker; found $count." >&2
  exit 1
fi

if ! printf '%s' "$machines" | jq -e '
  first(.[] | select(
    .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
    and .config.env.QUEUE_TARGET_BACKEND == "raw"
  ))
  | (.state == "started" or .state == "starting" or .state == "stopped")
    and (.config.env.QUEUE_LOOP == "1")
    and ((.config.env.IDLE_EXIT_AFTER_SECONDS | tonumber?) // 0) > 0
    and ((.config.env.RAW_PUBLICATION_TIMEOUT_SECONDS | tonumber?) // 0) >= 2700
    and ((.config.restart.policy // "no") == "no")
' >/dev/null; then
  echo "The raw worker does not satisfy the on-demand queue invariant." >&2
  exit 1
fi
