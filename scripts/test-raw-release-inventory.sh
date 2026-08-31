#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

valid_inventory="$tmp/valid.json"
jq -n '{
  schema_version:1,
  environment:"production",
  resources:{
    raw_reader:{
      app:"reader-app",machine_id:"10101010101010",
      volume_id:"vol_1010101010101010",mount_path:"/data/mutable"
    },
    raw_reader_canary:{
      app:"reader-app",machine_id:"11111111111111",
      volume_id:"vol_1111111111111111",mount_path:"/data/mutable"
    },
    raw_queue_worker:{
      app:"builder-app",machine_id:"20202020202020",
      volume_id:"vol_2020202020202020",mount_path:"/var/lib/ytsearch"
    },
    raw_publication_builder:{
      app:"builder-app",machine_id:"22222222222222",
      volume_id:"vol_2222222222222222",mount_path:"/var/lib/ytsearch"
    }
  },
  retirement_sets:{typesense:{
    reason:"raw-search-cutover",
    machines:[
      {app:"old-engine",machine_id:"33333333333333",volume_id:"vol_3333333333333333",successor_app:"reader-app"},
      {app:"old-engine",machine_id:"44444444444444",volume_id:"vol_4444444444444444",successor_app:"reader-app"},
      {app:"old-api",machine_id:"55555555555555",volume_id:"vol_5555555555555555",successor_app:"new-api"},
      {app:"old-indexer",machine_id:"66666666666666",volume_id:"vol_6666666666666666",successor_app:"builder-app"}
    ],
    unattached_volumes:[
      {app:"old-indexer",volume_id:"vol_7777777777777777"},
      {app:"old-indexer",volume_id:"vol_8888888888888888"},
      {app:"old-indexer",volume_id:"vol_9999999999999999"}
    ]
  }}
}' > "$valid_inventory"

FLY_PRODUCTION_INVENTORY_PATH="$valid_inventory"
fly_production_inventory_load_raw_release
[[ "$RAW_APP" == reader-app ]]
[[ "$RAW_CANARY_MACHINE_ID" == 11111111111111 ]]
[[ "$RAW_BUILDER_APP" == builder-app ]]
[[ "$RAW_PUBLICATION_TARGET_MACHINE_ID" == 11111111111111 ]]

fly_production_inventory_load_raw_runtime
[[ "$RAW_READER_APP" == reader-app ]]
[[ "$RAW_READER_MACHINE_ID" == 10101010101010 ]]
[[ "$RAW_QUEUE_WORKER_APP" == builder-app ]]
[[ "$RAW_QUEUE_WORKER_MACHINE_ID" == 20202020202020 ]]

if (
  unset RAW_CANARY_MACHINE_ID
  RAW_APP=wrong-reader FLY_PRODUCTION_INVENTORY_PATH="$valid_inventory"
  fly_production_inventory_load_raw_release
) >/dev/null 2>&1; then
  printf 'inventory must reject a mixed identity tuple\n' >&2
  exit 1
fi

(
  unset RAW_APP RAW_CANARY_MACHINE_ID RAW_CANARY_VOLUME_ID RAW_CANARY_MOUNT_PATH
  unset RAW_BUILDER_APP RAW_BUILDER_MACHINE_ID RAW_BUILDER_VOLUME_ID RAW_BUILDER_MOUNT_PATH
  unset RAW_TARGET_APP RAW_PUBLICATION_TARGET_MACHINE_ID
  RAW_PUBLICATION_TARGET_MACHINE_ID=99999999999999
  if fly_production_inventory_load_raw_release >/dev/null 2>&1; then
    printf 'late identity conflicts must fail\n' >&2
    exit 1
  fi
  if [[ -v RAW_APP ]]; then
    printf 'failed inventory loads must not partially assign identity fields\n' >&2
    exit 1
  fi
)

(
  unset RAW_READER_APP RAW_READER_MACHINE_ID RAW_READER_VOLUME_ID RAW_READER_MOUNT_PATH
  unset RAW_QUEUE_WORKER_APP RAW_QUEUE_WORKER_MACHINE_ID RAW_QUEUE_WORKER_VOLUME_ID
  unset RAW_QUEUE_WORKER_MOUNT_PATH
  RAW_QUEUE_WORKER_MACHINE_ID=99999999999999
  if FLY_PRODUCTION_INVENTORY_PATH="$valid_inventory" \
    fly_production_inventory_load_raw_runtime >/dev/null 2>&1; then
    printf 'runtime identity conflicts must fail\n' >&2
    exit 1
  fi
  if [[ -v RAW_READER_APP ]]; then
    printf 'failed runtime inventory loads must not partially assign identity fields\n' >&2
    exit 1
  fi
)

jq '.resources.raw_reader_canary.secret = "must-not-be-here"' "$valid_inventory" > "$tmp/extra-key.json"
if FLY_PRODUCTION_INVENTORY_PATH="$tmp/extra-key.json" fly_production_inventory_validate >/dev/null 2>&1; then
  printf 'inventory must reject unknown fields\n' >&2
  exit 1
fi

jq '.resources.raw_reader_canary.machine_id = "guess"' "$valid_inventory" > "$tmp/invalid-id.json"
if FLY_PRODUCTION_INVENTORY_PATH="$tmp/invalid-id.json" fly_production_inventory_validate >/dev/null 2>&1; then
  printf 'inventory must reject malformed Machine IDs\n' >&2
  exit 1
fi

jq '.resources.raw_queue_worker.app = "different-builder"' "$valid_inventory" \
  > "$tmp/mismatched-app.json"
if FLY_PRODUCTION_INVENTORY_PATH="$tmp/mismatched-app.json" \
  fly_production_inventory_validate >/dev/null 2>&1; then
  printf 'queue worker and publication builder must belong to one app\n' >&2
  exit 1
fi

printf 'raw_release_inventory=ok\n'
