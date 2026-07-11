#!/usr/bin/env sh
set -eu

# Check that exactly one started persistent Typesense queue worker exists.

APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-staging}"

if ! command -v flyctl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  echo "flyctl and jq are required." >&2
  exit 1
fi

machines="$(flyctl machine list --app "$APP_NAME" --json)"
printf 'backend\tmachine\tstate\trestart\n'
printf '%s' "$machines" | jq -r '
  .[]
  | select((.config.env.QUEUE_TARGET_BACKEND // "") != "")
  | [
      .config.env.QUEUE_TARGET_BACKEND,
      .name,
      .state,
      (.config.restart.policy // "")
    ]
  | @tsv
'

failed=0
for backend in typesense-fly; do
  count="$(printf '%s' "$machines" | jq --arg backend "$backend" '[.[] | select(.state == "started" and .config.env.QUEUE_TARGET_BACKEND == $backend)] | length')"
  if [ "$count" -ne 1 ]; then
    echo "Expected exactly one started $backend queue worker; found $count." >&2
    failed=1
  fi
done

legacy="$(printf '%s' "$machines" | jq '[
  .[]
  | select(
      (.state == "started")
      and ((.config.env.BACKENDS // "") | contains("meilisearch"))
      and ((.config.env.BACKENDS // "") | contains("typesense-fly"))
      and ((.config.env.QUEUE_TARGET_BACKEND // "") == "")
    )
] | length')"
if [ "$legacy" -ne 0 ]; then
  echo "Found $legacy unsafe shared queue worker(s) with no QUEUE_TARGET_BACKEND." >&2
  failed=1
fi

exit "$failed"
