#!/usr/bin/env sh
set -eu

export FLY_API_TOKEN=test-secret-token

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' HUP INT TERM
RAW_READER_MACHINES_JSON_FILE="$tmp/reader-machines.json"
export RAW_READER_MACHINES_JSON_FILE
jq -n '[{
    id:"d89303da029328",
  state:"started",
  config:{
    env:{RAW_SEARCH_BIND:"[::]:8108"},
    services:[{internal_port:8108,protocol:"tcp"}]
  }
}]' > "$RAW_READER_MACHINES_JSON_FILE"

old_entrypoint="$(printf 'old entrypoint\n' | base64 | tr -d '\n')"
old_list="$(printf '@old\n' | base64 | tr -d '\n')"
jq -n \
  --arg old_entrypoint "$old_entrypoint" \
  --arg old_list "$old_list" '[{
    id:"machine-1",
    instance_id:"instance-1",
    host_status:"ok",
    name:"ytsearch-queue-raw",
    state:"stopped",
    config:{
      image:"registry.invalid/old:one",
      env:{
        YTSEARCH_MACHINE_ROLE:"queue-indexer",
        QUEUE_TARGET_BACKEND:"raw",
        RAW_SEARCH_API_URL:"http://stopped-canary.internal:8108",
        QUEUE_ONE_REQUEST_PER_PASS:"1"
      },
      init:{entrypoint:["/bin/sh"],cmd:["-c","sleep 3600"],exec:["/bin/false"]},
      files:[
        {guest_path:"/run/secrets/youtube-api-key.txt",raw_value:$old_entrypoint},
        {guest_path:"/run/secrets/youtube-api-key.txt",secret_name:"YOUTUBE_API_KEY_FILE_B64"},
        {guest_path:"/run/secrets/yt-dlp-cookies.txt",secret_name:"YT_DLP_COOKIES_FILE_B64"},
        {guest_path:"/usr/local/bin/cloud-index-queue",raw_value:$old_entrypoint},
        {guest_path:"/tmp/daily-indexed-channels.txt",raw_value:$old_list}
      ],
      restart:{policy:"always"},
      metadata:{lifecycle:"temporary-builder-gate"}
    }
  }]' > "$tmp/machines.json"

MACHINES_JSON_FILE="$tmp/machines.json" \
RECONCILED_CONFIG_FILE="$tmp/config.json" \
INDEXER_IMAGE="registry.invalid/current:two" \
"$SCRIPT_DIR/reconcile-raw-queue-worker.sh" > "$tmp/output"

jq -e '
  .image == "registry.invalid/current:two"
  and .env.RAW_SEARCH_API_URL == "http://d89303da029328.vm.ytsearch-raw-production.internal:8108"
  and .env.QUEUE_ONE_REQUEST_PER_PASS == "0"
  and .env.LIMIT == "1000"
  and .env.DAILY_UPDATE_ENABLED == "1"
  and .env.DAILY_UPDATE_INTERVAL_SECONDS == "86400"
  and .env.DAILY_UPDATE_INITIAL_DELAY_SECONDS == "0"
  and .env.DAILY_UPDATE_STATE_FILE == "/var/lib/ytsearch/daily-update.last-completed"
  and .env.IDLE_GAP_FILL_ENABLED == "1"
  and ((.env.IDLE_GAP_FILL_MAX_PASSES | tonumber) > 0)
  and .env.IDLE_GAP_FILL_BATCH_PASSES == "4"
  and .env.IDLE_GAP_FILL_STATE_FILE == "/var/lib/ytsearch/idle-gap-fill.cursor"
  and .env.IDLE_GAP_FILL_INTERVAL_SECONDS == "21600"
  and .env.ROUTINE_PUBLICATION_INTERVAL_SECONDS == "21600"
  and .env.RAW_LAST_PUBLICATION_FILE == "/var/lib/ytsearch/raw-publication.last-completed"
  and .env.RAW_PENDING_PUBLICATION_FILE == "/var/lib/ytsearch/raw-publication.pending"
  and .init.entrypoint == ["/bin/sh"]
  and .init.cmd == ["/usr/local/bin/cloud-index-queue"]
  and (.init | has("exec") | not)
  and .restart.policy == "no"
  and .metadata.lifecycle == "on-demand-queue-worker"
  and (.files | length) == 4
  and ([.files[] | select(
    .guest_path == "/run/secrets/youtube-api-key.txt"
    and .secret_name == "YOUTUBE_API_KEY_FILE_B64"
  )] | length) == 1
  and ([.files[] | select(.guest_path == "/run/secrets/youtube-api-key.txt")] | length) == 1
  and ([.files[] | select(
    .guest_path == "/run/secrets/yt-dlp-cookies.txt"
    and .secret_name == "YT_DLP_COOKIES_FILE_B64"
  )] | length) == 1
  and ([.files[] | select(.guest_path == "/usr/local/bin/cloud-index-all")] | length) == 0
  and ([.files[] | select(.guest_path == "/tmp/indexed-channels.txt")] | length) == 0
