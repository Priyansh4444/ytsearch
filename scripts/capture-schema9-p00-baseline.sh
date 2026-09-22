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
failure_cases=()
reader_machine_count=null
api_machine_count=null
indexer_machine_count=null
generation=null
last_sequence=null

if flyctl status --app "$READER_APP" --json > "$OUTPUT/reader-app.json" 2>/dev/null; then
  reader_machine_count="$(jq -sce 'select(length == 1) | .[0].Machines | select(type == "array") | length' "$OUTPUT/reader-app.json" 2>/dev/null)" || {
    reader_machine_count=null
    failure_cases+=("reader app status response invalid")
  }
else
  failure_cases+=("reader app status probe failed")
fi
if flyctl machines list --app "$API_APP" --json > "$OUTPUT/api-machines.json" 2>/dev/null; then
  api_machine_count="$(jq -sce 'select(length == 1) | .[0] | select(type == "array") | length' "$OUTPUT/api-machines.json" 2>/dev/null)" || {
    api_machine_count=null
    failure_cases+=("api machines response invalid")
  }
else
  failure_cases+=("api machines probe failed")
fi
if flyctl machines list --app "$INDEXER_APP" --json > "$OUTPUT/indexer-machines.json" 2>/dev/null; then
  indexer_machine_count="$(jq -sce 'select(length == 1) | .[0] | select(type == "array") | length' "$OUTPUT/indexer-machines.json" 2>/dev/null)" || {
    indexer_machine_count=null
    failure_cases+=("indexer machines response invalid")
  }
else
  failure_cases+=("indexer machines probe failed")
fi
if curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
  --header "@$admin_header" "${RAW_SEARCH_URL%/}/admin/readiness" \
  > "$OUTPUT/readiness.json" 2>/dev/null; then
  readiness_values="$(jq -sce '
    select(length == 1) | .[0] | select(type == "object") |
    select((.generation | type == "string" and length > 0) and
      (.last_sequence | type == "number" and . >= 0 and . == floor)) |
    [.generation, .last_sequence]
  ' "$OUTPUT/readiness.json" 2>/dev/null)" || {
    readiness_values=
    failure_cases+=("readiness response invalid")
  }
  if [[ -n "$readiness_values" ]]; then
    generation="$(jq -c '.[0]' <<< "$readiness_values")"
    last_sequence="$(jq -c '.[1]' <<< "$readiness_values")"
  fi
else
  failure_cases+=("readiness probe failed")
fi
if curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
  "${PRODUCTION_FRONTEND_URL%/}/api/availability" \
  > "$OUTPUT/availability.json" 2>/dev/null; then
  if ! jq -se 'select(length == 1) | .[0] | select(type == "object") |
    select((.search_enabled | type) == "boolean" and (.mode | type) == "string")
  ' "$OUTPUT/availability.json" >/dev/null 2>&1; then
    failure_cases+=("availability response invalid")
  fi
else
  failure_cases+=("availability probe failed")
fi
probe_failures="${#failure_cases[@]}"

matrix_status=pass
node "$ROOT/tests/schema9/validate-query-matrix.mjs" > "$OUTPUT/matrix-check.txt" 2>&1 || matrix_status=fail
contract_status=pass
test -s "$ROOT/docs/architecture/lazy-exact/query-contract-v2.md" || contract_status=fail

samples_digest="$(sha256sum "$OUTPUT/readiness.json" | cut -d ' ' -f1)"
finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

if [[ "$matrix_status" != pass || "$contract_status" != pass ]]; then
  failure_cases+=("matrix or query-contract structural validation failed")
fi
if [[ "$reader_machine_count" != null && "$api_machine_count" != null && "$indexer_machine_count" != null ]] &&
  [[ "$reader_machine_count" != 1 || "$api_machine_count" != 1 || "$indexer_machine_count" -lt 1 ]]; then
  failure_cases+=("unexpected production machine cardinality")
fi
status=pass
if (( ${#failure_cases[@]} > 0 )); then status=fail; fi
failures="$(jq -cn --args '$ARGS.positional' "${failure_cases[@]}")"

jq -n \
  --arg gate P00-read-only-capture --arg status "$status" \
  --arg source_commit "$source_commit" --arg parent_commit "$parent_commit" \
  --arg tree_digest "$tree_digest" --argjson generation "$generation" \
  --argjson last_sequence "$last_sequence" --arg started_at "$started_at" \
  --arg finished_at "$finished_at" --arg samples_digest "sha256:$samples_digest" \
  --arg matrix_status "$matrix_status" --arg contract_status "$contract_status" \
  --argjson probe_failures "$probe_failures" \
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
    attempted:5,passed:(5 - $probe_failures),failed:$probe_failures,skipped:0,
    samples_path:"readiness.json",samples_digest:$samples_digest,
    metrics:{matrix:$matrix_status,contract:$contract_status,reader_machines:$reader_machine_count,
      api_machines:$api_machine_count,indexer_machines:$indexer_machine_count},
    failure_cases:$failures,reviewer:"pending-independent-P00-verdict"
  }' > "$OUTPUT/receipt.json"

jq -e '.status == "pass"' "$OUTPUT/receipt.json" >/dev/null
printf 'P00 capture: %s\n' "$OUTPUT/receipt.json"
