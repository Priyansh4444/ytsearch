#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
RAW_SEARCH_URL="${RAW_SEARCH_API_URL:-http://ytsearch-raw-production.internal:8108}"
CREATOR_LIST_FILE="${CREATOR_LIST_FILE:-$SCRIPT_DIR/indexed-channels.txt}"
ENTRYPOINT_FILE="${ENTRYPOINT_FILE:-$SCRIPT_DIR/cloud-index-queue-entrypoint.sh}"
APPLY="${APPLY_QUEUE_WORKER_RECONCILIATION:-0}"

for command in jq base64 git; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Missing required command: $command" >&2
    exit 1
  }
done
if [ -z "${MACHINES_JSON_FILE:-}" ]; then
  command -v flyctl >/dev/null 2>&1 || {
    echo "flyctl is required when MACHINES_JSON_FILE is not provided." >&2
    exit 1
  }
fi
if [ ! -s "$CREATOR_LIST_FILE" ] || [ ! -s "$ENTRYPOINT_FILE" ]; then
  echo "The queue entrypoint and canonical creator list must both be non-empty." >&2
  exit 1
fi

if [ -n "${MACHINES_JSON_FILE:-}" ]; then
  machines="$(cat "$MACHINES_JSON_FILE")"
else
  machines="$(flyctl machine list --app "$APP_NAME" --json)"
fi
worker_count="$(printf '%s' "$machines" | jq '[.[] | select(
  .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
)] | length')"
if [ "$worker_count" -ne 1 ]; then
  echo "Expected exactly one existing Raw queue worker; found $worker_count." >&2
  exit 1
fi
worker="$(printf '%s' "$machines" | jq -c 'first(.[] | select(
  .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
))')"
machine_id="$(printf '%s' "$worker" | jq -r '.id')"
machine_state="$(printf '%s' "$worker" | jq -r '.state')"
if [ "$machine_state" != "stopped" ]; then
  echo "Refusing to reconcile Raw queue worker $machine_id while state=$machine_state." >&2
  exit 1
fi

if [ "$APPLY" = "1" ] && [ -z "${INDEXER_IMAGE:-}" ]; then
  echo "Set INDEXER_IMAGE to the verified current Raw indexer image before applying." >&2
  exit 1
fi
image="${INDEXER_IMAGE:-$(printf '%s' "$worker" | jq -r '.config.image // empty')}"
if [ -z "$image" ]; then
  echo "Set INDEXER_IMAGE to the verified current Raw indexer image." >&2
  exit 1
fi
entrypoint_b64="$(base64 < "$ENTRYPOINT_FILE" | tr -d '\n')"
creator_list_b64="$(base64 < "$CREATOR_LIST_FILE" | tr -d '\n')"
creator_count="$(awk 'NF && $1 !~ /^#/ { count += 1 } END { print count + 0 }' "$CREATOR_LIST_FILE")"
source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse --short HEAD)"

config_file="${RECONCILED_CONFIG_FILE:-$(mktemp --suffix=.json)}"
cleanup_config=0
if [ -z "${RECONCILED_CONFIG_FILE:-}" ]; then
  cleanup_config=1
  trap 'rm -f "$config_file"' EXIT HUP INT TERM
fi

printf '%s' "$worker" | jq \
  --arg image "$image" \
  --arg raw_url "$RAW_SEARCH_URL" \
  --arg entrypoint "$entrypoint_b64" \
  --arg creator_list "$creator_list_b64" \
  --arg max_passes "${IDLE_GAP_FILL_MAX_PASSES:-$creator_count}" \
  --arg max_new_videos "${IDLE_GAP_FILL_MAX_NEW_VIDEOS:-10}" \
  --arg source_commit "$source_commit" '
  def upsert_file($path; $raw):
    .config.files = (
      ((.config.files // []) | map(select(.guest_path != $path)))
      + [{guest_path: $path, raw_value: $raw}]
    );
  .config.image = $image
  | .config.env += {
      YTSEARCH_MACHINE_ROLE: "queue-indexer",
      BACKENDS: "raw",
      QUEUE_TARGET_BACKEND: "raw",
      QUEUE_LOOP: "1",
      QUEUE_ONE_REQUEST_PER_PASS: "0",
      LIMIT: "1000",
      RAW_SEARCH_API_URL: $raw_url,
      RAW_PUBLICATION_TIMEOUT_SECONDS: "3600",
      RAW_PENDING_PUBLICATION_FILE: "/var/lib/ytsearch/raw-publication.pending",
      IDLE_EXIT_AFTER_SECONDS: "300",
      IDLE_GAP_FILL_ENABLED: "1",
      IDLE_GAP_FILL_MAX_PASSES: $max_passes,
      IDLE_GAP_FILL_BATCH_PASSES: "4",
      IDLE_GAP_FILL_MAX_NEW_VIDEOS: $max_new_videos,
      DAILY_UPDATE_ENABLED: "0",
      DAILY_UPDATE_LIST_FILE: "/tmp/daily-indexed-channels.txt",
      QUEUE_PRIORITY_LIST_FILE: "/tmp/daily-indexed-channels.txt"
    }
  | .config.init.entrypoint = ["/bin/sh"]
  | .config.init.cmd = ["/usr/local/bin/cloud-index-queue"]
  | .config.restart.policy = "no"
  | .config.metadata += {
      lifecycle: "on-demand-queue-worker",
      source_commit: $source_commit
    }
  | upsert_file("/usr/local/bin/cloud-index-queue"; $entrypoint)
  | upsert_file("/tmp/daily-indexed-channels.txt"; $creator_list)
  | .config
' > "$config_file"

if [ "$APPLY" != "1" ]; then
  echo "Validated reconciliation config for machine=$machine_id creators=$creator_count apply=0"
  echo "Set APPLY_QUEUE_WORKER_RECONCILIATION=1 to update the existing stopped Machine."
  exit 0
fi

flyctl machine update "$machine_id" \
  --app "$APP_NAME" \
  --machine-config "$config_file" \
  --skip-start \
  --yes

"$SCRIPT_DIR/check-queue-indexers.sh"

if [ "$cleanup_config" -eq 1 ]; then
  rm -f "$config_file"
  trap - EXIT HUP INT TERM
fi
echo "Reconciled stopped Raw queue worker machine=$machine_id creators=$creator_count"
