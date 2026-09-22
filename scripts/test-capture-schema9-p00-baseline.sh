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
  '  status) if [[ "${P00_TEST_FAIL_READER:-}" == 1 ]]; then exit 43; fi; printf '\''%s\n'\'' '\''{"Machines":[{"id":"reader-1"}]}'\'' ;;' \
  '  machines) printf '\''%s\n'\'' '\''[{"id":"machine-1"}]'\'' ;;' \
  '  *) exit 2 ;;' \
  'esac' > "$tmp/bin/flyctl"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'set -euo pipefail' \
  'url="${!#}"' \
  'case "$url" in' \
  '  */admin/readiness) if [[ "${P00_TEST_BAD_READINESS:-}" == 1 ]]; then printf '\''%s\n'\'' '\''not-json'\''; else printf '\''%s\n'\'' '\''{"generation":"00000000000000002950","last_sequence":2950}'\''; fi ;;' \
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

mkdir -p "$tmp/failed-output"
if PATH="$tmp/bin:$PATH" RAW_SEARCH_URL=http://reader.internal:8108 \
  RAW_ADMIN_API_KEY="$secret" PRODUCTION_FRONTEND_URL=https://search.example \
  P00_TEST_FAIL_READER=1 P00_OUTPUT="$tmp/failed-output" \
  "$SCRIPT_DIR/capture-schema9-p00-baseline.sh" >/dev/null 2>&1; then
  printf 'failed reader probe unexpectedly passed\n' >&2
  exit 1
fi
jq -e '
  .status == "fail" and
  (.failure_cases | any(. == "reader app status probe failed")) and
  .metrics.reader_machines == null
' "$tmp/failed-output/receipt.json" >/dev/null

mkdir -p "$tmp/malformed-output"
if PATH="$tmp/bin:$PATH" RAW_SEARCH_URL=http://reader.internal:8108 \
  RAW_ADMIN_API_KEY="$secret" PRODUCTION_FRONTEND_URL=https://search.example \
  P00_TEST_BAD_READINESS=1 P00_OUTPUT="$tmp/malformed-output" \
  "$SCRIPT_DIR/capture-schema9-p00-baseline.sh" >/dev/null 2>&1; then
  printf 'malformed readiness probe unexpectedly passed\n' >&2
  exit 1
fi
jq -e '
  .status == "fail" and
  (.failure_cases | any(. == "readiness response invalid")) and
  .generation == null and .last_sequence == null
' "$tmp/malformed-output/receipt.json" >/dev/null

mkdir -p "$tmp/multi-failure-output"
if PATH="$tmp/bin:$PATH" RAW_SEARCH_URL=http://reader.internal:8108 \
  RAW_ADMIN_API_KEY="$secret" PRODUCTION_FRONTEND_URL=https://search.example \
  P00_TEST_FAIL_READER=1 P00_TEST_BAD_READINESS=1 \
  P00_OUTPUT="$tmp/multi-failure-output" \
  "$SCRIPT_DIR/capture-schema9-p00-baseline.sh" >/dev/null 2>&1; then
  printf 'multiple failed probes unexpectedly passed\n' >&2
  exit 1
fi
jq -e '
  .status == "fail" and .attempted == 5 and .passed == 3 and .failed == 2 and
  (.failure_cases | any(. == "reader app status probe failed")) and
  (.failure_cases | any(. == "readiness response invalid"))
' "$tmp/multi-failure-output/receipt.json" >/dev/null
if rg --fixed-strings "$secret" "$tmp/output" "$tmp/failed-output" \
  "$tmp/malformed-output" "$tmp/multi-failure-output" >/dev/null; then
  printf 'P00 collector leaked its admin secret\n' >&2
  exit 1
fi

printf 'P00 capture tests passed\n'
