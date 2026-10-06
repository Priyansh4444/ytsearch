#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
source_commit="$(printf '%040d' 1)"
mismatched_commit="$(printf '%040d' 2)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
trap 'exit 130' INT TERM
mkdir -p "$temporary_directory/bin"

cat > "$temporary_directory/bin/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
method=GET
output=""
body=""
url=""
header_files=()
printf '%q ' "$@" >> "$FAKE_CURL_ARGV_LOG"
printf '\n' >> "$FAKE_CURL_ARGV_LOG"
while (($#)); do
  case "$1" in
    --request) method="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    --data-binary) body="$2"; shift 2 ;;
    --header)
      [[ "$2" == @* ]] && header_files+=("${2#@}")
      shift 2
      ;;
    http://*|https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
admin_header_ok=0
for header_file in "${header_files[@]}"; do
  if [[ "$(stat -c '%a' "$header_file")" == 600 ]] \
    && [[ "$(cat "$header_file")" == "x-raw-admin-key: $RAW_ADMIN_API_KEY" ]]; then
    admin_header_ok=1
  fi
done
if (( admin_header_ok == 0 )); then
  printf 'protocol request is missing its mode-600 admin header file\n' >&2
  exit 1
fi
case "$method:$url" in
  GET:*/admin/readiness)
    jq -n --arg source "$FAKE_SOURCE_COMMIT" \
      --argjson last "$(cat "$FAKE_LAST_SEQUENCE")" '{
        backend:"raw",ready:true,generation:"00000000000000002940",
        last_sequence:$last,acknowledged_readers:1,
        publication_api_version:1,source_commit:$source
      }' > "$output"
    printf 200
    ;;
  PUT:*/admin/channels/__ytsearch_protocol_probe__)
    if ! jq -e 'has("publication_api_version")' <<<"$body" >/dev/null; then
      printf '{"error":"missing"}' > "$output"
      printf 422
    elif [[ "$(jq -r '.publication_api_version' <<<"$body")" != 1 ]]; then
      printf '{"error":"conflict"}' > "$output"
      printf 409
    else
      exit 1
    fi
    ;;
  PUT:*/admin/channels/__ytsearch_protocol_*__)
    key="$(jq -r '.idempotency_key' <<<"$body")"
    if [[ -e "$FAKE_OPERATION" ]]; then
      cat "$FAKE_OPERATION" > "$output"
    else
      last="$(cat "$FAKE_LAST_SEQUENCE")"
      sequence=$((last + 1))
      printf '%s\n' "$sequence" > "$FAKE_LAST_SEQUENCE"
      jq -n --arg id "operation-$key" --argjson sequence "$sequence" '{
        id:$id,sequence:$sequence,state:"Durable",acknowledged_readers:0,error:null
      }' > "$FAKE_OPERATION"
      cat "$FAKE_OPERATION" > "$output"
    fi
    printf 202
    ;;
  *) printf 'unexpected request: %s %s\n' "$method" "$url" >&2; exit 1 ;;
esac
SH
chmod +x "$temporary_directory/bin/curl"
printf '2940\n' > "$temporary_directory/last-sequence"
: > "$temporary_directory/curl-argv.log"

common_env=(
  PATH="$temporary_directory/bin:$PATH"
  SEARCH_URL=http://127.0.0.1:18108
  RAW_ADMIN_API_KEY=012345678901234567890123
  EXPECTED_SOURCE_COMMIT="$source_commit"
  FAKE_SOURCE_COMMIT="$source_commit"
  FAKE_LAST_SEQUENCE="$temporary_directory/last-sequence"
  FAKE_OPERATION="$temporary_directory/operation.json"
  FAKE_CURL_ARGV_LOG="$temporary_directory/curl-argv.log"
)

output="$(env "${common_env[@]}" "$SCRIPT_DIR/check-raw-publication-protocol.sh")"
jq -e '.invalid_mutations_rejected and (.mutation_created | not) and .last_sequence == 2940' \
  <<<"$output" >/dev/null
test "$(cat "$temporary_directory/last-sequence")" = 2940

output="$(env "${common_env[@]}" ALLOW_PROTOCOL_MUTATION=1 PROTOCOL_RUN_ID=run-1 \
  "$SCRIPT_DIR/check-raw-publication-protocol.sh")"
jq -e '.invalid_mutations_rejected and .mutation_created and .sequence == 2941 and .final_last_sequence == 2941' \
  <<<"$output" >/dev/null
test "$(cat "$temporary_directory/last-sequence")" = 2941

output="$(env "${common_env[@]}" ALLOW_PROTOCOL_MUTATION=1 PROTOCOL_RUN_ID=run-1 \
  "$SCRIPT_DIR/check-raw-publication-protocol.sh")"
jq -e '.invalid_mutations_rejected and (.mutation_created | not) and .sequence == 2941 and .final_last_sequence == 2941' \
  <<<"$output" >/dev/null
test "$(cat "$temporary_directory/last-sequence")" = 2941

if env "${common_env[@]}" EXPECTED_SOURCE_COMMIT="$mismatched_commit" \
  "$SCRIPT_DIR/check-raw-publication-protocol.sh" >/dev/null 2>&1; then
  printf 'source mismatch must fail\n' >&2
  exit 1
fi
if grep -F '012345678901234567890123' "$temporary_directory/curl-argv.log" >/dev/null; then
  printf 'admin key must not enter curl argv\n' >&2
  exit 1
fi

printf 'raw_publication_protocol_gate=ok\n'
