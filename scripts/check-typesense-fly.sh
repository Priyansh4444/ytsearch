#!/usr/bin/env sh
set -eu

# Confirm the self-hosted Typesense Fly path is healthy and benchmarkable.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

TYPESENSE_APP_NAME="${TYPESENSE_APP_NAME:-ytsearch-typesense-staging}"
TYPESENSE_API_APP_NAME="${TYPESENSE_API_APP_NAME:-ytsearch-api-typesense-fly-staging}"
TYPESENSE_ENGINE_URL="${TYPESENSE_SELFHOST_API_URL:-https://ytsearch-typesense-staging.fly.dev}"
TYPESENSE_API_BASE_URL="${TYPESENSE_FLY_API_BASE_URL:-https://ytsearch-api-typesense-fly-staging.fly.dev}"
QUERY="${QUERY:-react}"
YOUTUBER="${YOUTUBER:-@t3dotgg}"
RUNS="${RUNS:-5}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

require_command curl
require_command flyctl
require_command jq
require_env TYPESENSE_SELFHOST_API_KEY

echo "machines"
flyctl machine list --app "$TYPESENSE_APP_NAME"
flyctl machine list --app "$TYPESENSE_API_APP_NAME"

echo
echo "engine_health"
curl -fsS \
  -H "X-TYPESENSE-API-KEY: $TYPESENSE_SELFHOST_API_KEY" \
  "$TYPESENSE_ENGINE_URL/health" |
  jq .

echo
echo "engine_debug"
curl -fsS \
  -H "X-TYPESENSE-API-KEY: $TYPESENSE_SELFHOST_API_KEY" \
  "$TYPESENSE_ENGINE_URL/debug" |
  jq '{version, state}'

echo
echo "engine_metrics"
curl -fsS \
  -H "X-TYPESENSE-API-KEY: $TYPESENSE_SELFHOST_API_KEY" \
  "$TYPESENSE_ENGINE_URL/metrics.json" |
  jq '{
    cpu_percent: .system_cpu_active_percentage,
    memory_used_mb: (((.system_memory_used_bytes // 0) | tonumber) / 1048576 | floor),
    memory_total_mb: (((.system_memory_total_bytes // 0) | tonumber) / 1048576 | floor),
    typesense_resident_mb: (((.typesense_memory_resident_bytes // 0) | tonumber) / 1048576 | floor),
    disk_used_gb: (((.system_disk_used_bytes // 0) | tonumber) / 1073741824),
    disk_total_gb: (((.system_disk_total_bytes // 0) | tonumber) / 1073741824)
  }'

echo
echo "api_health"
curl -fsS "$TYPESENSE_API_BASE_URL/api/health" | jq .

echo
echo "indexed_channels"
curl -fsS "$TYPESENSE_API_BASE_URL/api/youtubers" |
  jq -r '.channels[] | [.handle, .indexed_video_count, .indexed_chunk_count] | @tsv'

echo
echo "search_benchmark"
BACKENDS="typesense-fly" \
QUERY="$QUERY" \
YOUTUBER="$YOUTUBER" \
RUNS="$RUNS" \
scripts/benchmark-search-backends.sh
