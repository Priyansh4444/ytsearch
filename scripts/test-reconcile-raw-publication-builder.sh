#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/lib/test-fly-fakes.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
fake_fly_setup "$tmp" ytsearch-indexer-raw-production 104052995

builder_id=33333333333333
target_id=22222222222222
image="registry.fly.io/ytsearch-indexer-raw-production:raw-indexer-${source_commit:0:12}-${base_digest:0:12}"
export FAKE_MACHINE_ID="$builder_id" \
  FAKE_TARGET_STATE="$tmp/target-state.json" \
  INDEXER_IMAGE="$image" \
  RAW_PUBLICATION_BUILDER_PROFILE=performance-1x-4gb \
  FAKE_UPDATE_FILTER='
    .current_version == "builder-version-1" and
    .skip_launch == true and
    .min_secrets_version == 104052995 and
    (.config.env | keys | sort) == [
      "RAW_PUBLICATION_MIN_FREE_BYTES",
      "RAW_PUBLICATION_WORK_DIRECTORY",
      "RAW_SEARCH_API_URL",
      "YTSEARCH_MACHINE_ROLE"
    ] and
    .config.init.entrypoint == ["/usr/bin/sleep"] and
    .config.init.cmd == ["infinity"] and
    (.config.services | length) == 0 and
    (.config.checks | length) == 0 and
    (.config.files | length) == 0 and
    .config.guest == {cpu_kind:"performance",cpus:1,memory_mb:4096} and
    .config.metadata.builder_profile == "performance-1x-4gb"
  '

cat > "$tmp/state.json" <<JSON
[{
  "id":"$builder_id",
  "instance_id":"builder-version-1",
  "state":"stopped",
  "host_status":"ok",
  "incomplete_config":false,
  "image_ref":{"digest":"sha256:$resolved_digest"},
  "config":{
    "env":{"YTSEARCH_MACHINE_ROLE":"publication-builder","STALE_QUEUE_MODE":"1"},
    "init":{"entrypoint":["/bin/sh"],"cmd":["/usr/local/bin/cloud-index-all"]},
    "guest":{"cpu_kind":"shared","cpus":4,"memory_mb":4096},
    "metadata":{"lifecycle":"on-demand-publication-builder"},
    "mounts":[{"volume":"vol_2222222222222222","path":"/data"}],
    "restart":{"policy":"no"},
    "dns":{"skip_registration":true},
    "services":[],
    "checks":{},
    "files":[{"guest_path":"/old-worker","raw_value":"old"}],
    "image":"registry.invalid/old:one"
  }
}]
JSON

jq -n --arg source "$source_commit" --arg id "$target_id" '[{
  id:$id,
  state:"stopped",
  private_ip:"fdaa::2",
  config:{metadata:{source_commit:$source}}
}]' > "$tmp/target-state.json"

"$SCRIPT_DIR/reconcile-raw-publication-builder.sh" > "$tmp/dry-run.json"
jq -e --arg machine "$builder_id" '
  .apply == false and .machine == $machine and
  .builder_profile == "performance-1x-4gb"
' "$tmp/dry-run.json" >/dev/null
[[ ! -s "$tmp/curl.log" ]]

APPLY_RAW_PUBLICATION_BUILDER=configure-exact-inert-builder \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh" > "$tmp/applied.json"
jq -e --arg image "$image@sha256:$resolved_digest" '
  .kind == "raw-publication-builder" and .resolved_image == $image and
  .state == "stopped" and .memory_mb == 4096 and .cpus == 1 and
  .builder_profile == "performance-1x-4gb"
' "$tmp/applied.json" >/dev/null
jq -e '
  .[0].state == "stopped" and
  (.[0].config.files | length) == 0 and
  (.[0].config.services | length) == 0 and
  (.[0].config.env | has("STALE_QUEUE_MODE") | not) and
  (.[0].config.env | keys | length) == 4 and
  .[0].config.guest == {cpu_kind:"performance",cpus:1,memory_mb:4096} and
  .[0].config.metadata.builder_profile == "performance-1x-4gb" and
  .[0].config.dns.skip_registration == true
' "$tmp/state.json" >/dev/null
if grep -Eq 'test-secret-token|test-lease-nonce' "$tmp/curl.log"; then
  printf 'builder staging credentials must not appear in curl arguments\n' >&2
  exit 1
fi

jq '.[0].config.services = [{internal_port:8108}]' "$tmp/state.json" > "$tmp/unsafe.json"
expect_failure 'builder reconciliation must reject public services' \
  env FAKE_FLY_STATE="$tmp/unsafe.json" "$SCRIPT_DIR/reconcile-raw-publication-builder.sh"
expect_failure 'builder reconciliation must reject public target URLs' \
  env RAW_PUBLICATION_TARGET_URL=https://search.pronsh.dev \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh"
expect_failure 'builder reconciliation must reject unknown resource profiles' \
  env RAW_PUBLICATION_BUILDER_PROFILE=performance-2x \
  "$SCRIPT_DIR/reconcile-raw-publication-builder.sh"

source "$SCRIPT_DIR/lib/raw-publication-builder-profile.sh"
raw_publication_builder_profile_load
raw_publication_builder_profile_apply_production_budgets
[[ "$RAW_CANARY_LEASE_TTL_SECONDS" == 43200 ]]
[[ "$PUBLICATION_TIMEOUT_SECONDS" == 10800 ]]

RAW_PUBLICATION_BUILDER_PROFILE=shared-2x
raw_publication_builder_profile_load
raw_publication_builder_profile_apply_production_budgets
[[ "$RAW_CANARY_LEASE_TTL_SECONDS" == 21600 ]]
[[ "$PUBLICATION_TIMEOUT_SECONDS" == 2400 ]]

printf 'raw_publication_builder_reconciliation=ok\n'
