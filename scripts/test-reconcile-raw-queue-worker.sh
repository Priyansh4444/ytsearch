#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

old_entrypoint="$(printf 'old entrypoint\n' | base64 | tr -d '\n')"
old_list="$(printf '@old\n' | base64 | tr -d '\n')"
jq -n \
  --arg old_entrypoint "$old_entrypoint" \
  --arg old_list "$old_list" '[{
    id:"machine-1",
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
      init:{entrypoint:["/bin/sh"],cmd:["-c","sleep 3600"]},
      files:[
        {guest_path:"/run/secrets/key",secret_name:"KEY_B64"},
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
  and .env.RAW_SEARCH_API_URL == "http://ytsearch-raw-production.internal:8108"
  and .env.QUEUE_ONE_REQUEST_PER_PASS == "0"
  and .env.LIMIT == "1000"
  and .env.IDLE_GAP_FILL_ENABLED == "1"
  and ((.env.IDLE_GAP_FILL_MAX_PASSES | tonumber) > 0)
  and .env.IDLE_GAP_FILL_BATCH_PASSES == "4"
  and .env.RAW_PENDING_PUBLICATION_FILE == "/var/lib/ytsearch/raw-publication.pending"
  and .init.entrypoint == ["/bin/sh"]
  and .init.cmd == ["/usr/local/bin/cloud-index-queue"]
  and .restart.policy == "no"
  and .metadata.lifecycle == "on-demand-queue-worker"
  and ([.files[] | select(.guest_path == "/run/secrets/key" and .secret_name == "KEY_B64")] | length) == 1
' "$tmp/config.json" >/dev/null

jq -r 'first(.files[] | select(.guest_path == "/usr/local/bin/cloud-index-queue")) | .raw_value' \
  "$tmp/config.json" | base64 -d > "$tmp/entrypoint"
cmp -s "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh" "$tmp/entrypoint"
jq -r 'first(.files[] | select(.guest_path == "/tmp/daily-indexed-channels.txt")) | .raw_value' \
  "$tmp/config.json" | base64 -d > "$tmp/creators"
cmp -s "$SCRIPT_DIR/indexed-channels.txt" "$tmp/creators"

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
cat > "$tmp/fake-bin/flyctl" <<'SH'
#!/usr/bin/env sh
set -eu
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "machine list")
    cat "$FAKE_FLY_STATE"
    ;;
  "machine update")
    shift 2
    config=""
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--machine-config" ]; then
        config="$2"
        break
      fi
      shift
    done
    [ -n "$config" ]
    case "$config" in
      *.json) ;;
      *) echo "machine config must use a .json path: $config" >&2; exit 1 ;;
    esac
    jq --slurpfile config "$config" '.[0].config = $config[0]' \
      "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    ;;
  "machine start")
    jq '.[0].state = "started"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    ;;
  *)
    echo "unexpected fake flyctl command: $*" >&2
    exit 1
    ;;
esac
SH
chmod +x "$tmp/fake-bin/flyctl"
cp "$tmp/machines.json" "$tmp/fly-state.json"
: > "$tmp/fly.log"
PATH="$tmp/fake-bin:$PATH" \
FAKE_FLY_LOG="$tmp/fly.log" \
FAKE_FLY_STATE="$tmp/fly-state.json" \
APPLY_QUEUE_WORKER_RECONCILIATION=1 \
INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/reconcile-raw-queue-worker.sh" >/dev/null
jq -e '.[0].state == "stopped" and .[0].config.init.cmd == ["/usr/local/bin/cloud-index-queue"]' \
  "$tmp/fly-state.json" >/dev/null

: > "$tmp/fly.log"
PATH="$tmp/fake-bin:$PATH" \
FAKE_FLY_LOG="$tmp/fly.log" \
FAKE_FLY_STATE="$tmp/fly-state.json" \
INDEXER_IMAGE="registry.invalid/current:two" \
  "$SCRIPT_DIR/fly-run-queue-indexers.sh" >/dev/null
jq -e '.[0].state == "started"' "$tmp/fly-state.json" >/dev/null
update_line="$(grep -n '^machine update ' "$tmp/fly.log" | cut -d: -f1)"
start_line="$(grep -n '^machine start ' "$tmp/fly.log" | cut -d: -f1)"
[ -n "$update_line" ] && [ -n "$start_line" ] && [ "$update_line" -lt "$start_line" ] || {
  echo "the worker must finish reconciliation before it starts" >&2
  exit 1
}
if grep -Eq '^machine (run|create) ' "$tmp/fly.log"; then
  echo "the existing-worker path must not create a Machine" >&2
  exit 1
fi

echo "raw_queue_worker_reconciliation=ok"
