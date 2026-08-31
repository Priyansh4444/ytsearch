#!/usr/bin/env bash
set -euo pipefail

# Prove that one exact Raw reader implements the release publication protocol.
# Invalid probes must never mutate state. The valid idempotency probe is opt-in
# because it intentionally leaves one durable canary channel mutation pending.

: "${SEARCH_URL:?set SEARCH_URL to the exact Raw reader base URL}"
: "${RAW_ADMIN_API_KEY:?set RAW_ADMIN_API_KEY}"
: "${EXPECTED_SOURCE_COMMIT:?set EXPECTED_SOURCE_COMMIT to the full release commit}"

if [[ ! "$EXPECTED_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  printf 'EXPECTED_SOURCE_COMMIT must be a full Git commit OID\n' >&2
  exit 1
fi
if [[ "${ALLOW_PROTOCOL_MUTATION:-0}" == 1 ]]; then
  : "${PROTOCOL_RUN_ID:?set PROTOCOL_RUN_ID for the durable idempotency probe}"
  if [[ ! "$PROTOCOL_RUN_ID" =~ ^[a-z0-9][a-z0-9-]{0,47}$ ]]; then
    printf 'PROTOCOL_RUN_ID must contain 1-48 lowercase letters, digits, or hyphens\n' >&2
    exit 1
  fi
fi

for command in curl jq; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

temporary_directory="$(mktemp -d)"
admin_header="$temporary_directory/admin.header"
response="$temporary_directory/response.json"
cleanup() {
  rm -rf "$temporary_directory"
}
trap cleanup EXIT INT TERM
chmod 700 "$temporary_directory"
if [[ -z "$RAW_ADMIN_API_KEY" || "$(printf '%s' "$RAW_ADMIN_API_KEY" | tr -d '\r\n')" != "$RAW_ADMIN_API_KEY" ]]; then
  printf 'RAW_ADMIN_API_KEY must be a non-empty, single-line value\n' >&2
  exit 1
fi
printf 'x-raw-admin-key: %s\n' "$RAW_ADMIN_API_KEY" > "$admin_header"
chmod 600 "$admin_header"

admin_status() {
  curl --silent --show-error --connect-timeout 3 --max-time 10 \
    --output "$response" --write-out '%{http_code}' --header "@$admin_header" "$@"
}

readiness() {
  status="$(admin_status "${SEARCH_URL%/}/admin/readiness")"
  if [[ "$status" != 200 ]]; then
    printf 'Raw readiness returned HTTP %s\n' "$status" >&2
    exit 1
  fi
  jq -ce --arg source "$EXPECTED_SOURCE_COMMIT" '
    select(
      .backend == "raw" and
      .ready == true and
      .publication_api_version == 1 and
      .source_commit == $source and
      (.generation | type) == "string" and
      (.generation | test("^[0-9]{20}$")) and
      (.last_sequence | type) == "number" and
      .last_sequence >= 0 and
      (.last_sequence | floor) == .last_sequence
    )
  ' "$response"
}

initial_readiness="$(readiness)"
initial_last="$(jq -r '.last_sequence' <<<"$initial_readiness")"
probe_id="__ytsearch_protocol_probe__"

missing_version_body="$(jq -cn --arg id "$probe_id" '{
  idempotency_key:"protocol-missing-version",
  document:{
    id:$id,
    handle:"@ytsearch_protocol_probe",
    name:"ytsearch protocol probe",
    channel_id:$id,
    uploads_playlist:null,
    public_video_count:0,
    indexed_video_count:0,
    indexed_chunk_count:0
  }
}')"
status="$(admin_status --request PUT --header 'Content-Type: application/json' \
  --data-binary "$missing_version_body" \
  "${SEARCH_URL%/}/admin/channels/$probe_id")"
if [[ "$status" != 422 ]]; then
  printf 'missing publication version returned HTTP %s, expected 422\n' "$status" >&2
  exit 1
fi

mismatched_version_body="$(jq -cn --arg id "$probe_id" '{
  publication_api_version:2,
  idempotency_key:"protocol-mismatched-version",
  document:{
    id:$id,
    handle:"@ytsearch_protocol_probe",
    name:"ytsearch protocol probe",
    channel_id:$id,
    uploads_playlist:null,
    public_video_count:0,
    indexed_video_count:0,
    indexed_chunk_count:0
  }
}')"
status="$(admin_status --request PUT --header 'Content-Type: application/json' \
  --data-binary "$mismatched_version_body" \
  "${SEARCH_URL%/}/admin/channels/$probe_id")"
