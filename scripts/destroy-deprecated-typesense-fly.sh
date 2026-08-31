#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_validate
# shellcheck source=scripts/lib/fly-typesense-retirement.sh
source "$SCRIPT_DIR/lib/fly-typesense-retirement.sh"

confirmation="${APPLY_TYPESENSE_RETIREMENT:-}"
case "$confirmation" in
  verify-exact-deprecated-typesense-resources) ;;
  *)
  cat >&2 <<'EOF'
Preservation mode only. This branch cannot destroy Typesense resources.

To verify the complete stopped rollback set without changing it, run:
  APPLY_TYPESENSE_RETIREMENT=verify-exact-deprecated-typesense-resources \
    scripts/destroy-deprecated-typesense-fly.sh

Any future destruction requires a new explicit user decision and a separately
reviewed implementation. The historical destructive implementation is retained
on the Typesense archive branch as trial evidence only.
EOF
  exit 0
  ;;
esac

flyctl auth whoami >/dev/null 2>&1 || {
  echo "Fly authentication is required; run 'flyctl auth login' first." >&2
  exit 1
}

# Resolve the complete preservation set and fail if it drifts or restarts.
fly_typesense_verify_exact_sets
fly_typesense_verify_all_deprecated

echo "Verified the four exact stopped deprecated Machines and complete seven-volume preservation set. No resources were changed."