' "$tmp/config.json" >/dev/null

jq -r 'first(.files[] | select(.guest_path == "/usr/local/bin/cloud-index-queue")) | .raw_value' \
  "$tmp/config.json" | base64 -d > "$tmp/entrypoint"
cmp -s "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh" "$tmp/entrypoint"
jq -r 'first(.files[] | select(.guest_path == "/tmp/daily-indexed-channels.txt")) | .raw_value' \
  "$tmp/config.json" | base64 -d > "$tmp/creators"
cmp -s "$SCRIPT_DIR/indexed-channels.txt" "$tmp/creators"

if RAW_SEARCH_API_URL="http://ytsearch-raw-production.internal:8108" \
  MACHINES_JSON_FILE="$tmp/machines.json" \
  RECONCILED_CONFIG_FILE="$tmp/app-wide-config.json" \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must reject an app-wide Raw reader URL" >&2
  exit 1
fi

jq '.[0].config.env.YTSEARCH_MACHINE_ROLE = "queue-indexer-migration-hold"' \
  "$tmp/machines.json" > "$tmp/held.json"
MACHINES_JSON_FILE="$tmp/held.json" \
RECONCILED_CONFIG_FILE="$tmp/held-config.json" \
INDEXER_IMAGE="registry.invalid/current:two" \
"$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null
jq -e '
  .env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
  and .metadata.lifecycle == "on-demand-queue-worker"
' "$tmp/held-config.json" >/dev/null

jq '.[0].state = "started"' "$tmp/machines.json" > "$tmp/started.json"
if MACHINES_JSON_FILE="$tmp/started.json" \
  RECONCILED_CONFIG_FILE="$tmp/started-config.json" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must reject a running worker" >&2
  exit 1
fi

jq '. + .[0:1]' "$tmp/machines.json" > "$tmp/duplicate.json"
if MACHINES_JSON_FILE="$tmp/duplicate.json" \
  RECONCILED_CONFIG_FILE="$tmp/duplicate-config.json" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must reject duplicate workers" >&2
  exit 1
fi

mkdir -p "$tmp/fake-bin"
cat > "$tmp/fly-config.yml" <<'YAML'
access_token: must-not-be-read-as-a-version
app_secrets_minvers:
    another-app: 1
    ytsearch-indexer-raw-production: 104052993
