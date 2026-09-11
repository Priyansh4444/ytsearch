#!/usr/bin/env bash

raw_publication_builder_profile_load() {
  RAW_PUBLICATION_BUILDER_PROFILE="${RAW_PUBLICATION_BUILDER_PROFILE:-shared-2x}"
  case "$RAW_PUBLICATION_BUILDER_PROFILE" in
    shared-2x)
      RAW_PUBLICATION_BUILDER_CPU_KIND=shared
      RAW_PUBLICATION_BUILDER_CPUS=2
      ;;
    performance-1x)
      RAW_PUBLICATION_BUILDER_CPU_KIND=performance
      RAW_PUBLICATION_BUILDER_CPUS=1
      ;;
    *)
      printf 'RAW_PUBLICATION_BUILDER_PROFILE must be shared-2x or performance-1x\n' >&2
      return 1
      ;;
  esac
  RAW_PUBLICATION_BUILDER_MEMORY_MB=2048
}

raw_publication_builder_profile_apply_production_budgets() {
  case "$RAW_PUBLICATION_BUILDER_PROFILE" in
    shared-2x)
      RAW_CANARY_LEASE_TTL_SECONDS=21600
      PUBLICATION_TIMEOUT_SECONDS=2400
      ;;
    performance-1x)
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
