#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-machine-session.sh
source "$SCRIPT_DIR/lib/fly-machine-session.sh"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
# shellcheck source=scripts/lib/fly-publication-job.sh
source "$SCRIPT_DIR/lib/fly-publication-job.sh"
fly_production_inventory_load_raw_release

# Run the external Raw generation builder on the existing on-demand indexer.
# The narrow argument contract lets this script plug directly into
# benchmark-publication-isolation.sh without forwarding shell text remotely.

: "${RAW_PUBLICATION_TARGET_URL:?set RAW_PUBLICATION_TARGET_URL to the exact Raw reader URL}"
: "${RAW_PUBLICATION_SOURCE_COMMIT:?set RAW_PUBLICATION_SOURCE_COMMIT to the full release commit}"
: "${RAW_PUBLICATION_BASE_IMAGE_DIGEST:?set RAW_PUBLICATION_BASE_IMAGE_DIGEST to the shared corpus digest}"
: "${RAW_BUILDER_EXPECTED_IMAGE:?set RAW_BUILDER_EXPECTED_IMAGE to the reviewed resolved image}"
: "${RAW_BUILDER_RESOURCE_PROBE:?set RAW_BUILDER_RESOURCE_PROBE to the exact builder probe}"
: "${RAW_BUILDER_FINAL_RESOURCE_OUTPUT:?set RAW_BUILDER_FINAL_RESOURCE_OUTPUT}"
: "${RAW_BUILDER_STOP_RETRIES:=3}"
: "${RAW_BUILDER_STOP_POLL_ATTEMPTS:=10}"
: "${RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS:=2}"
: "${RAW_BUILDER_LEASE_TTL_SECONDS:=7200}"
: "${RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS:=4500}"
: "${RAW_BUILDER_COMMAND_TIMEOUT_SECONDS:=120}"
: "${RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS:=15}"
: "${RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS:=2400}"
: "${RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS:=10}"
: "${RAW_BUILDER_START_POLL_ATTEMPTS:=60}"
: "${RAW_BUILDER_START_POLL_INTERVAL_SECONDS:=1}"
: "${RAW_BUILDER_SSH_READY_ATTEMPTS:=30}"
: "${RAW_BUILDER_SSH_READY_INTERVAL_SECONDS:=2}"
: "${FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS:=10}"

