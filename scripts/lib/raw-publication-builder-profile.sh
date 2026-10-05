#!/usr/bin/env bash

raw_publication_builder_profile_load() {
  RAW_PUBLICATION_BUILDER_PROFILE="${RAW_PUBLICATION_BUILDER_PROFILE:-shared-2x}"
  case "$RAW_PUBLICATION_BUILDER_PROFILE" in
    shared-2x)
      RAW_PUBLICATION_BUILDER_CPU_KIND=shared
      RAW_PUBLICATION_BUILDER_CPUS=2
      ;;
    performance-1x-4gb)
      RAW_PUBLICATION_BUILDER_CPU_KIND=performance
      RAW_PUBLICATION_BUILDER_CPUS=1
      ;;
    *)
      printf 'RAW_PUBLICATION_BUILDER_PROFILE must be shared-2x or performance-1x-4gb\n' >&2
      return 1
      ;;
  esac
  case "$RAW_PUBLICATION_BUILDER_PROFILE" in
    shared-2x) RAW_PUBLICATION_BUILDER_MEMORY_MB=2048 ;;
    performance-1x-4gb) RAW_PUBLICATION_BUILDER_MEMORY_MB=4096 ;;
  esac
}

raw_publication_builder_profile_apply_production_budgets() {
  case "$RAW_PUBLICATION_BUILDER_PROFILE" in
    shared-2x)
      RAW_CANARY_LEASE_TTL_SECONDS=21600
      PUBLICATION_TIMEOUT_SECONDS=2400
      ;;
    performance-1x-4gb)
      # The corpus scan alone takes about eight minutes on shared storage. A
      # schema rebuild needs enough time for sorting, encoding, verification,
      # and a second publication process within the reader's lease.
      RAW_CANARY_LEASE_TTL_SECONDS=43200
      PUBLICATION_TIMEOUT_SECONDS=10800
      ;;
    *)
      printf 'load the Raw publication builder profile before applying budgets\n' >&2
      return 1
      ;;
  esac
}

# Succeeds when the Machine JSON on stdin is the stopped, inert publication
# builder staged for one release and loaded profile. Callers add their own
# checks as a jq filter, followed by any jq arguments that filter needs:
#   raw_publication_builder_is_staged <source-commit> <target-url> \
#     <target-machine> <base-image-digest> <mount-path> <filter> [jq-args...]
raw_publication_builder_is_staged() {
  local source="$1" target="$2" target_machine="$3" base_digest="$4" mount_path="$5"
  local filter="$6"
  shift 6
  jq -e "$@" \
    --arg source "$source" \
    --arg target "$target" \
    --arg target_machine "$target_machine" \
    --arg base_digest "$base_digest" \
    --arg mount_path "$mount_path" \
    --arg volume "$RAW_BUILDER_VOLUME_ID" \
    --arg builder_profile "$RAW_PUBLICATION_BUILDER_PROFILE" \
    --arg cpu_kind "$RAW_PUBLICATION_BUILDER_CPU_KIND" \
    --argjson cpus "$RAW_PUBLICATION_BUILDER_CPUS" \
    --argjson memory_mb "$RAW_PUBLICATION_BUILDER_MEMORY_MB" '
    .state == "stopped" and
    (.image_ref.digest | test("^sha256:[0-9a-f]{64}$")) and
    .config.env.YTSEARCH_MACHINE_ROLE == "publication-builder" and
    .config.env.RAW_SEARCH_API_URL == $target and
    .config.init.entrypoint == ["/usr/bin/sleep"] and
    .config.init.cmd == ["infinity"] and
    .config.metadata.lifecycle == "on-demand-publication-builder" and
    .config.metadata.source_commit == $source and
    .config.metadata.base_image_digest == $base_digest and
    .config.metadata.builder_profile == $builder_profile and
    .config.metadata.target_machine == $target_machine and
    .config.restart.policy == "no" and
    .config.guest.cpu_kind == $cpu_kind and
    .config.guest.cpus == $cpus and
    .config.guest.memory_mb == $memory_mb and
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
    .config.mounts[0].path == $mount_path and
    ('"$filter"')
  ' >/dev/null
}
