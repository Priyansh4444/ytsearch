#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/test-fly-fakes.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
fake_fly_setup "$tmp" ytsearch-raw-production 104052994

machine_id=22222222222222
image="registry.fly.io/ytsearch-raw-production:raw-overlay-${source_commit:0:12}-${base_digest:0:12}"
export FAKE_MACHINE_ID="$machine_id" \
  READER_IMAGE="$image" \
  FAKE_UPDATE_FILTER='
    .current_version == "reader-version-1" and
    .skip_launch == true and
    .min_secrets_version == 104052994 and
    (.config.env | keys | sort) == [
      "RAW_SEARCH_BASE_CHANNELS_GZIP",
      "RAW_SEARCH_BASE_CHUNKS_GZIP",
      "RAW_SEARCH_BIND",
      "RAW_SEARCH_INDEX_PATH",
      "RAW_SEARCH_REQUIRE_FILE_SECRETS",
      "RAW_SEARCH_STATE_DIRECTORY",
      "RUST_LOG"
    ] and
    .config.init == {swap_size_mb:512} and
    (.config.services | length) == 0 and
    (.config.checks | length) == 0 and
    .config.files == [
      {guest_path:"/run/secrets/raw-search-admin-api-key",secret_name:"RAW_SEARCH_ADMIN_API_KEY"},
      {guest_path:"/run/secrets/raw-search-api-key",secret_name:"RAW_SEARCH_API_KEY"}
    ]
  '

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

"$SCRIPT_DIR/stage-raw-reader-canary.sh" > "$tmp/dry.json"
jq -e --arg machine "$machine_id" '.apply == false and .machine == $machine' \
  "$tmp/dry.json" >/dev/null
[[ ! -s "$tmp/curl.log" ]]

APPLY_RAW_READER_CANARY=stage-exact-isolated-canary \
  "$SCRIPT_DIR/stage-raw-reader-canary.sh" > "$tmp/applied.json"
jq -e --arg image "$image@sha256:$resolved_digest" \
  --arg state_directory "/data/mutable/trials/${source_commit:0:12}" '
  .kind == "raw-reader-canary" and .resolved_image == $image and
  .state == "stopped" and .isolated == true and .memory_mb == 2560 and
  .state_directory == $state_directory
' "$tmp/applied.json" >/dev/null
jq -e --arg state_directory "/data/mutable/trials/${source_commit:0:12}" '
  .[0].state == "stopped" and
  .[0].config.env.RAW_SEARCH_STATE_DIRECTORY == $state_directory and
  (.[0].config.env | has("STALE_QUEUE_MODE") | not) and
  (.[0].config.init | has("entrypoint") | not) and
  (.[0].config.init | has("cmd") | not) and
  .[0].config.files == [
    {guest_path:"/run/secrets/raw-search-admin-api-key",secret_name:"RAW_SEARCH_ADMIN_API_KEY"},
    {guest_path:"/run/secrets/raw-search-api-key",secret_name:"RAW_SEARCH_API_KEY"}
  ] and
  (.[0].config.services | length) == 0 and
  .[0].config.guest == {cpu_kind:"shared",cpus:4,memory_mb:2560}
' "$tmp/state.json" >/dev/null
grep -F 'https://api.test/v1/apps/ytsearch-raw-production/machines/22222222222222' \
  "$tmp/curl.log" >/dev/null
if grep -Eq 'test-secret-token|test-lease-nonce' "$tmp/curl.log"; then
  printf 'reader staging credentials must not appear in curl arguments\n' >&2
  exit 1
fi

jq '.[0].config.services = [{internal_port:8108}]' "$tmp/state.json" > "$tmp/public.json"
expect_failure 'Raw canary staging must reject public services' \
  env FAKE_FLY_STATE="$tmp/public.json" "$SCRIPT_DIR/stage-raw-reader-canary.sh"

jq '.[0].config.mounts[0].volume = "wrong-volume"' "$tmp/state.json" > "$tmp/wrong.json"
expect_failure 'Raw canary staging must reject the wrong volume' \
  env FAKE_FLY_STATE="$tmp/wrong.json" "$SCRIPT_DIR/stage-raw-reader-canary.sh"

printf 'raw_reader_canary_staging=ok\n'