if [[ ! "$RAW_BUILDER_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'RAW_BUILDER_MACHINE_ID must be one exact Machine ID\n' >&2
  exit 1
fi
if [[ ! "$RAW_PUBLICATION_TARGET_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'RAW_PUBLICATION_TARGET_MACHINE_ID must be one exact Machine ID\n' >&2
  exit 1
fi
if [[ ! "$RAW_PUBLICATION_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  printf 'RAW_PUBLICATION_SOURCE_COMMIT must be a full Git commit OID\n' >&2
  exit 1
fi
if [[ ! "$RAW_PUBLICATION_BASE_IMAGE_DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  printf 'RAW_PUBLICATION_BASE_IMAGE_DIGEST must be one sha256 digest\n' >&2
  exit 1
fi
base_digest="${RAW_PUBLICATION_BASE_IMAGE_DIGEST#sha256:}"
expected_builder_image="registry.fly.io/${RAW_BUILDER_APP}:raw-indexer-${RAW_PUBLICATION_SOURCE_COMMIT:0:12}-${base_digest:0:12}"
if [[ ! "$RAW_BUILDER_EXPECTED_IMAGE" =~ ^registry\.fly\.io/[a-z0-9][a-z0-9-]{0,62}:[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}$ ]] \
  || [[ "$RAW_BUILDER_EXPECTED_IMAGE" != "$expected_builder_image"@sha256:* ]]; then
  printf 'RAW_BUILDER_EXPECTED_IMAGE must be the exact resolved release image\n' >&2
  exit 1
fi
if [[ ! -x "$RAW_BUILDER_RESOURCE_PROBE" ]]; then
  printf 'RAW_BUILDER_RESOURCE_PROBE must be executable\n' >&2
  exit 1
fi
if [[ ! "$RAW_BUILDER_STOP_RETRIES" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_STOP_POLL_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$RAW_BUILDER_LEASE_TTL_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_START_POLL_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_START_POLL_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || (( RAW_BUILDER_LEASE_TTL_SECONDS < RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS )); then
  printf 'builder stop and lease settings must be valid bounded integers\n' >&2
  exit 1
fi
builder_start_budget=$((
  (2 * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
  (RAW_BUILDER_START_POLL_ATTEMPTS *
    (FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS +
      RAW_BUILDER_START_POLL_INTERVAL_SECONDS))
))
builder_ssh_ready_budget=$((
  RAW_BUILDER_SSH_READY_ATTEMPTS *
  (RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS + RAW_BUILDER_SSH_READY_INTERVAL_SECONDS)
))
builder_cleanup_budget=$((
  RAW_BUILDER_STOP_RETRIES *
  (((RAW_BUILDER_STOP_POLL_ATTEMPTS + 2) * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
    (RAW_BUILDER_STOP_POLL_ATTEMPTS * RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS)) +
  FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS
))
derived_lease_budget=$((
  builder_start_budget + builder_ssh_ready_budget +
  (2 * RAW_BUILDER_COMMAND_TIMEOUT_SECONDS) +
  RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS + builder_cleanup_budget
))
if (( RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS < derived_lease_budget \
  || RAW_BUILDER_LEASE_TTL_SECONDS < derived_lease_budget )); then
  printf 'builder lease is below the derived bounded command budget: configured=%ss required=%ss\n' \
    "$RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS" "$derived_lease_budget" >&2
  exit 1
fi

if [[ "$#" -ne 3 || "$1" != publish || "$2" != --backend || "$3" != raw ]]; then
  printf 'usage: %s publish --backend raw\n' "$0" >&2
  exit 2
fi

for command in curl flyctl jq timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

bounded_flyctl() {
  local maximum_seconds="$1"
  shift
  timeout --signal=TERM --kill-after="$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" \
    "$maximum_seconds" flyctl "$@"
}

target_machine="$(bounded_flyctl "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
  machine list --app "$RAW_TARGET_APP" --json |
  jq -ce --arg id "$RAW_PUBLICATION_TARGET_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact publication target not found") end
  ')"
expected_target_url="$(jq -er '"http://[" + .private_ip + "]:8108"' <<<"$target_machine")"
if [[ "$RAW_PUBLICATION_TARGET_URL" != "$expected_target_url" ]] \
  || ! jq -e --arg source "$RAW_PUBLICATION_SOURCE_COMMIT" '
    .state == "started" and .config.metadata.source_commit == $source
  ' <<<"$target_machine" >/dev/null; then
  printf 'publication target URL, state, or source does not match the exact Raw Machine\n' >&2
  exit 1
fi

machine="$(bounded_flyctl "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
  machine list --app "$RAW_BUILDER_APP" --json |
  jq -ce --arg id "$RAW_BUILDER_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact publication builder not found") end
  ')"
machine_is_safe() {
  jq -e \
  --arg target "$RAW_PUBLICATION_TARGET_URL" \
  --arg target_machine "$RAW_PUBLICATION_TARGET_MACHINE_ID" \
  --arg source "$RAW_PUBLICATION_SOURCE_COMMIT" \
  --arg base "$RAW_PUBLICATION_BASE_IMAGE_DIGEST" \
  --arg image "$expected_builder_image" \
  --arg expected_resolved_image "$RAW_BUILDER_EXPECTED_IMAGE" \
  --arg volume "$RAW_BUILDER_VOLUME_ID" '
  .state == "stopped" and
  (.config.image == $image or (.config.image | startswith($image + "@sha256:"))) and
  ((if (.config.image | contains("@sha256:")) then .config.image
    else .config.image + "@" + .image_ref.digest end) == $expected_resolved_image) and
  .config.env.YTSEARCH_MACHINE_ROLE == "publication-builder" and
  .config.env.RAW_SEARCH_API_URL == $target and
  .config.env.RAW_PUBLICATION_WORK_DIRECTORY == "/var/lib/ytsearch/raw-generation-work" and
  .config.env.RAW_PUBLICATION_MIN_FREE_BYTES == "8000000000" and
  ((.config.env | keys | sort) == [
    "RAW_PUBLICATION_MIN_FREE_BYTES",
    "RAW_PUBLICATION_WORK_DIRECTORY",
    "RAW_SEARCH_API_URL",
    "YTSEARCH_MACHINE_ROLE"
  ]) and
  .config.init.entrypoint == ["/usr/bin/sleep"] and
  .config.init.cmd == ["infinity"] and
  .config.init.swap_size_mb == 512 and
  .config.metadata.lifecycle == "on-demand-publication-builder" and
  .config.metadata.source_commit == $source and
  .config.metadata.base_image_digest == $base and
  .config.metadata.target_machine == $target_machine and
  .config.restart.policy == "no" and
  .config.dns.skip_registration == true and
  .config.guest.cpu_kind == "shared" and
  .config.guest.cpus == 2 and
  .config.guest.memory_mb == 2048 and
  ((.config.services // []) | length) == 0 and
  ((.config.checks // {}) | length) == 0 and
  ((.config.files // []) | length) == 0 and
  ((.config.mounts // []) | length) == 1 and
  .config.mounts[0].volume == $volume and
  .config.mounts[0].path == "/var/lib/ytsearch"
' >/dev/null
}
if ! printf '%s' "$machine" | machine_is_safe; then
  printf 'refusing to start %s: expected one stopped, inert publication builder targeting %s\n' \
    "$RAW_BUILDER_MACHINE_ID" "$RAW_PUBLICATION_TARGET_URL" >&2
  exit 1
fi

temporary_directory="$(mktemp -d)"
chmod 700 "$temporary_directory"
session_directory="$temporary_directory/session"
fresh_machine="$temporary_directory/fresh-machine.json"
lease_response="$temporary_directory/lease-response.json"
lease_acquired=0

cleanup_builder() {
  local prior_status="$1"
  local cleanup_status="$prior_status"
  local release_allowed=1
  local preserve_session=0
  local recovery_root recovery_destination
  trap - EXIT
  trap '' INT TERM
  if (( lease_acquired == 1 )) && ! fly_machine_session_stop "$session_directory" \
    "$RAW_BUILDER_STOP_RETRIES" "$RAW_BUILDER_STOP_POLL_ATTEMPTS" \
    "$RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS"; then
    printf 'publication builder stop could not be confirmed; retaining lease until TTL expiry\n' >&2
    cleanup_status=1
    release_allowed=0
    preserve_session=1
  fi
  if (( lease_acquired == 1 && release_allowed == 1 )); then
    if ! fly_machine_session_release "$session_directory"; then
      printf 'publication builder lease release failed\n' >&2
      cleanup_status=1
      preserve_session=1
    fi
  fi
  if (( preserve_session == 1 )); then
    recovery_root="${RAW_RELEASE_RECOVERY_ROOT:-$(fly_machine_session_default_recovery_root)}"
    recovery_destination="$recovery_root/builder-publish-${RAW_BUILDER_MACHINE_ID}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
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
trap 'cleanup_builder $?' EXIT
trap 'exit 130' INT TERM

fly_machine_session_create "$session_directory" "$RAW_BUILDER_APP" "$RAW_BUILDER_MACHINE_ID"
fly_machine_session_acquire "$session_directory" 'ytsearch detached Raw publication' \
  "$RAW_BUILDER_LEASE_TTL_SECONDS"
lease_acquired=1
fly_machine_session_lease_response "$session_directory" "$lease_response"
lease_expires_at="$(jq -er '.data.expires_at | select(type == "number" and floor == .)' \
  "$lease_response")"
if (( lease_expires_at - $(date +%s) < RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS )); then
  printf 'publication builder lease is too short for the bounded publication phase\n' >&2
  exit 1
fi
fly_machine_session_get "$session_directory" "$fresh_machine"
if ! machine_is_safe < "$fresh_machine"; then
  printf 'publication builder changed before the exclusive lease was acquired\n' >&2
  exit 1
fi

FLY_MACHINE_SESSION_START_POLL_ATTEMPTS="$RAW_BUILDER_START_POLL_ATTEMPTS" \
FLY_MACHINE_SESSION_START_POLL_INTERVAL_SECONDS="$RAW_BUILDER_START_POLL_INTERVAL_SECONDS" \
  fly_machine_session_start "$session_directory"

ssh_ready=0
for ((attempt = 1; attempt <= RAW_BUILDER_SSH_READY_ATTEMPTS; attempt += 1)); do
  if bounded_flyctl "$RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS" \
    ssh console --app "$RAW_BUILDER_APP" \
    --machine "$RAW_BUILDER_MACHINE_ID" --command true >/dev/null 2>&1; then
    ssh_ready=1
    break
  fi
  sleep "$RAW_BUILDER_SSH_READY_INTERVAL_SECONDS"
done
if (( ssh_ready == 0 )); then
  printf 'external Raw builder did not become reachable\n' >&2
  exit 1
fi

provenance="$(bounded_flyctl "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
  ssh console --app "$RAW_BUILDER_APP" --machine "$RAW_BUILDER_MACHINE_ID" \
  --command \
  "sh -c 'cat /usr/local/share/ytsearch-release/source-commit; cat /usr/local/share/ytsearch-release/base-image'")"
provenance_source="$(sed -n '1p' <<<"$provenance")"
provenance_base="$(sed -n '2p' <<<"$provenance")"
if [[ "$provenance_source" != "$RAW_PUBLICATION_SOURCE_COMMIT" ]] \
  || [[ "$provenance_base" != *@"$RAW_PUBLICATION_BASE_IMAGE_DIGEST" ]]; then
  printf 'publication builder image provenance does not match the source/base release\n' >&2
  exit 1
fi

bounded_flyctl "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
  ssh console --app "$RAW_BUILDER_APP" \
  --machine "$RAW_BUILDER_MACHINE_ID" \
  --command 'ytsearch init-backend --backend raw'
publication_job="/tmp/ytsearch-publication-${temporary_directory##*.}"
fly_publication_job_wait "$publication_job" "$temporary_directory/publication-ssh.log"

mkdir -p "$(dirname -- "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT")"
final_resource_snapshot="$("$RAW_BUILDER_RESOURCE_PROBE")"
if ! jq -e '
  .state == "started" and
  (.memory_peak_bytes | type) == "number" and
  (.swap_peak_bytes | type) == "number" and
  (.disk_percent | type) == "number" and
  (.oom | type) == "number" and
  (.oom_kill | type) == "number" and
  (.cgroup_version == 1 or .cgroup_version == 2) and
  (.memory_peak_source == "kernel" or .memory_peak_source == "sampled_current") and
  (.swap_peak_source == "kernel" or .swap_peak_source == "sampled_current")
' <<<"$final_resource_snapshot" >/dev/null; then
  printf 'publication builder final resource snapshot is invalid\n' >&2
  exit 1
fi
printf '%s\n' "$final_resource_snapshot" > "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT"
chmod 600 "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT"