last_login: never
YAML
cat > "$tmp/fake-bin/flyctl" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "auth token")
    printf 'test-refreshed-token\n'
    ;;
  "machine list")
    cat "$FAKE_FLY_STATE"
    ;;
  "machine start")
    jq '.[0].state = "started"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    ;;
  "machine wait")
    case " $* " in
      *" --state stopped "*)
        if [ "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = "stopping" ]; then
          jq '.[0].state = "stopped"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
          mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
        fi
        [ "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = "stopped" ]
        ;;
      *" --state settled "*)
        case "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" in
          created|stopped) ;;
          *) exit 1 ;;
        esac
        ;;
      *)
        [ "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = "started" ]
        ;;
    esac
    ;;
  "logs --app")
    [ "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = "started" ]
    [ "${FAKE_RUNTIME_PREFLIGHT_FAILURE:-0}" != "1" ]
    generation="$(jq -r '.[0].config.env.CREDENTIAL_PREFLIGHT_GENERATION' "$FAKE_FLY_STATE")"
    printf 'cloud-queue-indexer: credential preflight passed generation=%s\n' "$generation"
    printf 'runtime preflight observed\n' >> "$FAKE_FLY_LOG"
    ;;
  "secrets list")
    case "${FAKE_SECRETS_STATUS:-ready}" in
      ready)
        printf '[{"name":"YOUTUBE_API_KEY_FILE_B64","status":"Partial"},{"name":"YT_DLP_COOKIES_FILE_B64","status":"Partial"}]\n'
        ;;
      missing)
        printf '[{"name":"UNRELATED_SECRET","status":"Deployed"}]\n'
        ;;
      unknown)
        printf '[{"name":"YOUTUBE_API_KEY_FILE_B64","status":"Unknown"},{"name":"YT_DLP_COOKIES_FILE_B64","status":"Partial"}]\n'
        ;;
      staged)
        printf '[{"name":"YOUTUBE_API_KEY_FILE_B64","status":"Staged"},{"name":"YT_DLP_COOKIES_FILE_B64","status":"Partial"}]\n'
        ;;
      *) exit 1 ;;
    esac
    ;;
  "secrets sync")
    if [ "${FAKE_SECRET_SYNC_FAILURE:-0}" = "1" ]; then
      exit 1
    fi
    sed 's/ytsearch-indexer-raw-production: 104052993/ytsearch-indexer-raw-production: 104052994/' \
      "$FLY_CONFIG_FILE" > "$FLY_CONFIG_FILE.next"
    mv "$FLY_CONFIG_FILE.next" "$FLY_CONFIG_FILE"
    printf 'secrets synchronized\n' >> "$FAKE_FLY_LOG"
    ;;
  *)
    echo "unexpected fake flyctl command: $*" >&2
    exit 1
    ;;
esac
SH
chmod +x "$tmp/fake-bin/flyctl"
cat > "$tmp/fake-bin/curl" <<SH
#!/usr/bin/env bash
set -euo pipefail
source "$SCRIPT_DIR/lib/test-fly-fakes.sh"
SH
cat >> "$tmp/fake-bin/curl" <<'SH'
fake_curl_parse "$@"
case "$method:$url" in
  POST:*/lease)
    if [ "${FAKE_LEASE_CONFLICT:-0}" = "1" ]; then
      printf '{"error":"lease conflict"}\n' > "$output"
      printf '409'
      exit 0
    fi
    [ -n "$request" ]
    jq -e '.ttl == 120' "$request" >/dev/null
    if [ "${FAKE_START_BEFORE_LEASE:-0}" = "1" ]; then
      jq '.[0].state = "started"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
      mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    fi
    if [ "${FAKE_CONFIG_CHANGE_BEFORE_LEASE:-0}" = "1" ]; then
      jq '.[0].config.metadata.external_owner = "fresh-leased-get"' \
        "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
      mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    fi
    if [ "${FAKE_INCOMPLETE_BEFORE_LEASE:-0}" = "1" ]; then
      jq '.[0].host_status = "unavailable" | .[0].incomplete_config = true' \
        "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
      mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    fi
    printf '{"status":"success","data":{"nonce":"test-lease-nonce"}}\n' > "$output"
    : > "$FAKE_FLY_LEASE"
    printf 'lease acquired\n' >> "$FAKE_FLY_LOG"
    printf '201'
    ;;
  GET:*/machines/machine-1)
    [ -f "$FAKE_FLY_LEASE" ]
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 'machine read under lease\n' >> "$FAKE_FLY_LOG"
    printf '200'
    ;;
  POST:*/machines/machine-1)
    [ -n "$request" ] && [ -f "$FAKE_FLY_LEASE" ]
    case "$request" in
      *.json) ;;
      *) echo "machine request must use a .json path: $request" >&2; exit 1 ;;
    esac
    current_version="$(jq -r '.[0].instance_id' "$FAKE_FLY_STATE")"
    jq -e --arg version "$current_version" '
      .current_version == $version
      and .min_secrets_version == 104052994
      and .skip_launch == true
    ' "$request" >/dev/null
    if [ "${FAKE_UPDATE_CONFLICT:-0}" = "1" ]; then
      printf '{"error":"stale version"}\n' > "$output"
      printf '409'
      exit 0
    fi
    jq --slurpfile request "$request" '
      .[0].config = $request[0].config
      | .[0].instance_id = "instance-2"
      | .[0].state = "created"
    ' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 'api update\n' >> "$FAKE_FLY_LOG"
    if [ -n "${FAKE_SIGNAL_PID_FILE:-}" ]; then
      kill -TERM "$(cat "$FAKE_SIGNAL_PID_FILE")"
    fi
    printf '200'
    ;;
  POST:*/machines/machine-1/start)
    [ -f "$FAKE_FLY_LEASE" ]
    [ "$(jq -r '.[0].state' "$FAKE_FLY_STATE")" = "created" ]
    jq '.[0].state = "started"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 'api start\n' >> "$FAKE_FLY_LOG"
    if [ "${FAKE_START_RESPONSE_LOST:-0}" = "1" ]; then
      exit 7
    fi
    printf '200'
    ;;
  POST:*/machines/machine-1/stop)
    [ -f "$FAKE_FLY_LEASE" ]
    jq --arg async "${FAKE_ASYNC_STOP:-0}" \
      '.[0].state = (if $async == "1" then "stopping" else "stopped" end)' \
      "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    printf 'api stop\n' >> "$FAKE_FLY_LOG"
    printf '200'
    ;;
  DELETE:*/lease)
    [ -f "$FAKE_FLY_LEASE" ]
    rm -f "$FAKE_FLY_LEASE"
    printf 'lease released\n' >> "$FAKE_FLY_LOG"
    printf '200'
    ;;
  *)
    echo "unexpected fake curl request: $method $url" >&2
    exit 1
    ;;
