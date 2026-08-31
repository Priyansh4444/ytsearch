#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

config="$tmp/wrangler.jsonc"
cp "$SCRIPT_DIR/../wrangler.jsonc" "$config"
common_env=(
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  WRANGLER_CONFIG_PATH="$config"
)

env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --write \
  > "$tmp/write.json"
jq -e '
  .mode == "write" and .synchronized == true and
  .raw_reader.app == "ytsearch-raw-production" and
  .raw_reader.machine_id == "10101010101010" and
  .raw_queue_worker.app == "ytsearch-indexer-raw-production"
' "$tmp/write.json" >/dev/null
env "${common_env[@]}" "$SCRIPT_DIR/sync-wrangler-production-resources.sh" --check \
  > "$tmp/check.json"
jq -e '.mode == "check" and .synchronized == true' "$tmp/check.json" >/dev/null
grep -F '"FLY_RAW_SEARCH_MACHINE_ID": "10101010101010"' "$config" >/dev/null

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
