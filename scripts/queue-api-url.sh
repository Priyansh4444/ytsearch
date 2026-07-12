#!/usr/bin/env sh

# Normalize the retired standalone queue hostname so older local env files
# cannot direct an operator or a newly created worker to a deleted service.
normalize_queue_api_url() {
  case "${1:-}" in
    ""|https://ytsearch-index-queue-staging.fly.dev|https://ytsearch-index-queue-staging.fly.dev/)
      printf '%s\n' 'https://ytsearch-api-typesense-fly-staging.fly.dev'
      ;;
    *)
      printf '%s\n' "${1%/}"
      ;;
  esac
}
