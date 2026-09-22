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
storage_schema=null
active_documents=null
active_videos=null
active_channels=null
active_manifest=null
attempted=5
reader_machine_id=
reader_state_directory=

if flyctl status --app "$READER_APP" --json > "$OUTPUT/reader-app.json" 2>/dev/null; then
  reader_machine_count="$(jq -sce 'select(length == 1) | .[0].Machines | select(type == "array") | length' "$OUTPUT/reader-app.json" 2>/dev/null)" || {
    reader_machine_count=null
    failure_cases+=("reader app status response invalid")
  }
  if [[ "$reader_machine_count" == 1 ]]; then
    reader_location="$(jq -sce '
      select(length == 1) | .[0].Machines[0] |
      select((.id | type) == "string" and (.config.env.RAW_SEARCH_STATE_DIRECTORY | type) == "string") |
      [.id, .config.env.RAW_SEARCH_STATE_DIRECTORY]
    ' "$OUTPUT/reader-app.json" 2>/dev/null)" || reader_location=
    if [[ -n "$reader_location" ]]; then
      reader_machine_id="$(jq -r '.[0]' <<< "$reader_location")"
      reader_state_directory="$(jq -r '.[1]' <<< "$reader_location")"
    fi
    # These values become SSH command arguments, so reject shell syntax and traversal.
    if [[ ! "$reader_machine_id" =~ ^[0-9a-f]+$ ||
      ! "$reader_state_directory" =~ ^/[A-Za-z0-9_./-]+$ ||
      "$reader_state_directory" == *..* ]]; then
      reader_machine_id=
      reader_state_directory=
      failure_cases+=("reader machine location invalid")
    fi
  fi
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
    if [[ ! "$(jq -r '.[0]' <<< "$readiness_values")" =~ ^[0-9]{20}$ ]]; then
      generation=null
      last_sequence=null
      failure_cases+=("readiness generation invalid")
    fi
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

if [[ -n "$reader_machine_id" && -n "$reader_state_directory" &&
  "$generation" != null && "$last_sequence" != null ]]; then
  attempted=$((attempted + 1))
  if flyctl ssh console --app "$READER_APP" --machine "$reader_machine_id" \
    --command "cat $reader_state_directory/active-generation.json" \
    > "$OUTPUT/active-generation.json" 2>/dev/null; then
    pointer="$(jq -sce '
      select(length == 1) | .[0] | select(type == "object") |
      select((.sequence | type) == "number" and
        (.sequence | . >= 0 and . == floor) and
        (.directory | type) == "string") |
      [.sequence, .directory]
    ' "$OUTPUT/active-generation.json" 2>/dev/null)" || pointer=
    if [[ -z "$pointer" ]]; then
      failure_cases+=("active generation pointer invalid")
    else
      expected_directory="generations/$(jq -r . <<< "$generation")"
      if [[ "$(jq -r '.[0]' <<< "$pointer")" != "$last_sequence" ||
        "$(jq -r '.[1]' <<< "$pointer")" != "$expected_directory" ]]; then
        failure_cases+=("active generation pointer differs from readiness")
      else
        attempted=$((attempted + 1))
        if flyctl ssh console --app "$READER_APP" --machine "$reader_machine_id" \
          --command "cat $reader_state_directory/$expected_directory/manifest.json" \
          > "$OUTPUT/active-manifest.json" 2>/dev/null; then
          manifest="$(jq -sce '
            select(length == 1) | .[0] | select(type == "object") |
            select((.schema_version | type) == "number" and (.schema_version | . > 0 and . == floor) and
              (.documents | type) == "number" and (.documents | . >= 0 and . == floor) and
              (.videos | type) == "number" and (.videos | . >= 0 and . == floor) and
              (.channels | type) == "number" and (.channels | . >= 0 and . == floor) and
              (.files | type) == "object" and (.files | length) > 0 and
              all(.files[]; (.bytes | type) == "number" and (.bytes | . >= 0 and . == floor) and
                (.blake3 | type) == "string" and (.blake3 | test("^[0-9a-f]{64}$")))) |
            [.schema_version, .documents, .videos, .channels]
          ' "$OUTPUT/active-manifest.json" 2>/dev/null)" || manifest=
          if [[ -n "$manifest" ]]; then
            storage_schema="$(jq -c '.[0]' <<< "$manifest")"
            active_documents="$(jq -c '.[1]' <<< "$manifest")"
            active_videos="$(jq -c '.[2]' <<< "$manifest")"
            active_channels="$(jq -c '.[3]' <<< "$manifest")"
            manifest_digest="$(sha256sum "$OUTPUT/active-manifest.json" | cut -d ' ' -f1)"
            active_manifest="$(jq -cn --arg directory "$expected_directory" \
              --arg digest "sha256:$manifest_digest" '{directory:$directory,digest:$digest}')"
          else
            failure_cases+=("active generation manifest invalid")
          fi
        else
          failure_cases+=("active generation manifest probe failed")
        fi
      fi
    fi
  else
    failure_cases+=("active generation pointer probe failed")
  fi
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
  --argjson storage_schema "$storage_schema" \
  --argjson active_documents "$active_documents" \
  --argjson active_videos "$active_videos" \
  --argjson active_channels "$active_channels" \
  --argjson active_manifest "$active_manifest" \
  --argjson attempted "$attempted" \
  --argjson failures "$failures" '
  {
    gate:$gate,status:$status,source_commit:$source_commit,parent_commit:$parent_commit,
    tree_digest:$tree_digest,generation:$generation,last_sequence:$last_sequence,
    storage_schema:$storage_schema,grammar_version:2,rank_version:2,
    command:"scripts/capture-schema9-p00-baseline.sh (secrets omitted)",
    started_at:$started_at,finished_at:$finished_at,process_handle:"completed",
    attempted:$attempted,passed:($attempted - $probe_failures),failed:$probe_failures,skipped:(7 - $attempted),
    samples_path:"readiness.json",samples_digest:$samples_digest,
    metrics:{matrix:$matrix_status,contract:$contract_status,reader_machines:$reader_machine_count,
      api_machines:$api_machine_count,indexer_machines:$indexer_machine_count,
      active_documents:$active_documents,active_videos:$active_videos,
      active_channels:$active_channels},
    active_manifest:$active_manifest,
    failure_cases:$failures,reviewer:"pending-independent-P00-verdict"
  }' > "$OUTPUT/receipt.json"

jq -e '.status == "pass"' "$OUTPUT/receipt.json" >/dev/null
printf 'P00 capture: %s\n' "$OUTPUT/receipt.json"
