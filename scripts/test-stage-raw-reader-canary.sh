#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin"
source_commit="$(git -C "$SCRIPT_DIR/.." rev-parse HEAD)"
base_digest="$(printf '1%.0s' {1..64})"
resolved_digest="$(printf '2%.0s' {1..64})"
machine_id=22222222222222
image="registry.fly.io/ytsearch-raw-production:raw-overlay-${source_commit:0:12}-${base_digest:0:12}"

cat > "$tmp/state.json" <<JSON
[{
  "id":"$machine_id",
  "instance_id":"reader-version-1",
  "state":"stopped",
  "host_status":"ok",
  "incomplete_config":false,
  "private_ip":"fdaa::1",
  "image_ref":{"digest":"sha256:$resolved_digest"},
  "config":{
    "image":"registry.invalid/old:one",
    "env":{"STALE_QUEUE_MODE":"1"},
    "init":{"entrypoint":["/bin/sh"],"cmd":["/old-worker"],"swap_size_mb":512},
    "guest":{"cpu_kind":"shared","cpus":2,"memory_mb":2048},
    "metadata":{"lifecycle":"temporary-canary"},
    "mounts":[{"volume":"vol_1111111111111111","path":"/data/mutable"}],
    "restart":{"policy":"no"},
    "dns":{"skip_registration":true},
    "services":[],
    "checks":{},
    "files":[{"guest_path":"/old-worker","raw_value":"old"}]
  }
}]
JSON

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "machine list") cat "$FAKE_FLY_STATE" ;;
  "secrets sync")
    [[ "$*" == "secrets sync --stage --app ytsearch-raw-production" ]]
    ;;
  "auth token") printf 'fake-token\n' ;;
  *) printf 'unexpected flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/flyctl"

cat > "$tmp/bin/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
method=GET
output=""
request=""
url=""
auth_header=""
lease_header=""
printf '%s\n' "$*" >> "$FAKE_CURL_LOG"
while (( $# > 0 )); do
  case "$1" in
    --request) method="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    --data-binary) request="${2#@}"; shift 2 ;;
    --header)
      if [[ "$2" == @* ]]; then
        header_file="${2#@}"
        if grep -q '^Authorization:' "$header_file"; then
          auth_header="$header_file"
        elif grep -q '^fly-machine-lease-nonce:' "$header_file"; then
          lease_header="$header_file"
        fi
      fi
      shift 2
      ;;
    --write-out|--connect-timeout|--max-time) shift 2 ;;
    --silent|--show-error) shift ;;
    http://*|https://*) url="$1"; shift ;;
    *) printf 'unexpected curl argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done
[[ -n "$output" && -n "$auth_header" && -n "$url" ]]
grep -Fx 'Authorization: Bearer fake-token' "$auth_header" >/dev/null
if [[ "$url" != */lease || "$method" == DELETE ]]; then
  [[ -n "$lease_header" ]]
  grep -Fx 'fly-machine-lease-nonce: fake-nonce' "$lease_header" >/dev/null
fi
case "$method:$url" in
  POST:*/lease)
    jq -e '.ttl == 300' "$request" >/dev/null
    printf '{"data":{"nonce":"fake-nonce","expires_at":4102444800}}\n' > "$output"
    printf 201
    ;;
  GET:*/machines/22222222222222)
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 200
    ;;
  POST:*/machines/22222222222222/stop)
    jq '.[0].state = "stopped"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    printf '{}\n' > "$output"
    printf 200
    ;;
  POST:*/machines/22222222222222)
    jq -e '
      .current_version == "reader-version-1" and
      .skip_launch == true and
      .min_secrets_version == 104052994 and
      (.config.env | keys | sort) == [
        "RAW_SEARCH_BASE_CHANNELS_GZIP",
        "RAW_SEARCH_BASE_CHUNKS_GZIP",
        "RAW_SEARCH_BIND",
        "RAW_SEARCH_INDEX_PATH",
        "RAW_SEARCH_STATE_DIRECTORY",
        "RUST_LOG"
      ] and
      .config.init == {swap_size_mb:512} and
      (.config.services | length) == 0 and
      (.config.checks | length) == 0 and
      (.config.files | length) == 0
    ' "$request" >/dev/null
    cp "$request" "$FAKE_UPDATE_CAPTURE"
    jq --slurpfile request "$request" --arg digest "$FAKE_RESOLVED_DIGEST" '
      .[0].config = $request[0].config |
      .[0].instance_id = "reader-version-2" |
      .[0].state = "created" |
      .[0].image_ref.digest = ("sha256:" + $digest)
    ' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
    mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
    jq '.[0]' "$FAKE_FLY_STATE" > "$output"
    printf 200
    ;;
  DELETE:*/lease)
    printf '{}\n' > "$output"
    printf 204
    ;;
  *) printf 'unexpected curl request: %s %s\n' "$method" "$url" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/curl"