esac
SH
chmod +x "$tmp/fake-bin/curl"
cp "$tmp/machines.json" "$tmp/fly-state.json"
: > "$tmp/fly.log"
: > "$tmp/curl.log"
PATH="$tmp/fake-bin:$PATH" \
FAKE_FLY_LOG="$tmp/fly.log" \
FAKE_FLY_STATE="$tmp/fly-state.json" \
FAKE_FLY_LEASE="$tmp/fly-lease" \
FAKE_CURL_LOG="$tmp/curl.log" \
FAKE_CONFIG_CHANGE_BEFORE_LEASE=1 \
FLY_CONFIG_FILE="$tmp/fly-config.yml" \
APPLY_QUEUE_WORKER_RECONCILIATION=1 \
INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null
jq -e '
  .[0].state == "created"
  and .[0].instance_id == "instance-2"
  and .[0].config.init.cmd == ["/usr/local/bin/cloud-index-queue"]
  and .[0].config.metadata.external_owner == "fresh-leased-get"
' \
  "$tmp/fly-state.json" >/dev/null
if grep -E 'test-secret-token|test-lease-nonce' "$tmp/curl.log"; then
  echo "Fly credentials must not appear in curl arguments" >&2
  exit 1
fi
grep -Fx 'lease acquired' "$tmp/fly.log" >/dev/null
grep -Fx 'lease released' "$tmp/fly.log" >/dev/null

: > "$tmp/fly.log"
PATH="$tmp/fake-bin:$PATH" \
FAKE_FLY_LOG="$tmp/fly.log" \
FAKE_FLY_STATE="$tmp/fly-state.json" \
FAKE_FLY_LEASE="$tmp/fly-lease" \
FAKE_CURL_LOG="$tmp/curl.log" \
FLY_CONFIG_FILE="$tmp/fly-config.yml" \
INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/fly-run-queue-indexers.sh" >/dev/null
jq -e '.[0].state == "started"' "$tmp/fly-state.json" >/dev/null
update_line="$(grep -n '^api update$' "$tmp/fly.log" | cut -d: -f1)"
settled_line="$(grep -n 'machine wait .*--state settled' "$tmp/fly.log" | cut -d: -f1)"
api_start_line="$(grep -n '^api start$' "$tmp/fly.log" | cut -d: -f1)"
wait_line="$(grep -n 'machine wait .*--state started' "$tmp/fly.log" | cut -d: -f1)"
[ -n "$update_line" ] && [ -n "$settled_line" ] && [ -n "$api_start_line" ] && [ -n "$wait_line" ] \
  && [ "$update_line" -lt "$settled_line" ] && [ "$settled_line" -lt "$api_start_line" ] \
  && [ "$api_start_line" -lt "$wait_line" ] || {
  echo "the leased update must start the worker before readiness is checked" >&2
  exit 1
}
if grep -Eq '^machine (run|create|start) ' "$tmp/fly.log"; then
  echo "the existing-worker path must atomically launch through the leased update" >&2
  exit 1
fi

