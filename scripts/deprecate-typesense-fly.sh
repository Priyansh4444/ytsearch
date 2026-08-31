#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_validate
# shellcheck source=scripts/lib/fly-typesense-retirement.sh
source "$SCRIPT_DIR/lib/fly-typesense-retirement.sh"

# This marks the four known legacy Machines after Raw owns both reads and
# writes. Fly Machine metadata updates can restart a Machine, so this script is
# deliberately fail-closed and must not be run before the cutover proof passes.

if [ "${APPLY_TYPESENSE_DEPRECATION:-}" != "mark-known-typesense-machines" ]; then
  cat >&2 <<'EOF'
Dry run only. No Fly resources were changed.

After the Raw read, write, queue, publication, and restart gates pass, run:
  APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines scripts/deprecate-typesense-fly.sh
EOF
  exit 0
fi

if ! flyctl auth whoami >/dev/null 2>&1; then
  echo "Fly authentication is required; run 'flyctl auth login' first." >&2
  exit 1
fi

mark_deprecated() {
  local app="$1"
  local machine="$2"
  local successor="$3"
  flyctl machine update "$machine" \
    --app "$app" \
    --metadata "lifecycle=deprecated" \
    --metadata "deprecated_reason=$retirement_reason" \
    --metadata "successor=$successor" \
    --skip-start \
    --yes
}

retirement_reason="$(jq -er '.retirement_sets.typesense.reason' \
  "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")"
fly_typesense_verify_exact_sets
fly_typesense_verify_all_base
while IFS=$'\t' read -r app machine successor; do
  mark_deprecated "$app" "$machine" "$successor"
done < <(jq -er '.retirement_sets.typesense.machines[] |
  [.app, .machine_id, .successor_app] | @tsv' \
  "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")

fly_typesense_verify_exact_sets
fly_typesense_verify_all_deprecated

echo "Marked the four known legacy Machines as deprecated. No app or volume was destroyed."