if [[ "$status" != 409 ]]; then
  printf 'mismatched publication version returned HTTP %s, expected 409\n' "$status" >&2
  exit 1
fi

after_invalid="$(readiness)"
after_invalid_last="$(jq -r '.last_sequence' <<<"$after_invalid")"
if [[ "$after_invalid_last" != "$initial_last" ]]; then
  printf 'invalid protocol probes consumed a sequence: before=%s after=%s\n' \
    "$initial_last" "$after_invalid_last" >&2
  exit 1
fi

if [[ "${ALLOW_PROTOCOL_MUTATION:-0}" != 1 ]]; then
  jq -cn --argjson last_sequence "$initial_last" \
    '{protocol:"v1",invalid_mutations_rejected:true,last_sequence:$last_sequence,mutation_created:false}'
  exit 0
fi

channel_id="__ytsearch_protocol_${PROTOCOL_RUN_ID}__"
idempotency_key="protocol-channel-${PROTOCOL_RUN_ID}"
valid_body="$(jq -cn --arg id "$channel_id" --arg run "$PROTOCOL_RUN_ID" \
  --arg key "$idempotency_key" '{
    publication_api_version:1,
    idempotency_key:$key,
    document:{
      id:$id,
      handle:("@ytsearch_protocol_" + $run),
      name:("ytsearch protocol " + $run),
      channel_id:$id,
      uploads_playlist:null,
      public_video_count:0,
      indexed_video_count:0,
      indexed_chunk_count:0
    }
  }')"

status="$(admin_status --request PUT --header 'Content-Type: application/json' \
  --data-binary "$valid_body" "${SEARCH_URL%/}/admin/channels/$channel_id")"
if [[ "$status" != 202 ]]; then
  printf 'valid publication v1 mutation returned HTTP %s, expected 202\n' "$status" >&2
  exit 1
fi
first_operation="$(jq -ce '
  select(
    (.id | type) == "string" and (.id | length) > 0 and
    (.sequence | type) == "number" and
    .state == "Durable" and
    .acknowledged_readers == 0 and
    .error == null
  )
' "$response")"

status="$(admin_status --request PUT --header 'Content-Type: application/json' \
  --data-binary "$valid_body" "${SEARCH_URL%/}/admin/channels/$channel_id")"
if [[ "$status" != 202 ]]; then
  printf 'idempotent publication v1 replay returned HTTP %s, expected 202\n' "$status" >&2
  exit 1
fi
second_operation="$(jq -ce '.' "$response")"
if [[ "$second_operation" != "$first_operation" ]]; then
  printf 'idempotent mutation replay returned a different operation\n' >&2
  exit 1
fi

final_readiness="$(readiness)"
final_last="$(jq -r '.last_sequence' <<<"$final_readiness")"
operation_sequence="$(jq -r '.sequence' <<<"$first_operation")"
if (( operation_sequence == initial_last + 1 )); then
  if (( final_last != initial_last + 1 )); then
    printf 'new mutation did not advance the sequence exactly once\n' >&2
    exit 1
  fi
elif (( operation_sequence <= initial_last )); then
  if (( final_last != initial_last )); then
    printf 'replayed mutation unexpectedly advanced the sequence\n' >&2
    exit 1
  fi
else
  printf 'mutation sequence jumped: before=%s operation=%s\n' \
    "$initial_last" "$operation_sequence" >&2
  exit 1
fi

jq -cn --arg channel_id "$channel_id" --arg operation_id "$(jq -r '.id' <<<"$first_operation")" \
  --argjson sequence "$operation_sequence" --argjson initial_last_sequence "$initial_last" \
  --argjson final_last_sequence "$final_last" '{
    protocol:"v1",
    invalid_mutations_rejected:true,
    mutation_created:($sequence > $initial_last_sequence),
    channel_id:$channel_id,
    operation_id:$operation_id,
    sequence:$sequence,
    initial_last_sequence:$initial_last_sequence,
    final_last_sequence:$final_last_sequence
  }'