cp "$tmp/machines.json" "$tmp/race-state.json"
: > "$tmp/race.log"
: > "$tmp/race-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/race.log" \
  FAKE_FLY_STATE="$tmp/race-state.json" \
  FAKE_FLY_LEASE="$tmp/race-lease" \
  FAKE_CURL_LOG="$tmp/race-curl.log" \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  FAKE_START_BEFORE_LEASE=1 \
  APPLY_QUEUE_WORKER_RECONCILIATION=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must reject a worker started before lease acquisition" >&2
  exit 1
fi
if grep -Fx 'api update' "$tmp/race.log"; then
  echo "a concurrent start must prevent the API update" >&2
  exit 1
fi
grep -Fx 'lease released' "$tmp/race.log" >/dev/null

cp "$tmp/machines.json" "$tmp/incomplete-state.json"
: > "$tmp/incomplete.log"
: > "$tmp/incomplete-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/incomplete.log" \
  FAKE_FLY_STATE="$tmp/incomplete-state.json" \
  FAKE_FLY_LEASE="$tmp/incomplete-lease" \
  FAKE_CURL_LOG="$tmp/incomplete-curl.log" \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  FAKE_INCOMPLETE_BEFORE_LEASE=1 \
  APPLY_QUEUE_WORKER_RECONCILIATION=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must reject an incomplete leased Machine response" >&2
  exit 1
fi
if grep -Fx 'api update' "$tmp/incomplete.log"; then
  echo "an incomplete Machine response must prevent the API update" >&2
  exit 1
fi
grep -Fx 'lease released' "$tmp/incomplete.log" >/dev/null

cp "$tmp/machines.json" "$tmp/sync-failure-state.json"
sed 's/104052994/104052993/' "$tmp/fly-config.yml" > "$tmp/sync-failure-config.yml"
: > "$tmp/sync-failure.log"
: > "$tmp/sync-failure-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/sync-failure.log" \
  FAKE_FLY_STATE="$tmp/sync-failure-state.json" \
  FAKE_FLY_LEASE="$tmp/sync-failure-lease" \
  FAKE_CURL_LOG="$tmp/sync-failure-curl.log" \
  FLY_CONFIG_FILE="$tmp/sync-failure-config.yml" \
  FAKE_SECRET_SYNC_FAILURE=1 \
  APPLY_QUEUE_WORKER_RECONCILIATION=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must fail when authoritative secret synchronization fails" >&2
  exit 1
fi
if grep -Fx 'api update' "$tmp/sync-failure.log"; then
  echo "failed secret synchronization must prevent the API update" >&2
  exit 1
fi
grep -Fx 'lease released' "$tmp/sync-failure.log" >/dev/null

cp "$tmp/machines.json" "$tmp/preflight-failure-state.json"
: > "$tmp/preflight-failure.log"
: > "$tmp/preflight-failure-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/preflight-failure.log" \
  FAKE_FLY_STATE="$tmp/preflight-failure-state.json" \
  FAKE_FLY_LEASE="$tmp/preflight-failure-lease" \
  FAKE_CURL_LOG="$tmp/preflight-failure-curl.log" \
  FAKE_RUNTIME_PREFLIGHT_FAILURE=1 \
  CREDENTIAL_PREFLIGHT_POLL_ATTEMPTS=1 \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/fly-run-queue-indexers.sh" >/dev/null 2>&1; then
  echo "the launcher must fail when runtime credential preflight is not observed" >&2
  exit 1
fi
grep -Fx 'api start' "$tmp/preflight-failure.log" >/dev/null
grep -Fx 'api stop' "$tmp/preflight-failure.log" >/dev/null
grep -Fx 'lease released' "$tmp/preflight-failure.log" >/dev/null
jq -e '.[0].state == "stopped"' "$tmp/preflight-failure-state.json" >/dev/null

cp "$tmp/machines.json" "$tmp/lost-start-state.json"
: > "$tmp/lost-start.log"
: > "$tmp/lost-start-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/lost-start.log" \
  FAKE_FLY_STATE="$tmp/lost-start-state.json" \
  FAKE_FLY_LEASE="$tmp/lost-start-lease" \
  FAKE_CURL_LOG="$tmp/lost-start-curl.log" \
  FAKE_START_RESPONSE_LOST=1 \
  FAKE_ASYNC_STOP=1 \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/fly-run-queue-indexers.sh" >/dev/null 2>&1; then
  echo "the launcher must fail when the start response is lost" >&2
  exit 1
