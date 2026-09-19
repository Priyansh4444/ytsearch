#!/usr/bin/env bash
set -euo pipefail

# Read-only P00 collector. It never starts a machine, creates a mutation, or
# requests publication. Secrets are passed to curl through a mode-600 file and
# are never written to the receipt.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
: "${RAW_SEARCH_URL:?set RAW_SEARCH_URL}"
: "${RAW_ADMIN_API_KEY:?set RAW_ADMIN_API_KEY}"
PRODUCTION_FRONTEND_URL="${PRODUCTION_FRONTEND_URL:-https://search.pronsh.dev}"
READER_APP="${READER_APP:-ytsearch-raw-production}"
API_APP="${API_APP:-ytsearch-api-raw-production}"
INDEXER_APP="${INDEXER_APP:-ytsearch-indexer-raw-production}"
OUTPUT="${P00_OUTPUT:-$ROOT/docs/experiments/search/schema9/P00/capture-$(date -u +%Y%m%dT%H%M%SZ)}"

for command in curl flyctl git jq node sha256sum; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done
if [[ "$(printf '%s' "$RAW_ADMIN_API_KEY" | tr -d '\r\n')" != "$RAW_ADMIN_API_KEY" ]]; then
  printf 'RAW_ADMIN_API_KEY must be a single-line value\n' >&2
  exit 1
fi

mkdir -p "$OUTPUT"
OUTPUT="$(cd -- "$OUTPUT" && pwd)"
secret_dir="$(mktemp -d)"
trap 'rm -rf "$secret_dir"' EXIT INT TERM
chmod 700 "$secret_dir"
admin_header="$secret_dir/admin.header"
printf 'x-raw-admin-key: %s\n' "$RAW_ADMIN_API_KEY" > "$admin_header"
chmod 600 "$admin_header"

started_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
source_commit="$(git -C "$ROOT" rev-parse HEAD)"
parent_commit="$(git -C "$ROOT" rev-parse HEAD^)"
tree_digest="$(git -C "$ROOT" rev-parse HEAD^{tree})"

git -C "$ROOT" worktree list --porcelain > "$OUTPUT/worktrees.txt"
git -C "$ROOT" status --short > "$OUTPUT/status.txt"
flyctl status --app "$READER_APP" --json > "$OUTPUT/reader-app.json"
flyctl machines list --app "$API_APP" --json > "$OUTPUT/api-machines.json"
flyctl machines list --app "$INDEXER_APP" --json > "$OUTPUT/indexer-machines.json"
curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
  --header "@$admin_header" "${RAW_SEARCH_URL%/}/admin/readiness" \
  > "$OUTPUT/readiness.json"
curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
  "${PRODUCTION_FRONTEND_URL%/}/api/availability" > "$OUTPUT/availability.json"

matrix_status=pass
node "$ROOT/tests/schema9/validate-query-matrix.mjs" > "$OUTPUT/matrix-check.txt" 2>&1 || matrix_status=fail
contract_status=pass
test -s "$ROOT/docs/architecture/lazy-exact/query-contract-v2.md" || contract_status=fail

generation="$(jq -er '.generation' "$OUTPUT/readiness.json")"
last_sequence="$(jq -er '.last_sequence' "$OUTPUT/readiness.json")"
reader_machine_count="$(jq -r '.Machines | length' "$OUTPUT/reader-app.json")"
api_machine_count="$(jq -r 'length' "$OUTPUT/api-machines.json")"
indexer_machine_count="$(jq -r 'length' "$OUTPUT/indexer-machines.json")"
samples_digest="$(sha256sum "$OUTPUT/readiness.json" | cut -d ' ' -f1)"
finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

status=pass
failures='[]'
if [[ "$matrix_status" != pass || "$contract_status" != pass ]]; then
  status=fail
  failures='["matrix or query-contract structural validation failed"]'
fi
if [[ "$reader_machine_count" != 1 || "$api_machine_count" != 1 || "$indexer_machine_count" -lt 1 ]]; then
  status=fail
  failures="$(jq -cn --argjson old "$failures" '$old + ["unexpected production machine cardinality"]')"
fi

jq -n \
  --arg gate P00-read-only-capture --arg status "$status" \
  --arg source_commit "$source_commit" --arg parent_commit "$parent_commit" \
  --arg tree_digest "$tree_digest" --arg generation "$generation" \
  --argjson last_sequence "$last_sequence" --arg started_at "$started_at" \
  --arg finished_at "$finished_at" --arg samples_digest "sha256:$samples_digest" \
  --arg matrix_status "$matrix_status" --arg contract_status "$contract_status" \
  --argjson reader_machine_count "$reader_machine_count" \
  --argjson api_machine_count "$api_machine_count" \
  --argjson indexer_machine_count "$indexer_machine_count" \
  --argjson failures "$failures" '
  {
    gate:$gate,status:$status,source_commit:$source_commit,parent_commit:$parent_commit,
    tree_digest:$tree_digest,generation:$generation,last_sequence:$last_sequence,
    storage_schema:null,grammar_version:2,rank_version:2,
    command:"scripts/capture-schema9-p00-baseline.sh (secrets omitted)",
    started_at:$started_at,finished_at:$finished_at,process_handle:"completed",
    attempted:5,passed:(if $status == "pass" then 5 else 4 end),
    failed:(if $status == "pass" then 0 else 1 end),skipped:0,
    samples_path:"readiness.json",samples_digest:$samples_digest,
    metrics:{matrix:$matrix_status,contract:$contract_status,reader_machines:$reader_machine_count,
      api_machines:$api_machine_count,indexer_machines:$indexer_machine_count},
    failure_cases:$failures,reviewer:"pending-independent-P00-verdict"
  }' > "$OUTPUT/receipt.json"

jq -e '.status == "pass"' "$OUTPUT/receipt.json" >/dev/null
printf 'P00 capture: %s\n' "$OUTPUT/receipt.json"
