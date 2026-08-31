#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_release
FLY_RESOURCE_APP="$RAW_BUILDER_APP" \
FLY_RESOURCE_MACHINE_ID="$RAW_BUILDER_MACHINE_ID" \
FLY_RESOURCE_MOUNT_PATH="$RAW_BUILDER_MOUNT_PATH" \
FLY_RESOURCE_ALLOW_STOPPED=1 \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh"
