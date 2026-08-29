#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${INDEXER_APP_NAME:-ytsearch-indexer-raw-production}"
RAW_SEARCH_URL="${RAW_SEARCH_API_URL:-http://ytsearch-raw-production.internal:8108}"
CREATOR_LIST_FILE="${CREATOR_LIST_FILE:-$SCRIPT_DIR/indexed-channels.txt}"
ENTRYPOINT_FILE="${ENTRYPOINT_FILE:-$SCRIPT_DIR/cloud-index-queue-entrypoint.sh}"
APPLY="${APPLY_QUEUE_WORKER_RECONCILIATION:-0}"
START_AFTER_RECONCILIATION="${START_QUEUE_WORKER_AFTER_RECONCILIATION:-0}"

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
  (.config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
    or .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer-migration-hold")
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
)] | length')"
if [ "$worker_count" -ne 1 ]; then
  echo "Expected exactly one existing Raw queue worker; found $worker_count." >&2
  exit 1
fi
worker="$(printf '%s' "$machines" | jq -c 'first(.[] | select(
  (.config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer"
    or .config.env.YTSEARCH_MACHINE_ROLE == "queue-indexer-migration-hold")
  and .config.env.QUEUE_TARGET_BACKEND == "raw"
))')"
machine_id="$(printf '%s' "$worker" | jq -r '.id')"
machine_state="$(printf '%s' "$worker" | jq -r '.state')"
machine_version="$(printf '%s' "$worker" | jq -r '.instance_id // empty')"
case "$machine_state" in
  stopped|created) ;;
  *)
  echo "Refusing to reconcile Raw queue worker $machine_id while state=$machine_state." >&2
  exit 1
  ;;
esac

if [ "$APPLY" = "1" ] && [ -z "${INDEXER_IMAGE:-}" ]; then
  echo "Set INDEXER_IMAGE to the verified current Raw indexer image before applying." >&2
  exit 1
fi
if [ "$APPLY" = "1" ] && ! command -v curl >/dev/null 2>&1; then
  echo "curl is required when applying the reconciliation." >&2
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
preflight_generation="$source_commit-$machine_version"

config_file="${RECONCILED_CONFIG_FILE:-$(mktemp --suffix=.json)}"
cleanup_config=0
request_file=""
response_file=""
auth_header_file=""
lease_header_file=""
lease_request_file=""
lease_response_file=""
lease_acquired=0
started_by_reconciliation=0
reconciliation_succeeded=0
fly_api_hostname=""
cleanup_files() {
  if [ "$started_by_reconciliation" -eq 1 ] && [ "$reconciliation_succeeded" -ne 1 ]; then
    stop_status="$(curl --silent --show-error \
      --output /dev/null \
      --write-out '%{http_code}' \
      --request POST \
      --header "@$auth_header_file" \
      --header "@$lease_header_file" \
      "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id/stop" || true)"
    case "$stop_status" in
      200) ;;
      *) echo "Warning: fail-closed Raw queue worker stop returned status=$stop_status." >&2 ;;
    esac
    if ! flyctl machine wait \
      --app "$APP_NAME" \
      --state stopped \
      --wait-timeout 30s \
      "$machine_id" >/dev/null; then
      echo "Warning: Raw queue worker stop could not be confirmed before lease release." >&2
    fi
  fi
  if [ "$lease_acquired" -eq 1 ]; then
    release_status="$(curl --silent --show-error \
      --output /dev/null \
      --write-out '%{http_code}' \
      --request DELETE \
      --header "@$auth_header_file" \
      --header "@$lease_header_file" \
      "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id/lease" || true)"
    case "$release_status" in
      200|204) ;;
      *) echo "Warning: Raw queue worker lease release returned status=$release_status; it will expire automatically." >&2 ;;
    esac
  fi
  if [ "$cleanup_config" -eq 1 ]; then
    rm -f "$config_file"
  fi
  if [ -n "$request_file" ]; then
    rm -f "$request_file"
  fi
  if [ -n "$response_file" ]; then
    rm -f "$response_file"
  fi
  if [ -n "$auth_header_file" ]; then
    rm -f "$auth_header_file"
  fi
  if [ -n "$lease_header_file" ]; then
    rm -f "$lease_header_file"
  fi
  if [ -n "$lease_request_file" ]; then
    rm -f "$lease_request_file"
  fi
  if [ -n "$lease_response_file" ]; then
    rm -f "$lease_response_file"
  fi
}
if [ -z "${RECONCILED_CONFIG_FILE:-}" ]; then
  cleanup_config=1
fi
trap cleanup_files EXIT HUP INT TERM

