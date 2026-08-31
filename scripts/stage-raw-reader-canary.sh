#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/fly-machine-session.sh
source "$SCRIPT_DIR/lib/fly-machine-session.sh"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_release

: "${BASE_IMAGE:?set BASE_IMAGE to the immutable production Raw image}"

if [[ ! "$RAW_CANARY_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'RAW_CANARY_MACHINE_ID must be one exact Machine ID\n' >&2
  exit 1
fi
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
expected_image="registry.fly.io/${RAW_APP}:raw-overlay-${source_commit:0:12}-${base_digest:0:12}"
reader_image="${READER_IMAGE:-$expected_image}"
if [[ "$reader_image" != "$expected_image" ]]; then
  printf 'READER_IMAGE must equal the source/base-derived image %s\n' "$expected_image" >&2
  exit 1
fi

canary_is_stageable() {
  jq -e --arg volume "$RAW_CANARY_VOLUME_ID" '
    .state == "stopped" and
    (.host_status // "ok") == "ok" and
    ((.incomplete_config // false) | not) and
    (.instance_id | type) == "string" and
    (.instance_id | length) > 0 and
    (.config | type) == "object" and
    .config.metadata.lifecycle == "temporary-canary" and
    .config.restart.policy == "no" and
    .config.dns.skip_registration == true and
    ((.config.services // []) | length) == 0 and
    ((.config.checks // {}) | length) == 0 and
    ((.config.mounts // []) | length) == 1 and
    .config.mounts[0].volume == $volume and
    .config.mounts[0].path == $mount_path and
    .config.guest.cpu_kind == "shared" and
    .config.guest.cpus == 2 and
    .config.guest.memory_mb == 2048
  ' --arg mount_path "$RAW_CANARY_MOUNT_PATH" >/dev/null
}

canary_is_exact() {
  local candidate
  candidate="$(cat)"
  printf '%s' "$candidate" | canary_is_stageable && jq -e \
    --arg image "$reader_image" \
    --arg source "$source_commit" \
    --arg base "sha256:$base_digest" '
    (.config.image == $image or (.config.image | startswith($image + "@sha256:"))) and
    (.image_ref.digest | test("^sha256:[0-9a-f]{64}$")) and
    .config.env == {
      RAW_SEARCH_BASE_CHANNELS_GZIP:"/data/source/youtube_youtubers.jsonl.gz",
      RAW_SEARCH_BASE_CHUNKS_GZIP:"/data/source/youtube_transcript_chunks.jsonl.gz",
      RAW_SEARCH_BIND:"[::]:8108",
      RAW_SEARCH_INDEX_PATH:"/data/index",
      RAW_SEARCH_STATE_DIRECTORY:"/data/mutable",
      RUST_LOG:"youtube_raw_search=info"
    } and
    .config.init == {swap_size_mb:512} and
    .config.metadata == {
      lifecycle:"temporary-canary",
      source_commit:$source,
      base_image_digest:$base
    } and
    ((.config.services // []) | length) == 0 and
    ((.config.checks // {}) | length) == 0 and
    ((.config.files // []) | length) == 0
  ' <<<"$candidate" >/dev/null
}

machine="$(flyctl machine list --app "$RAW_APP" --json |
  jq -ce --arg id "$RAW_CANARY_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact Raw canary not found") end
  ')"
if ! printf '%s' "$machine" | canary_is_stageable; then
  printf 'refusing to stage Raw reader on a non-isolated or unexpected canary\n' >&2
  exit 1
fi

if [[ "${APPLY_RAW_READER_CANARY:-}" != stage-exact-isolated-canary ]]; then
  jq -cn --arg machine "$RAW_CANARY_MACHINE_ID" --arg volume "$RAW_CANARY_VOLUME_ID" \
    --arg image "$reader_image" \
    '{apply:false,machine:$machine,volume:$volume,image:$image}'
  exit 0
fi

temporary_directory="$(mktemp -d)"
chmod 700 "$temporary_directory"
session_directory="$temporary_directory/session"
machine_config="$temporary_directory/machine.json"
fresh_machine="$temporary_directory/fresh-machine.json"
resolved_machine="$temporary_directory/resolved-machine.json"
lease_acquired=0

cleanup_reader_stage() {
  local prior_status="$1"
  local cleanup_status="$prior_status"
  local release_allowed=1
  local preserve_session=0
  local recovery_root recovery_destination
  trap - EXIT
  trap '' INT TERM
  if (( lease_acquired == 1 )) && ! fly_machine_session_stop "$session_directory" 3 10 1; then
    printf 'Raw canary stop could not be confirmed; retaining lease until TTL expiry\n' >&2
    cleanup_status=1
    release_allowed=0
    preserve_session=1
  fi
  if (( lease_acquired == 1 && release_allowed == 1 )) \
    && ! fly_machine_session_release "$session_directory"; then
    printf 'Raw canary lease release failed after staging\n' >&2
    cleanup_status=1
    preserve_session=1
  fi
  if (( preserve_session == 1 )); then
    recovery_root="${RAW_RELEASE_RECOVERY_ROOT:-$(fly_machine_session_default_recovery_root)}"
    recovery_destination="$recovery_root/reader-stage-${RAW_CANARY_MACHINE_ID}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
    if fly_machine_session_preserve "$session_directory" "$recovery_destination" >/dev/null; then
      printf 'Raw canary recovery session preserved at %s\n' "$recovery_destination" >&2
    else
      printf 'Raw canary recovery session remains at %s\n' "$session_directory" >&2
      exit "$cleanup_status"
    fi
  fi
  rm -rf "$temporary_directory"
  exit "$cleanup_status"
}
trap 'cleanup_reader_stage $?' EXIT
trap 'exit 130' INT TERM

jq -n --arg image "$reader_image" --arg source "$source_commit" \
  --arg base_digest "sha256:$base_digest" --arg volume "$RAW_CANARY_VOLUME_ID" '{
  image:$image,
  env:{
    RAW_SEARCH_BASE_CHANNELS_GZIP:"/data/source/youtube_youtubers.jsonl.gz",
    RAW_SEARCH_BASE_CHUNKS_GZIP:"/data/source/youtube_transcript_chunks.jsonl.gz",
    RAW_SEARCH_BIND:"[::]:8108",
    RAW_SEARCH_INDEX_PATH:"/data/index",
    RAW_SEARCH_STATE_DIRECTORY:"/data/mutable",
    RUST_LOG:"youtube_raw_search=info"
  },
  init:{swap_size_mb:512},
  guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
  metadata:{
    lifecycle:"temporary-canary",
    source_commit:$source,
    base_image_digest:$base_digest
  },
  mounts:[{volume:$volume,path:$mount_path}],
  restart:{policy:"no"},
  dns:{skip_registration:true},
  services:[],
  checks:{},
  files:[]
}' --arg mount_path "$RAW_CANARY_MOUNT_PATH" > "$machine_config"
chmod 600 "$machine_config"

fly_machine_session_create "$session_directory" "$RAW_APP" "$RAW_CANARY_MACHINE_ID"
fly_machine_session_acquire "$session_directory" 'ytsearch exact Raw reader staging' 300
lease_acquired=1
fly_machine_session_get "$session_directory" "$fresh_machine"
if ! canary_is_stageable < "$fresh_machine"; then
  printf 'Raw canary changed before its exclusive staging lease was acquired\n' >&2
  exit 1
fi
min_secrets_version="$(fly_machine_app_secrets_version "$RAW_APP")"
fly_machine_session_update "$session_directory" "$machine_config" "$min_secrets_version"
fly_machine_session_stop "$session_directory" 3 10 1
fly_machine_session_get "$session_directory" "$resolved_machine"
if ! canary_is_exact < "$resolved_machine"; then
  printf 'Raw canary verification failed after staging\n' >&2
  exit 1
fi
fly_machine_session_release "$session_directory"
lease_acquired=0

jq -c --arg source_commit "$source_commit" --arg base_image "$BASE_IMAGE" \
  --arg image "$reader_image" '{
  schema_version:1,
  kind:"raw-reader-canary",
  source_commit:$source_commit,
  base_image:$base_image,
  machine_id:.id,
  volume_id:.config.mounts[0].volume,
  private_ip:.private_ip,
  resolved_image:($image + "@" + .image_ref.digest),
  state:.state,
  isolated:true,
  memory_mb:.config.guest.memory_mb,
  cpus:.config.guest.cpus
}' "$resolved_machine"
