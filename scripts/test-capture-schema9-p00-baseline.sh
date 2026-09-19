#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin" "$tmp/output"

printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'case "$1" in' \
  '  status) printf '\''%s\n'\'' '\''{"Machines":[{"id":"reader-1"}]}'\'' ;;' \
  '  machines) printf '\''%s\n'\'' '\''[{"id":"machine-1"}]'\'' ;;' \
  '  *) exit 2 ;;' \
  'esac' > "$tmp/bin/flyctl"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'url="${!#}"' \
  'case "$url" in' \
  '  */admin/readiness) printf '\''%s\n'\'' '\''{"generation":"00000000000000002950","last_sequence":2950}'\'' ;;' \
  '  */api/availability) printf '\''%s\n'\'' '\''{"search_enabled":true,"mode":"always_on"}'\'' ;;' \
  '  *) exit 22 ;;' \
  'esac' > "$tmp/bin/curl"
chmod +x "$tmp/bin/flyctl" "$tmp/bin/curl"

secret='p00-test-secret-that-must-not-leak'
PATH="$tmp/bin:$PATH" RAW_SEARCH_URL=http://reader.internal:8108 \
RAW_ADMIN_API_KEY="$secret" PRODUCTION_FRONTEND_URL=https://search.example \
P00_OUTPUT="$tmp/output" "$SCRIPT_DIR/capture-schema9-p00-baseline.sh" >/dev/null

jq -e '
  .status == "pass" and .generation == "00000000000000002950" and
  .last_sequence == 2950 and .metrics.reader_machines == 1 and
  .metrics.api_machines == 1 and .metrics.indexer_machines == 1 and
  .metrics.matrix == "pass" and .metrics.contract == "pass"
' "$tmp/output/receipt.json" >/dev/null
if rg --fixed-strings "$secret" "$tmp/output" >/dev/null; then
  printf 'P00 collector leaked its admin secret\n' >&2
  exit 1
fi

printf 'P00 capture tests passed\n'