fi
grep -Fx 'api start' "$tmp/lost-start.log" >/dev/null
grep -Fx 'api stop' "$tmp/lost-start.log" >/dev/null
stop_wait_line="$(grep -n 'machine wait .*--state stopped' "$tmp/lost-start.log" | cut -d: -f1)"
release_line="$(grep -n '^lease released$' "$tmp/lost-start.log" | cut -d: -f1)"
[ -n "$stop_wait_line" ] && [ -n "$release_line" ] && [ "$stop_wait_line" -lt "$release_line" ] || {
  echo "cleanup must confirm stopped before releasing the Machine lease" >&2
  exit 1
}
jq -e '.[0].state == "stopped"' "$tmp/lost-start-state.json" >/dev/null

cp "$tmp/machines.json" "$tmp/conflict-state.json"
: > "$tmp/conflict.log"
: > "$tmp/conflict-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/conflict.log" \
  FAKE_FLY_STATE="$tmp/conflict-state.json" \
  FAKE_FLY_LEASE="$tmp/conflict-lease" \
  FAKE_CURL_LOG="$tmp/conflict-curl.log" \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  FAKE_UPDATE_CONFLICT=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/fly-run-queue-indexers.sh" >/dev/null 2>&1; then
  echo "the launcher must fail when the Machines API rejects its update" >&2
  exit 1
fi
if grep -Eq '^machine (wait|start) ' "$tmp/conflict.log"; then
  echo "an update conflict must not continue to launch or wait" >&2
  exit 1
fi
jq -e '.[0].state == "stopped"' "$tmp/conflict-state.json" >/dev/null
grep -Fx 'lease released' "$tmp/conflict.log" >/dev/null

cp "$tmp/machines.json" "$tmp/lease-conflict-state.json"
: > "$tmp/lease-conflict.log"
: > "$tmp/lease-conflict-curl.log"
if PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/lease-conflict.log" \
  FAKE_FLY_STATE="$tmp/lease-conflict-state.json" \
  FAKE_FLY_LEASE="$tmp/lease-conflict-lease" \
  FAKE_CURL_LOG="$tmp/lease-conflict-curl.log" \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  FAKE_LEASE_CONFLICT=1 \
  APPLY_QUEUE_WORKER_RECONCILIATION=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1; then
  echo "reconciliation must fail closed when the Machine is already leased" >&2
  exit 1
fi
if grep -Fx 'api update' "$tmp/lease-conflict.log"; then
  echo "a lease conflict must prevent the API update" >&2
  exit 1
fi

# A signal must stop the run: cleanup once on exit, and no Fly calls after it.
cp "$tmp/machines.json" "$tmp/signal-state.json"
: > "$tmp/signal.log"
: > "$tmp/signal-curl.log"
PATH="$tmp/fake-bin:$PATH" \
  FAKE_FLY_LOG="$tmp/signal.log" \
  FAKE_FLY_STATE="$tmp/signal-state.json" \
  FAKE_FLY_LEASE="$tmp/signal-lease" \
  FAKE_CURL_LOG="$tmp/signal-curl.log" \
  FAKE_SIGNAL_PID_FILE="$tmp/signal.pid" \
  FLY_CONFIG_FILE="$tmp/fly-config.yml" \
  APPLY_QUEUE_WORKER_RECONCILIATION=1 \
  START_QUEUE_WORKER_AFTER_RECONCILIATION=1 \
  INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null 2>&1 &
signal_pid=$!
printf '%s\n' "$signal_pid" > "$tmp/signal.pid"
signal_status=0
wait "$signal_pid" || signal_status=$?
[ "$signal_status" -eq 130 ] || {
  echo "a signal must end reconciliation with status 130, got $signal_status" >&2
  exit 1
}
if sed -n '/^api update$/,$p' "$tmp/signal.log" | grep -Eq '^machine |^api start$'; then
  echo "reconciliation must not continue after a signal" >&2
  exit 1
fi
[ "$(grep -cFx 'lease released' "$tmp/signal.log")" -eq 1 ] || {
  echo "a signal must release the lease exactly once" >&2
  exit 1
}

echo "raw_queue_worker_reconciliation=ok"
