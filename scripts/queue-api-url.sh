#!/usr/bin/env sh

# Default to the production queue API and drop a trailing slash.
normalize_queue_api_url() {
  case "${1:-}" in
    "")
      printf '%s\n' 'https://ytsearch-api-raw-production.fly.dev'
      ;;
    *)
      printf '%s\n' "${1%/}"
      ;;
  esac
}
