#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

config="$tmp/wrangler.jsonc"
fly_api_config="$tmp/fly.raw-api.toml"
cp "$SCRIPT_DIR/../wrangler.jsonc" "$config"
cp "$SCRIPT_DIR/../fly.raw-api.toml" "$fly_api_config"
common_env=(
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  WRANGLER_CONFIG_PATH="$config"
  FLY_RAW_API_CONFIG_PATH="$fly_api_config"
)

env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write \
  > "$tmp/write.json"
jq -e '
  .mode == "write" and .synchronized == true and
  .raw_api.app == "ytsearch-api-raw-production" and
  .raw_api.machine_id == "13131313131313" and
  .raw_reader.app == "ytsearch-raw-production" and
  .raw_reader.machine_id == "10101010101010" and
  .raw_queue_worker.app == "ytsearch-indexer-raw-production"
' "$tmp/write.json" >/dev/null
env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --check \
  > "$tmp/check.json"
jq -e '.mode == "check" and .synchronized == true' "$tmp/check.json" >/dev/null
grep -F '"FLY_RAW_SEARCH_MACHINE_ID": "10101010101010"' "$config" >/dev/null
[[ "$(grep -c '"API_BASE_URL": "https://ytsearch-api-raw-production.fly.dev"' "$config")" == 1 ]]
[[ "$(grep -c '"INDEX_QUEUE_API_BASE_URL": "https://ytsearch-api-raw-production.fly.dev"' "$config")" == 1 ]]

sed -i \
  -e 's|"API_BASE_URL": "https://ytsearch-api-raw-production.fly.dev"|"API_BASE_URL": "https://drifted-api.fly.dev"|' \
  -e 's|"INDEX_QUEUE_API_BASE_URL": "https://ytsearch-api-raw-production.fly.dev"|"INDEX_QUEUE_API_BASE_URL": "https://drifted-api.fly.dev"|' \
  "$config"
env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write >/dev/null
[[ "$(grep -c 'https://ytsearch-api-raw-production.fly.dev' "$config")" == 2 ]]

sed -i "s/app = 'ytsearch-api-raw-production'/app = 'drifted-api'/" "$fly_api_config"
checksum_before="$(sha256sum "$config")"
if env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write \
  >/dev/null 2>&1; then
  printf 'Raw API Fly app drift must fail before Wrangler mutation\n' >&2
  exit 1
fi
[[ "$(sha256sum "$config")" == "$checksum_before" ]]
sed -i "s/app = 'drifted-api'/app = 'ytsearch-api-raw-production'/" "$fly_api_config"

sed -i 's/"FLY_RAW_SEARCH_MACHINE_ID": "10101010101010"/"FLY_RAW_SEARCH_MACHINE_ID": "30303030303030"/' \
  "$config"
if env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --check \
  >/dev/null 2>&1; then
  printf 'Wrangler identity drift must fail the check\n' >&2
  exit 1
fi
env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write >/dev/null

sed -i \
  -e 's/"FLY_APP_NAME": "ytsearch-indexer-raw-production"/"FLY_APP_NAME": "drifted-indexer"/' \
  -e 's/"FLY_RAW_SEARCH_APP_NAME": "ytsearch-raw-production"/"FLY_RAW_SEARCH_APP_NAME": "drifted-reader"/' \
  "$config"
sed -n '/"RAW_SEARCH_HEALTH_URL"/p' "$config" >> "$config"
checksum_before="$(sha256sum "$config")"
if env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write \
  >/dev/null 2>&1; then
  printf 'duplicate Wrangler identity keys must fail closed\n' >&2
  exit 1
fi
[[ "$(sha256sum "$config")" == "$checksum_before" ]]

sed -i '$d' "$config"
sed -i '/"RAW_SEARCH_HEALTH_URL"/d' "$config"
checksum_before="$(sha256sum "$config")"
if env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write \
  >/dev/null 2>&1; then
  printf 'missing Wrangler identity keys must fail closed\n' >&2
  exit 1
fi
[[ "$(sha256sum "$config")" == "$checksum_before" ]]

printf 'wrangler_production_resources=ok\n'