cat > "$tmp/fly-config.yml" <<'YAML'
app_secrets_minvers:
  ytsearch-raw-production: 104052994
YAML

common_env=(
  PATH="$tmp/bin:$PATH"
  FLY_CONFIG_FILE="$tmp/fly-config.yml"
  FLY_API_TOKEN=fake-token
  FLY_API_HOSTNAME=https://api.test
  FAKE_FLY_STATE="$tmp/state.json"
  FAKE_FLY_LOG="$tmp/fly.log"
  FAKE_CURL_LOG="$tmp/curl.log"
  FAKE_UPDATE_CAPTURE="$tmp/update.json"
  FAKE_RESOLVED_DIGEST="$resolved_digest"
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  BASE_IMAGE="registry.fly.io/ytsearch-raw-production:base@sha256:$base_digest"
  READER_IMAGE="$image"
)

: > "$tmp/fly.log"
: > "$tmp/curl.log"
env "${common_env[@]}" "$SCRIPT_DIR/stage-raw-reader-canary.sh" > "$tmp/dry.json"
jq -e --arg machine "$machine_id" '.apply == false and .machine == $machine' \
  "$tmp/dry.json" >/dev/null
[[ ! -s "$tmp/curl.log" ]]

env "${common_env[@]}" APPLY_RAW_READER_CANARY=stage-exact-isolated-canary \
  "$SCRIPT_DIR/stage-raw-reader-canary.sh" > "$tmp/applied.json"
jq -e --arg image "$image@sha256:$resolved_digest" '
  .kind == "raw-reader-canary" and .resolved_image == $image and
  .state == "stopped" and .isolated == true and .memory_mb == 2048
' "$tmp/applied.json" >/dev/null
jq -e '
  .[0].state == "stopped" and
  (.[0].config.env | has("STALE_QUEUE_MODE") | not) and
  (.[0].config.init | has("entrypoint") | not) and
  (.[0].config.init | has("cmd") | not) and
  (.[0].config.files | length) == 0 and
  (.[0].config.services | length) == 0
' "$tmp/state.json" >/dev/null
grep -F 'https://api.test/v1/apps/ytsearch-raw-production/machines/22222222222222' \
  "$tmp/curl.log" >/dev/null
if grep -Eq 'fake-token|fake-nonce' "$tmp/curl.log"; then
  printf 'reader staging credentials must not appear in curl arguments\n' >&2
  exit 1
fi

jq '.[0].config.services = [{internal_port:8108}]' "$tmp/state.json" > "$tmp/public.json"
if env "${common_env[@]}" FAKE_FLY_STATE="$tmp/public.json" \
  "$SCRIPT_DIR/stage-raw-reader-canary.sh" >/dev/null 2>&1; then
  printf 'Raw canary staging must reject public services\n' >&2
  exit 1
fi

jq '.[0].config.mounts[0].volume = "wrong-volume"' "$tmp/state.json" > "$tmp/wrong.json"
if env "${common_env[@]}" FAKE_FLY_STATE="$tmp/wrong.json" \
  "$SCRIPT_DIR/stage-raw-reader-canary.sh" >/dev/null 2>&1; then
  printf 'Raw canary staging must reject the wrong volume\n' >&2
  exit 1
fi

printf 'raw_reader_canary_staging=ok\n'