write_reconciled_config() {
  jq \
    --arg image "$image" \
    --arg raw_url "$RAW_SEARCH_URL" \
    --arg entrypoint "$entrypoint_b64" \
    --arg creator_list "$creator_list_b64" \
    --arg max_passes "${IDLE_GAP_FILL_MAX_PASSES:-$creator_count}" \
    --arg max_new_videos "${IDLE_GAP_FILL_MAX_NEW_VIDEOS:-10}" \
    --arg source_commit "$source_commit" \
    --arg preflight_generation "$preflight_generation" '
  def upsert_file($path; $raw):
    .config.files = (
      ((.config.files // []) | map(select(.guest_path != $path)))
      + [{guest_path: $path, raw_value: $raw}]
    );
  .config.files = [
    {
      guest_path: "/run/secrets/youtube-api-key.txt",
      secret_name: "YOUTUBE_API_KEY_FILE_B64"
    },
    {
      guest_path: "/run/secrets/yt-dlp-cookies.txt",
      secret_name: "YT_DLP_COOKIES_FILE_B64"
    }
  ]
  |
  .config.image = $image
  | .config.env += {
      YTSEARCH_MACHINE_ROLE: "queue-indexer",
      BACKENDS: "raw",
      QUEUE_TARGET_BACKEND: "raw",
      QUEUE_LOOP: "1",
      QUEUE_ONE_REQUEST_PER_PASS: "0",
      LIMIT: "1000",
      RAW_SEARCH_API_URL: $raw_url,
      YOUTUBE_API_KEY_FILE: "/run/secrets/youtube-api-key.txt",
      YT_DLP_COOKIES_FILE: "/run/secrets/yt-dlp-cookies.txt",
      CREDENTIAL_PREFLIGHT_MARKER: "/run/ytsearch-credential-preflight.ok",
      CREDENTIAL_PREFLIGHT_GENERATION: $preflight_generation,
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
  | del(.config.init.exec)
  | .config.restart.policy = "no"
  | .config.metadata += {
      lifecycle: "on-demand-queue-worker",
      source_commit: $source_commit
    }
  | upsert_file("/usr/local/bin/cloud-index-queue"; $entrypoint)
  | upsert_file("/tmp/daily-indexed-channels.txt"; $creator_list)
  | .config
'
}

printf '%s' "$worker" | write_reconciled_config > "$config_file"

if [ "$APPLY" != "1" ]; then
  echo "Validated reconciliation config for machine=$machine_id creators=$creator_count apply=0"
  echo "Set APPLY_QUEUE_WORKER_RECONCILIATION=1 to update the existing stopped Machine."
  exit 0
fi

request_file="$(mktemp --suffix=.json)"
response_file="$(mktemp --suffix=.json)"
auth_header_file="$(mktemp)"
chmod 600 "$auth_header_file"
if [ -z "$machine_version" ]; then
  echo "The Raw queue worker is missing the version required for a safe update." >&2
  exit 1
fi
fly_api_token="${FLY_API_TOKEN:-$(flyctl auth token)}"
if [ -z "$fly_api_token" ]; then
  echo "Could not obtain a Fly API token." >&2
  exit 1
fi
token_without_line_breaks="$(printf '%s' "$fly_api_token" | tr -d '\r\n')"
if [ "$token_without_line_breaks" != "$fly_api_token" ]; then
  echo "Refusing to use a Fly API token containing a line break." >&2
  exit 1
fi
printf 'Authorization: Bearer %s\n' "$fly_api_token" > "$auth_header_file"
fly_api_hostname="${FLY_API_HOSTNAME:-https://api.machines.dev}"

lease_request_file="$(mktemp --suffix=.json)"
lease_response_file="$(mktemp --suffix=.json)"
jq -n '{description: "ytsearch Raw queue worker reconciliation", ttl: 120}' \
  > "$lease_request_file"
lease_status="$(curl --silent --show-error \
  --output "$lease_response_file" \
  --write-out '%{http_code}' \
  --request POST \
  --header "@$auth_header_file" \
  --header 'Content-Type: application/json' \
  --data-binary "@$lease_request_file" \
  "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id/lease")"
if [ "$lease_status" != "201" ]; then
  echo "Could not exclusively lease Raw queue worker $machine_id: status=$lease_status" >&2
  exit 1
fi
lease_nonce="$(jq -er '.data.nonce | select(type == "string" and length > 0)' "$lease_response_file")" || {
  echo "The Fly lease response did not contain a nonce." >&2
  exit 1
}
lease_header_file="$(mktemp)"
chmod 600 "$lease_header_file"
printf 'fly-machine-lease-nonce: %s\n' "$lease_nonce" > "$lease_header_file"
lease_acquired=1

fresh_status="$(curl --silent --show-error \
  --output "$response_file" \
  --write-out '%{http_code}' \
  --request GET \
  --header "@$auth_header_file" \
  --header "@$lease_header_file" \
  "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id")"
if [ "$fresh_status" != "200" ]; then
  echo "Could not re-read leased Raw queue worker $machine_id: status=$fresh_status" >&2
  exit 1
fi
fresh_state="$(jq -r '.state // empty' "$response_file")"
fresh_version="$(jq -r '.instance_id // empty' "$response_file")"
if ! jq -e '
  .host_status == "ok"
  and (.config | type) == "object"
  and ((.incomplete_config // false) | not)
' "$response_file" >/dev/null; then
  echo "Refusing to replace Raw queue worker $machine_id from an incomplete Machine response." >&2
  exit 1
fi
case "$fresh_state" in
  stopped|created) ;;
  *)
    echo "Refusing to reconcile leased Raw queue worker $machine_id while state=$fresh_state." >&2
    exit 1
    ;;
esac
if [ "$fresh_version" != "$machine_version" ]; then
  echo "Raw queue worker $machine_id changed before its lease was acquired; retry reconciliation." >&2
  exit 1
fi

write_reconciled_config < "$response_file" > "$config_file"
if ! flyctl secrets sync --stage --app "$APP_NAME" >/dev/null; then
  echo "Could not synchronize the authoritative Fly secrets version for $APP_NAME." >&2
  exit 1
fi
min_secrets_version=""
fly_config_file="${FLY_CONFIG_FILE:-${FLY_CONFIG_DIR:-$HOME/.fly}/config.yml}"
if [ -r "$fly_config_file" ]; then
  min_secrets_version="$(awk -v app="$APP_NAME" '
    /^app_secrets_minvers:/ { in_minvers = 1; next }
    in_minvers && /^[^[:space:]]/ { exit }
    in_minvers {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      key = line
      sub(/:.*/, "", key)
      if (key == app) {
        sub(/^[^:]*:[[:space:]]*/, "", line)
        print line
        exit
      }
    }
  ' "$fly_config_file")"
fi
case "$min_secrets_version" in
  ''|*[!0-9]*)
    echo "Fly did not persist the synchronized secrets version for $APP_NAME." >&2
    exit 1
    ;;
esac
jq -n \
  --slurpfile config "$config_file" \
  --arg current_version "$fresh_version" \
  --argjson min_secrets_version "$min_secrets_version" \
  '{
    config: $config[0],
    skip_launch: true,
    current_version: $current_version,
    min_secrets_version: $min_secrets_version
  }' \
  > "$request_file"

http_status="$(curl --silent --show-error \
  --output "$response_file" \
  --write-out '%{http_code}' \
  --request POST \
  --header "@$auth_header_file" \
  --header "@$lease_header_file" \
  --header 'Content-Type: application/json' \
  --data-binary "@$request_file" \
  "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id")"
if [ "$http_status" != "200" ]; then
  error_message="$(jq -r '.error // .message // "unknown Machines API error"' "$response_file" 2>/dev/null || true)"
  echo "Machines API update failed: status=$http_status error=$error_message" >&2
  exit 1
fi
flyctl machine wait \
  --app "$APP_NAME" \
  --state settled \
  --wait-timeout 30s \
  "$machine_id" >/dev/null

if [ "$START_AFTER_RECONCILIATION" = "1" ]; then
  started_by_reconciliation=1
  start_status="$(curl --silent --show-error \
    --output "$response_file" \
    --write-out '%{http_code}' \
    --request POST \
    --header "@$auth_header_file" \
    --header "@$lease_header_file" \
    "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id/start")"
  if [ "$start_status" != "200" ]; then
    start_error="$(jq -r '.error // .message // "unknown Machines API start error"' "$response_file" 2>/dev/null || true)"
    echo "Could not start the reconciled Raw queue worker: status=$start_status error=$start_error" >&2
    exit 1
  fi
  flyctl machine wait --app "$APP_NAME" --state started --wait-timeout 30s "$machine_id"
  REQUIRED_QUEUE_WORKER_STATES=started "$SCRIPT_DIR/check-queue-indexers.sh"
  preflight_attempts="${CREDENTIAL_PREFLIGHT_POLL_ATTEMPTS:-10}"
  case "$preflight_attempts" in
    ''|*[!0-9]*|0)
      echo "CREDENTIAL_PREFLIGHT_POLL_ATTEMPTS must be a positive integer." >&2
      exit 1
      ;;
  esac
  preflight_observed=0
  preflight_log_line="cloud-queue-indexer: credential preflight passed generation=$preflight_generation"
  preflight_attempt=1
  while [ "$preflight_attempt" -le "$preflight_attempts" ]; do
    if flyctl logs \
      --app "$APP_NAME" \
      --machine "$machine_id" \
      --no-tail 2>/dev/null \
      | grep -F "$preflight_log_line" >/dev/null; then
      preflight_observed=1
      break
    fi
    sleep 1
    preflight_attempt=$((preflight_attempt + 1))
  done
  if [ "$preflight_observed" -ne 1 ]; then
    echo "The Raw queue worker credential preflight handshake was not observed." >&2
    exit 1
  fi
  sleep 2
  stable_status="$(curl --silent --show-error \
    --output "$response_file" \
    --write-out '%{http_code}' \
    --request GET \
    --header "@$auth_header_file" \
    --header "@$lease_header_file" \
    "$fly_api_hostname/v1/apps/$APP_NAME/machines/$machine_id")"
  if [ "$stable_status" != "200" ] \
    || ! jq -e '.state == "started"' "$response_file" >/dev/null; then
    echo "The Raw queue worker did not remain stable after credential preflight." >&2
    exit 1
  fi
else
  REQUIRED_QUEUE_WORKER_STATES=created,stopped "$SCRIPT_DIR/check-queue-indexers.sh"
fi

reconciliation_succeeded=1
echo "Reconciled Raw queue worker machine=$machine_id creators=$creator_count started=$START_AFTER_RECONCILIATION"
