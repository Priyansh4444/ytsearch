#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_release
FLY_RESOURCE_APP="$RAW_APP" \
FLY_RESOURCE_MACHINE_ID="$RAW_CANARY_MACHINE_ID" \
FLY_RESOURCE_MOUNT_PATH="$RAW_CANARY_MOUNT_PATH" \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh"
