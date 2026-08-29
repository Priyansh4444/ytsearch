#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
entrypoint_b64="$(base64 < "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh" | tr -d '\n')"
creator_list_b64="$(base64 < "$SCRIPT_DIR/indexed-channels.txt" | tr -d '\n')"

fixture() {
  state="$1"
  idle="$2"
  restart="$3"
  jq -n \
    --arg state "$state" \
    --arg idle "$idle" \
    --arg restart "$restart" \
    --arg entrypoint "$entrypoint_b64" \
    --arg creator_list "$creator_list_b64" '[{
    id:"machine-1",
    name:"ytsearch-queue-raw",
    state:$state,
    config:{
      env:{
        YTSEARCH_MACHINE_ROLE:"queue-indexer",
        QUEUE_TARGET_BACKEND:"raw",
        QUEUE_LOOP:"1",
        QUEUE_ONE_REQUEST_PER_PASS:"0",
        LIMIT:"1000",
        IDLE_EXIT_AFTER_SECONDS:$idle,
        IDLE_GAP_FILL_ENABLED:"1",
        IDLE_GAP_FILL_MAX_PASSES:"4",
        IDLE_GAP_FILL_BATCH_PASSES:"4",
        IDLE_GAP_FILL_MAX_NEW_VIDEOS:"10",
        RAW_PUBLICATION_TIMEOUT_SECONDS:"3600",
        RAW_PENDING_PUBLICATION_FILE:"/var/lib/ytsearch/raw-publication.pending",
        RAW_SEARCH_API_URL:"http://ytsearch-raw-production.internal:8108"
      },
      init:{
        entrypoint:["/bin/sh"],
        cmd:["/usr/local/bin/cloud-index-queue"]
      },
      files:[
        {guest_path:"/run/secrets/youtube-api-key.txt",secret_name:"YOUTUBE_API_KEY_FILE_B64"},
        {guest_path:"/run/secrets/yt-dlp-cookies.txt",secret_name:"YT_DLP_COOKIES_FILE_B64"},
        {guest_path:"/usr/local/bin/cloud-index-queue",raw_value:$entrypoint},
        {guest_path:"/tmp/daily-indexed-channels.txt",raw_value:$creator_list}
      ],
      restart:{policy:$restart}
    }
  }]'
}

fixture stopped 300 no >"$tmp/healthy.json"
MACHINES_JSON_FILE="$tmp/healthy.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null
if MACHINES_JSON_FILE="$tmp/healthy.json" \
  REQUIRED_QUEUE_WORKER_STATES=started \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "a stopped worker must fail a required-started check" >&2
  exit 1
fi
fixture started 300 no >"$tmp/started.json"
MACHINES_JSON_FILE="$tmp/started.json" \
  REQUIRED_QUEUE_WORKER_STATES=started \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null
fixture created 300 no >"$tmp/created.json"
MACHINES_JSON_FILE="$tmp/created.json" \
  REQUIRED_QUEUE_WORKER_STATES=created,stopped \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null

fixture stopped 0 no >"$tmp/no-idle-exit.json"
if MACHINES_JSON_FILE="$tmp/no-idle-exit.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker without idle exit should fail" >&2
  exit 1
fi

jq '.[0].config.env.RAW_PUBLICATION_TIMEOUT_SECONDS = "1200"' \
  "$tmp/healthy.json" >"$tmp/short-publication-timeout.json"
if MACHINES_JSON_FILE="$tmp/short-publication-timeout.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker with a short Raw publication timeout should fail" >&2
  exit 1
fi

jq '.[0].config.init.cmd = ["-c", "sleep 3600"]' \
  "$tmp/healthy.json" >"$tmp/sleep-command.json"
if MACHINES_JSON_FILE="$tmp/sleep-command.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker with a placeholder sleep command should fail" >&2
  exit 1
fi

jq '.[0].config.init.exec = ["/bin/sh", "-c", "sleep 3600"]' \
  "$tmp/healthy.json" >"$tmp/stale-init-exec.json"
if MACHINES_JSON_FILE="$tmp/stale-init-exec.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker with an overriding init exec should fail" >&2
  exit 1
fi

jq '.[0].config.env.RAW_SEARCH_API_URL = "http://stopped-canary.internal:8108"' \
  "$tmp/healthy.json" >"$tmp/stale-raw-url.json"
if MACHINES_JSON_FILE="$tmp/stale-raw-url.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker targeting a stale Raw reader should fail" >&2
  exit 1
fi

jq '.[0].config.env.QUEUE_ONE_REQUEST_PER_PASS = "1"' \
  "$tmp/healthy.json" >"$tmp/uncoalesced.json"
if MACHINES_JSON_FILE="$tmp/uncoalesced.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker that disables queue publication coalescing should fail" >&2
  exit 1
fi

jq '.[0].config.env.LIMIT = "1"' "$tmp/healthy.json" >"$tmp/single-pull.json"
if MACHINES_JSON_FILE="$tmp/single-pull.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker that cannot pull a coalesced queue batch should fail" >&2
  exit 1
fi

jq '.[0].config.files |= map(if .guest_path == "/tmp/daily-indexed-channels.txt" then .raw_value = "c3RhbGUK" else . end)' \
  "$tmp/healthy.json" >"$tmp/stale-list.json"
if MACHINES_JSON_FILE="$tmp/stale-list.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker with a stale creator list should fail" >&2
  exit 1
fi

jq '.[0].config.files += [{
  guest_path:"/run/secrets/youtube-api-key.txt",
  raw_value:"c3RhbGUK"
}]' "$tmp/healthy.json" >"$tmp/duplicate-secret-path.json"
if MACHINES_JSON_FILE="$tmp/duplicate-secret-path.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "worker with an ambiguous secret file path should fail" >&2
  exit 1
fi

jq -s '.[0] + .[1]' "$tmp/healthy.json" "$tmp/healthy.json" >"$tmp/duplicate.json"
if MACHINES_JSON_FILE="$tmp/duplicate.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "duplicate workers should fail" >&2
  exit 1
fi

jq '. + [{
  id:"repair-1",
  name:"ytsearch-corpus-repair",
  state:"stopped",
  config:{
    env:{QUEUE_TARGET_BACKEND:"raw",QUEUE_LOOP:"1",IDLE_EXIT_AFTER_SECONDS:"300"},
    restart:{policy:"no"}
  }
}]' "$tmp/healthy.json" >"$tmp/repair.json"
MACHINES_JSON_FILE="$tmp/repair.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null

echo "queue_indexer_checks=ok"
