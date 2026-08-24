#!/usr/bin/env bash
set -euo pipefail

# Run the external Raw generation builder on the existing on-demand indexer.
# The narrow argument contract lets this script plug directly into
# benchmark-publication-isolation.sh without forwarding shell text remotely.

: "${RAW_BUILDER_APP:=ytsearch-indexer-raw-production}"
: "${RAW_BUILDER_MACHINE_ID:?set RAW_BUILDER_MACHINE_ID to the exact builder Machine}"

if [[ "$#" -ne 3 || "$1" != publish || "$2" != --backend || "$3" != raw ]]; then
  printf 'usage: %s publish --backend raw\n' "$0" >&2
  exit 2
fi

state="$(flyctl machine status "$RAW_BUILDER_MACHINE_ID" --app "$RAW_BUILDER_APP" |
  awk -F': ' '/^State:/ { print $2; exit }')"
if [[ "$state" != started ]]; then
  flyctl machine start "$RAW_BUILDER_MACHINE_ID" --app "$RAW_BUILDER_APP" >/dev/null
fi

ssh_ready=0
for attempt in {1..30}; do
  if flyctl ssh console --app "$RAW_BUILDER_APP" \
    --machine "$RAW_BUILDER_MACHINE_ID" --command true >/dev/null 2>&1; then
    ssh_ready=1
    break
  fi
  sleep 2
done
if (( ssh_ready == 0 )); then
  printf 'external Raw builder did not become reachable\n' >&2
  exit 1
fi

exec flyctl ssh console --app "$RAW_BUILDER_APP" \
  --machine "$RAW_BUILDER_MACHINE_ID" \
  --command 'ytsearch publish --backend raw'
