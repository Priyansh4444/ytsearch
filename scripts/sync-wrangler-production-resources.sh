#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_runtime

mode="${1:---check}"
if [[ "$#" -gt 1 || ( "$mode" != --check && "$mode" != --write ) ]]; then
  printf 'usage: %s [--check|--write]\n' "$0" >&2
  exit 2
fi

wrangler_config="${WRANGLER_CONFIG_PATH:-$REPOSITORY_ROOT/wrangler.jsonc}"
fly_raw_api_config="${FLY_RAW_API_CONFIG_PATH:-$REPOSITORY_ROOT/fly.raw-api.toml}"
if [[ ! -f "$wrangler_config" || -L "$wrangler_config" ]]; then
  printf 'Wrangler config must be one regular, non-symlink file: %s\n' \
    "$wrangler_config" >&2
  exit 1
fi
if [[ ! -f "$fly_raw_api_config" || -L "$fly_raw_api_config" ]]; then
  printf 'Raw API Fly config must be one regular, non-symlink file: %s\n' \
    "$fly_raw_api_config" >&2
  exit 1
fi

mapfile -t fly_api_apps < <(sed -nE \
  "s/^[[:space:]]*app[[:space:]]*=[[:space:]]*'([^']+)'[[:space:]]*$/\\1/p" \
  "$fly_raw_api_config")
if (( ${#fly_api_apps[@]} != 1 )) || [[ "${fly_api_apps[0]}" != "$RAW_API_APP" ]]; then
  printf 'Raw API Fly app drifted from %s: expected=%s actual=%s\n' \
    "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH" "$RAW_API_APP" \
    "${fly_api_apps[*]:-missing-or-duplicate}" >&2
  exit 1
fi

keys=(
  API_BASE_URL
  INDEX_QUEUE_API_BASE_URL
  FLY_APP_NAME
  FLY_RAW_SEARCH_APP_NAME
  FLY_RAW_SEARCH_MACHINE_ID
  RAW_SEARCH_HEALTH_URL
)
expected_values=(
  "https://${RAW_API_APP}.fly.dev"
  "https://${RAW_API_APP}.fly.dev"
  "$RAW_QUEUE_WORKER_APP"
  "$RAW_READER_APP"
  "$RAW_READER_MACHINE_ID"
  "https://${RAW_READER_APP}.fly.dev/health"
)

read_config_value() {
  local key="$1"
  local config_path="$2"
  local -a values
  mapfile -t values < <(sed -nE \
    "s/^[[:space:]]*\"${key}\":[[:space:]]*\"([^\"]*)\",?[[:space:]]*$/\\1/p" \
    "$config_path")
  if (( ${#values[@]} != 1 )); then
    printf 'Wrangler config must contain exactly one quoted %s value\n' "$key" >&2
    return 1
  fi
  printf '%s\n' "${values[0]}"
}

write_config_value() {
  local key="$1"
  local value="$2"
  local config_path="$3"
  sed -i -E \
    "s|^([[:space:]]*\"${key}\":[[:space:]]*)\"[^\"]*\"(,?[[:space:]]*)$|\\1\"${value}\"\\2|" \
    "$config_path"
}

current_values=()
for index in "${!keys[@]}"; do
  current_values+=("$(read_config_value "${keys[index]}" "$wrangler_config")")
done

if [[ "$mode" == --check ]]; then
  for index in "${!keys[@]}"; do
    current="${current_values[index]}"
    if [[ "$current" != "${expected_values[index]}" ]]; then
      printf 'Wrangler %s drifted from %s: expected=%s actual=%s\n' \
        "${keys[index]}" "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH" \
        "${expected_values[index]}" "$current" >&2
      exit 1
    fi
  done
else
  config_directory="$(cd -- "$(dirname -- "$wrangler_config")" && pwd)"
  candidate="$(mktemp "$config_directory/.wrangler-production-resources.XXXXXX")"
  cleanup_candidate() {
    rm -f "$candidate"
  }
  trap cleanup_candidate EXIT INT TERM
  cp --preserve=mode "$wrangler_config" "$candidate"
  for index in "${!keys[@]}"; do
    if [[ "${current_values[index]}" != "${expected_values[index]}" ]]; then
      write_config_value "${keys[index]}" "${expected_values[index]}" "$candidate"
    fi
  done
  for index in "${!keys[@]}"; do
    current="$(read_config_value "${keys[index]}" "$candidate")"
    if [[ "$current" != "${expected_values[index]}" ]]; then
      printf 'candidate Wrangler %s did not synchronize atomically\n' "${keys[index]}" >&2
      exit 1
    fi
  done
  mv -- "$candidate" "$wrangler_config"
  candidate=""
  trap - EXIT INT TERM
fi

for index in "${!keys[@]}"; do
  current="$(read_config_value "${keys[index]}" "$wrangler_config")"
  if [[ "$current" != "${expected_values[index]}" ]]; then
    printf 'Wrangler %s drifted from %s: expected=%s actual=%s\n' \
      "${keys[index]}" "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH" \
      "${expected_values[index]}" "$current" >&2
    exit 1
  fi
done

jq -cn --arg mode "${mode#--}" --arg config "$wrangler_config" \
  --arg api_app "$RAW_API_APP" --arg api_machine "$RAW_API_MACHINE_ID" \
  --arg reader_app "$RAW_READER_APP" --arg reader_machine "$RAW_READER_MACHINE_ID" \
  --arg queue_worker_app "$RAW_QUEUE_WORKER_APP" '{
    mode:$mode,
    config:$config,
    raw_api:{app:$api_app,machine_id:$api_machine},
    raw_reader:{app:$reader_app,machine_id:$reader_machine},
    raw_queue_worker:{app:$queue_worker_app},
    synchronized:true
  }'
