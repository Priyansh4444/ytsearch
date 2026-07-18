#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

fixture() {
  state="$1"
  idle="$2"
  restart="$3"
  jq -n --arg state "$state" --arg idle "$idle" --arg restart "$restart" '[{
    id:"machine-1",
    name:"ytsearch-queue-typesense",
    state:$state,
    config:{
      env:{QUEUE_TARGET_BACKEND:"typesense-fly",QUEUE_LOOP:"1",IDLE_EXIT_AFTER_SECONDS:$idle},
      restart:{policy:$restart}
    }
  }]'
}

fixture stopped 300 no >"$tmp/healthy.json"
MACHINES_JSON_FILE="$tmp/healthy.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null

fixture stopped 0 no >"$tmp/no-idle-exit.json"
if MACHINES_JSON_FILE="$tmp/no-idle-exit.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker without idle exit should fail" >&2
  exit 1
fi

jq -s '.[0] + .[1]' "$tmp/healthy.json" "$tmp/healthy.json" >"$tmp/duplicate.json"
if MACHINES_JSON_FILE="$tmp/duplicate.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "duplicate workers should fail" >&2
  exit 1
fi

echo "queue_indexer_checks=ok"
