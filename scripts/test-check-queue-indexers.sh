#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' HUP INT TERM
RAW_READER_MACHINES_JSON_FILE="$tmp/reader.json"
export RAW_READER_MACHINES_JSON_FILE
jq -n '[
  {id:"d89303da029328",state:"started",config:{env:{RAW_SEARCH_BIND:"[::]:8108"},services:[{internal_port:8108,protocol:"tcp"}]}},
  {id:"847314c22d52e8",state:"stopped",config:{env:{RAW_SEARCH_BIND:"[::]:8108"},services:[{internal_port:8108,protocol:"tcp"}]}}
]' > "$RAW_READER_MACHINES_JSON_FILE"
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
        DAILY_UPDATE_ENABLED:"1",
        DAILY_UPDATE_INTERVAL_SECONDS:"86400",
        TRANSCRIBE_MISSING:"1",
        WHISPER_BINARY:"/usr/local/bin/whisper-cli",
        WHISPER_MODEL:"/usr/local/share/ytsearch-asr/ggml-base.en.bin",
        WHISPER_THREADS:"2",
        TRANSCRIPTION_BUDGET_SECONDS:"480",
        DAILY_UPDATE_INITIAL_DELAY_SECONDS:"0",
        DAILY_UPDATE_STATE_FILE:"/var/lib/ytsearch/daily-update.last-completed",
        QUEUE_ONE_REQUEST_PER_PASS:"0",
        LIMIT:"1000",
        IDLE_EXIT_AFTER_SECONDS:$idle,
        IDLE_GAP_FILL_ENABLED:"1",
        IDLE_GAP_FILL_MAX_PASSES:"4",
        IDLE_GAP_FILL_BATCH_PASSES:"4",
        IDLE_GAP_FILL_MAX_NEW_VIDEOS:"10",
        RAW_PUBLICATION_TIMEOUT_SECONDS:"3600",
        RAW_PENDING_PUBLICATION_FILE:"/var/lib/ytsearch/raw-publication.pending",
        RAW_SEARCH_API_URL:"http://d89303da029328.vm.ytsearch-raw-production.internal:8108",
        YOUTUBE_API_KEY_FILE:"/run/secrets/youtube-api-key.txt",
        YT_DLP_COOKIES_FILE:"/run/secrets/yt-dlp-cookies.txt",
        CREDENTIAL_PREFLIGHT_MARKER:"/run/ytsearch-credential-preflight.ok",
        CREDENTIAL_PREFLIGHT_GENERATION:"test-instance-1"
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
jq '.[0].config.env.RAW_SEARCH_API_URL = "http://ytsearch-raw-production.internal:8108"' \
  "$tmp/healthy.json" > "$tmp/app-wide-raw-url.json"
if MACHINES_JSON_FILE="$tmp/app-wide-raw-url.json" \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "app-wide Raw DNS can route a production worker to a started canary" >&2
  exit 1
fi
jq '.[0].config.env.RAW_SEARCH_BIND = "0.0.0.0:8108"' \
  "$RAW_READER_MACHINES_JSON_FILE" > "$tmp/ipv4-only-reader.json"
if RAW_READER_MACHINES_JSON_FILE="$tmp/ipv4-only-reader.json" \
  MACHINES_JSON_FILE="$tmp/healthy.json" \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "IPv4-only reader must fail the private-network queue path" >&2
  exit 1
fi
RAW_READER_MACHINES_JSON_FILE="$tmp/ipv4-only-reader.json" \
  MACHINES_JSON_FILE="$tmp/healthy.json" \
  QUEUE_READER_CHECK_MODE=deferred REQUIRED_QUEUE_WORKER_STATES=created,stopped \
  "$SCRIPT_DIR/check-queue-indexers.sh" > "$tmp/staged-check.txt"
if [ "$(tail -n 1 "$tmp/staged-check.txt")" != "queue_reader_check=deferred worker_stage_only=1" ]; then
  echo "a deferred check must label its limited scope" >&2
  exit 1
fi
jq '.[0].state = "started"' "$tmp/healthy.json" > "$tmp/started-during-stage.json"
if RAW_READER_MACHINES_JSON_FILE="$tmp/ipv4-only-reader.json" \
  MACHINES_JSON_FILE="$tmp/started-during-stage.json" \
  QUEUE_READER_CHECK_MODE=deferred REQUIRED_QUEUE_WORKER_STATES=started \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "a started worker must never defer reader readiness" >&2
  exit 1
fi
jq '.[0].state = "stopped" | .[1].state = "started"' \
  "$RAW_READER_MACHINES_JSON_FILE" > "$tmp/canary-only-reader.json"
if RAW_READER_MACHINES_JSON_FILE="$tmp/canary-only-reader.json" \
  MACHINES_JSON_FILE="$tmp/healthy.json" \
  "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
  echo "a started canary must not satisfy the production reader invariant" >&2
  exit 1
fi
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

for field in DAILY_UPDATE_ENABLED DAILY_UPDATE_INTERVAL_SECONDS DAILY_UPDATE_INITIAL_DELAY_SECONDS DAILY_UPDATE_STATE_FILE TRANSCRIBE_MISSING WHISPER_BINARY WHISPER_MODEL WHISPER_THREADS TRANSCRIPTION_BUDGET_SECONDS; do
  jq --arg field "$field" '.[0].config.env[$field] = "invalid"' "$tmp/healthy.json" > "$tmp/invalid-daily.json"
  if MACHINES_JSON_FILE="$tmp/invalid-daily.json" "$SCRIPT_DIR/check-queue-indexers.sh" >/dev/null 2>&1; then
    echo "invalid daily setting $field must fail" >&2
    exit 1
  fi
done

echo "queue_indexer_checks=ok"
