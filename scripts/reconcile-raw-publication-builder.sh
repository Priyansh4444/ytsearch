#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/fly-machine-session.sh
source "$SCRIPT_DIR/lib/fly-machine-session.sh"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_release
: "${BASE_IMAGE:?set BASE_IMAGE to the immutable Raw corpus image used by the overlay}"

for command in curl flyctl git jq; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
if [[ ! "$source_commit" =~ ^[0-9a-f]{40}$ ]]; then
  printf 'unexpected Git commit: %s\n' "$source_commit" >&2
  exit 1
fi
if [[ ! "$BASE_IMAGE" =~ @sha256:([0-9a-f]{64})$ ]]; then
  printf 'BASE_IMAGE must name an immutable sha256 digest\n' >&2
  exit 1
fi
base_digest="${BASH_REMATCH[1]}"
if [[ ! "$RAW_BUILDER_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'RAW_BUILDER_MACHINE_ID must be one exact Machine ID\n' >&2
  exit 1
fi
if [[ ! "$RAW_PUBLICATION_TARGET_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'RAW_PUBLICATION_TARGET_MACHINE_ID must be one exact Machine ID\n' >&2
  exit 1
fi
target_machine="$(flyctl machine list --app "$RAW_TARGET_APP" --json |
  jq -ce --arg id "$RAW_PUBLICATION_TARGET_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact Raw target Machine not found") end
  ')"
expected_target_url="$(jq -er '"http://[" + .private_ip + "]:8108"' <<<"$target_machine")"
if [[ -n "${RAW_PUBLICATION_TARGET_URL:-}" && "$RAW_PUBLICATION_TARGET_URL" != "$expected_target_url" ]]; then
  printf 'provided publication target URL does not match the exact Raw Machine\n' >&2
  exit 1
fi
RAW_PUBLICATION_TARGET_URL="$expected_target_url"
if ! jq -e --arg source "$source_commit" \
    '.config.metadata.source_commit == $source' <<<"$target_machine" >/dev/null; then
  printf 'publication target source does not match the release commit\n' >&2
  exit 1
fi
target_machine_id="$RAW_PUBLICATION_TARGET_MACHINE_ID"
expected_image="registry.fly.io/${RAW_BUILDER_APP}:raw-indexer-${source_commit:0:12}-${base_digest:0:12}"
indexer_image="${INDEXER_IMAGE:-$expected_image}"
if [[ "$indexer_image" != "$expected_image" ]]; then
  printf 'INDEXER_IMAGE must equal the source/base-derived image %s\n' "$expected_image" >&2
  exit 1
fi

machine="$(flyctl machine list --app "$RAW_BUILDER_APP" --json |
  jq -ce --arg id "$RAW_BUILDER_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact builder Machine not found") end
  ')"
builder_is_stageable() {
  jq -e --arg volume "$RAW_BUILDER_VOLUME_ID" '
  .state == "stopped" and
  (.host_status // "ok") == "ok" and
  ((.incomplete_config // false) | not) and
  (.instance_id | type) == "string" and
  (.instance_id | length) > 0 and
  .config.restart.policy == "no" and
  ((.config.services // []) | length) == 0 and
  ((.config.mounts // []) | length) == 1 and
  .config.mounts[0].volume == $volume and
  (.config.mounts[0].path == "/data" or .config.mounts[0].path == $mount_path) and
  ((.config.env.YTSEARCH_MACHINE_ROLE // "publication-builder") == "publication-builder")
' --arg mount_path "$RAW_BUILDER_MOUNT_PATH" >/dev/null
}
if ! printf '%s' "$machine" | builder_is_stageable; then
  printf 'refusing to reconcile unexpected or running publication builder %s\n' \
    "$RAW_BUILDER_MACHINE_ID" >&2
  exit 1
fi

if [[ "${APPLY_RAW_PUBLICATION_BUILDER:-}" != "configure-exact-inert-builder" ]]; then
  jq -cn \
    --arg machine "$RAW_BUILDER_MACHINE_ID" \
    --arg volume "$RAW_BUILDER_VOLUME_ID" \
    --arg image "$indexer_image" \
    --arg target "$RAW_PUBLICATION_TARGET_URL" \
    --arg target_machine "$target_machine_id" \
    '{apply:false,machine:$machine,volume:$volume,image:$image,target:$target,target_machine:$target_machine}'
  exit 0
fi

temporary_directory="$(mktemp -d)"
chmod 700 "$temporary_directory"
session_directory="$temporary_directory/session"
machine_config="$temporary_directory/machine.json"
fresh_machine="$temporary_directory/fresh-machine.json"
resolved="$temporary_directory/resolved-machine.json"
lease_acquired=0

cleanup_builder_stage() {
  local prior_status="$1"
  local cleanup_status="$prior_status"
  local release_allowed=1
  local preserve_session=0
  local recovery_root recovery_destination
  trap - EXIT
  trap '' INT TERM
  if (( lease_acquired == 1 )) && ! fly_machine_session_stop "$session_directory" 3 10 1; then
    printf 'publication builder stop could not be confirmed; retaining lease until TTL expiry\n' >&2
    cleanup_status=1
    release_allowed=0
    preserve_session=1
  fi
  if (( lease_acquired == 1 && release_allowed == 1 )) \
    && ! fly_machine_session_release "$session_directory"; then
    printf 'publication builder lease release failed after staging\n' >&2
    cleanup_status=1
    preserve_session=1
  fi
  if (( preserve_session == 1 )); then
    recovery_root="${RAW_RELEASE_RECOVERY_ROOT:-$(fly_machine_session_default_recovery_root)}"
    recovery_destination="$recovery_root/builder-stage-${RAW_BUILDER_MACHINE_ID}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
    if fly_machine_session_preserve "$session_directory" "$recovery_destination" >/dev/null; then
      printf 'publication builder recovery session preserved at %s\n' "$recovery_destination" >&2
    else
      printf 'publication builder recovery session remains at %s\n' "$session_directory" >&2
      exit "$cleanup_status"
    fi
  fi
  rm -rf "$temporary_directory"
  exit "$cleanup_status"
}
trap 'cleanup_builder_stage $?' EXIT
trap 'exit 130' INT TERM

jq -n \
  --arg image "$indexer_image" \
  --arg target "$RAW_PUBLICATION_TARGET_URL" \
  --arg source "$source_commit" \
  --arg target_machine "$target_machine_id" \
  --arg base_digest "sha256:$base_digest" \
  --arg volume "$RAW_BUILDER_VOLUME_ID" '{
    image:$image,
    env:{
      YTSEARCH_MACHINE_ROLE:"publication-builder",
      RAW_SEARCH_API_URL:$target,
      RAW_PUBLICATION_WORK_DIRECTORY:"/var/lib/ytsearch/raw-generation-work",
      RAW_PUBLICATION_MIN_FREE_BYTES:"8000000000"
    },
    init:{
      entrypoint:["/usr/bin/sleep"],
      cmd:["infinity"],
      swap_size_mb:512
    },
    guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
    metadata:{
      lifecycle:"on-demand-publication-builder",
      source_commit:$source,
      base_image_digest:$base_digest,
      target_machine:$target_machine
    },
  mounts:[{volume:$volume,path:$mount_path}],
    restart:{policy:"no"},
    dns:{skip_registration:true},
    services:[],
    checks:{},
    files:[]
}' --arg mount_path "$RAW_BUILDER_MOUNT_PATH" > "$machine_config"
chmod 600 "$machine_config"

fly_machine_session_create "$session_directory" "$RAW_BUILDER_APP" "$RAW_BUILDER_MACHINE_ID"
fly_machine_session_acquire "$session_directory" 'ytsearch exact Raw publication builder staging' 300
lease_acquired=1
fly_machine_session_get "$session_directory" "$fresh_machine"
if ! builder_is_stageable < "$fresh_machine"; then
  printf 'publication builder changed before its exclusive staging lease was acquired\n' >&2
  exit 1
fi
min_secrets_version="$(fly_machine_app_secrets_version "$RAW_BUILDER_APP")"
fly_machine_session_update "$session_directory" "$machine_config" "$min_secrets_version"
fly_machine_session_stop "$session_directory" 3 10 1
fly_machine_session_get "$session_directory" "$resolved"
if ! jq -e \
  --arg image "$indexer_image" \
  --arg source "$source_commit" \
  --arg target "$RAW_PUBLICATION_TARGET_URL" \
  --arg target_machine "$target_machine_id" \
  --arg base "sha256:$base_digest" \
  --arg volume "$RAW_BUILDER_VOLUME_ID" '
  .state == "stopped" and
  (.config.image == $image or (.config.image | startswith($image + "@sha256:"))) and
  (.image_ref.digest | test("^sha256:[0-9a-f]{64}$")) and
  .config.env.YTSEARCH_MACHINE_ROLE == "publication-builder" and
  .config.env.RAW_SEARCH_API_URL == $target and
  .config.env.RAW_PUBLICATION_WORK_DIRECTORY == "/var/lib/ytsearch/raw-generation-work" and
  .config.env.RAW_PUBLICATION_MIN_FREE_BYTES == "8000000000" and
  .config.init.entrypoint == ["/usr/bin/sleep"] and
  .config.init.cmd == ["infinity"] and
  .config.init.swap_size_mb == 512 and
  .config.metadata.lifecycle == "on-demand-publication-builder" and
  .config.metadata.source_commit == $source and
  .config.metadata.base_image_digest == $base and
  .config.metadata.target_machine == $target_machine and
  .config.restart.policy == "no" and
  .config.guest.cpu_kind == "shared" and
  .config.guest.cpus == 2 and
  .config.guest.memory_mb == 2048 and
  ((.config.services // []) | length) == 0 and
  ((.config.checks // {}) | length) == 0 and
  ((.config.files // []) | length) == 0 and
  .config.dns.skip_registration == true and
  ((.config.env | keys | sort) == [
    "RAW_PUBLICATION_MIN_FREE_BYTES",
    "RAW_PUBLICATION_WORK_DIRECTORY",
    "RAW_SEARCH_API_URL",
    "YTSEARCH_MACHINE_ROLE"
  ]) and
  ((.config.mounts // []) | length) == 1 and
  .config.mounts[0].volume == $volume and
  .config.mounts[0].path == "/var/lib/ytsearch"
' "$resolved" >/dev/null; then
  printf 'publication builder verification failed after update\n' >&2
  exit 1
fi
fly_machine_session_release "$session_directory"
lease_acquired=0

jq -c \
  --arg source_commit "$source_commit" \
  --arg base_image "$BASE_IMAGE" \
  --arg image "$indexer_image" \
  --arg target "$RAW_PUBLICATION_TARGET_URL" '
  {
    schema_version:1,
    kind:"raw-publication-builder",
    source_commit:$source_commit,
    base_image:$base_image,
    target:$target,
    machine_id:.id,
    volume_id:.config.mounts[0].volume,
    resolved_image:($image + "@" + .image_ref.digest),
    state:.state,
    memory_mb:.config.guest.memory_mb,
    cpus:.config.guest.cpus
  }
' "$resolved"
