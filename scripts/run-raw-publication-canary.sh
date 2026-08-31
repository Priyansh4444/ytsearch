#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=scripts/lib/fly-machine-session.sh
source "$SCRIPT_DIR/lib/fly-machine-session.sh"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_release
: "${SEARCH_API_KEY:?set SEARCH_API_KEY to the isolated canary search key}"
: "${RAW_ADMIN_API_KEY:?set RAW_ADMIN_API_KEY}"
: "${EXPECTED_RAW_READER_IMAGE:?set EXPECTED_RAW_READER_IMAGE from reader staging output}"
: "${EXPECTED_RAW_BUILDER_IMAGE:?set EXPECTED_RAW_BUILDER_IMAGE from builder staging output}"
: "${RAW_CANARY_TEST_MODE:=0}"
: "${LOCAL_PORT:=18108}"
: "${RAW_CANARY_LEASE_TTL_SECONDS:=21600}"
: "${RAW_CANARY_STATE_POLL_ATTEMPTS:=60}"
: "${RAW_CANARY_STATE_POLL_INTERVAL_SECONDS:=2}"
: "${RAW_CANARY_STOP_RETRIES:=3}"
: "${RAW_CANARY_STOP_POLL_ATTEMPTS:=10}"
: "${MAX_CGROUP_MEMORY_BYTES:=1986422374}"
: "${MAX_SWAP_BYTES:=0}"
: "${MAX_DISK_PERCENT:=80}"
: "${MAX_ACCEPTABLE_MS:=200}"
: "${MAX_RSS_BYTES:=1610612736}"
: "${PUBLICATION_TIMEOUT_SECONDS:=2400}"
: "${WARM_NEW_SECONDS:=60}"
: "${CANARY_CASES_FILE:=$SCRIPT_DIR/benchmark-search-cases.tsv}"
: "${CANARY_BASELINE_RUNS:=3}"
: "${CANARY_CONCURRENCY:=4}"
: "${CANARY_SEARCH_CURL_MAX_TIME:=5}"
: "${CANARY_HEALTH_ATTEMPTS:=300}"
: "${CANARY_CLEANUP_RESERVE_SECONDS:=600}"
: "${CANARY_MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS:=1}"
: "${CANARY_MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS:=6000}"
: "${FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS:=10}"
: "${RAW_BUILDER_COMMAND_TIMEOUT_SECONDS:=120}"
: "${RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS:=15}"
: "${RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS:=10}"
: "${RAW_BUILDER_START_POLL_ATTEMPTS:=60}"
: "${RAW_BUILDER_START_POLL_INTERVAL_SECONDS:=1}"
: "${RAW_BUILDER_SSH_READY_ATTEMPTS:=30}"
: "${RAW_BUILDER_SSH_READY_INTERVAL_SECONDS:=2}"
: "${CANARY_PROTOCOL_CHECK_BIN:=$SCRIPT_DIR/check-raw-publication-protocol.sh}"
: "${CANARY_BENCHMARK_BIN:=$SCRIPT_DIR/benchmark-publication-isolation.sh}"
: "${CANARY_PUBLISH_BIN:=$SCRIPT_DIR/fly-publish-raw-generation.sh}"
: "${CANARY_READER_RESOURCE_PROBE:=$SCRIPT_DIR/fly-raw-canary-resource-snapshot.sh}"
: "${CANARY_BUILDER_RESOURCE_PROBE:=$SCRIPT_DIR/fly-raw-builder-resource-snapshot.sh}"

case "$RAW_CANARY_TEST_MODE" in
  0)
    # Production evidence is only comparable when its workload, limits, and
    # executable components are fixed by the release commit.
    RAW_CANARY_LEASE_TTL_SECONDS=21600
    RAW_CANARY_STATE_POLL_ATTEMPTS=60
    RAW_CANARY_STATE_POLL_INTERVAL_SECONDS=2
    RAW_CANARY_STOP_RETRIES=3
    RAW_CANARY_STOP_POLL_ATTEMPTS=10
    MAX_CGROUP_MEMORY_BYTES=1986422374
    MAX_SWAP_BYTES=0
    MAX_DISK_PERCENT=80
    MAX_ACCEPTABLE_MS=200
    MAX_RSS_BYTES=1610612736
    PUBLICATION_TIMEOUT_SECONDS=2400
    WARM_NEW_SECONDS=60
    CANARY_CASES_FILE="$SCRIPT_DIR/benchmark-search-cases.tsv"
    CANARY_BASELINE_RUNS=3
    CANARY_CONCURRENCY=4
    CANARY_SEARCH_CURL_MAX_TIME=5
    # A cold 20 GB Fly volume took 193 seconds to checksum and structurally
    # open the 8.4-million-document active generation on a shared 2x/2 GB
    # canary. Keep this above the measured cold path; query latency retains its
    # separate 200 ms fail-closed ceiling below.
    CANARY_HEALTH_ATTEMPTS=300
    CANARY_CLEANUP_RESERVE_SECONDS=600
    CANARY_MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS=1
    CANARY_MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS=6000
    FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS=10
    RAW_BUILDER_COMMAND_TIMEOUT_SECONDS=120
    RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS=15
    RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS=10
    RAW_BUILDER_START_POLL_ATTEMPTS=60
    RAW_BUILDER_START_POLL_INTERVAL_SECONDS=1
    RAW_BUILDER_SSH_READY_ATTEMPTS=30
    RAW_BUILDER_SSH_READY_INTERVAL_SECONDS=2
    CANARY_PROTOCOL_CHECK_BIN="$SCRIPT_DIR/check-raw-publication-protocol.sh"
    CANARY_BENCHMARK_BIN="$SCRIPT_DIR/benchmark-publication-isolation.sh"
    CANARY_PUBLISH_BIN="$SCRIPT_DIR/fly-publish-raw-generation.sh"
    CANARY_READER_RESOURCE_PROBE="$SCRIPT_DIR/fly-raw-canary-resource-snapshot.sh"
    CANARY_BUILDER_RESOURCE_PROBE="$SCRIPT_DIR/fly-raw-builder-resource-snapshot.sh"
    expected_production_inventory="$REPOSITORY_ROOT/config/fly-production-resources.json"
    if [[ "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH" != "$expected_production_inventory" ]]; then
      printf 'production canaries require the committed production inventory\n' >&2
      exit 1
    fi
    if [[ -n "${FLY_API_HOSTNAME:-}" \
      && "$FLY_API_HOSTNAME" != https://api.machines.dev ]]; then
      printf 'production canaries require the canonical Fly Machines API origin\n' >&2
      exit 1
    fi
    FLY_API_HOSTNAME=https://api.machines.dev
    export FLY_API_HOSTNAME
    ;;
  fixture-only) ;;
  *)
    printf 'RAW_CANARY_TEST_MODE must be 0 or fixture-only\n' >&2
    exit 1
    ;;
esac
if [[ "$RAW_CANARY_TEST_MODE" == fixture-only ]]; then
  expected_fixture_inventory="$SCRIPT_DIR/testdata/fly-production-resources.json"
  if [[ "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH" != "$expected_fixture_inventory" ]] \
    || [[ "${FLY_API_HOSTNAME:-}" != https://api.test ]]; then
    printf 'fixture-only canaries require the committed synthetic inventory and api.test\n' >&2
    exit 1
  fi
fi

if [[ "${ALLOW_RAW_CANARY_GATE:-}" != run-exact-isolated-canary ]]; then
  printf 'set ALLOW_RAW_CANARY_GATE=run-exact-isolated-canary\n' >&2
  exit 2
fi
if [[ ! "$LOCAL_PORT" =~ ^[0-9]+$ ]] || (( LOCAL_PORT < 1024 || LOCAL_PORT > 65535 )); then
  printf 'LOCAL_PORT must be between 1024 and 65535\n' >&2
  exit 1
fi
if [[ ! "$RAW_CANARY_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]] \
  || [[ ! "$RAW_BUILDER_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'canary and builder Machine IDs must be exact 14-character IDs\n' >&2
  exit 1
fi
if [[ ! "$EXPECTED_RAW_READER_IMAGE" =~ ^registry\.fly\.io/[a-z0-9][a-z0-9-]{0,62}:[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}$ ]] \
  || [[ ! "$EXPECTED_RAW_BUILDER_IMAGE" =~ ^registry\.fly\.io/[a-z0-9][a-z0-9-]{0,62}:[a-zA-Z0-9._-]+@sha256:[0-9a-f]{64}$ ]]; then
  printf 'expected reader and builder images must be exact tagged sha256 references\n' >&2
  exit 1
fi
if [[ ! "$RAW_CANARY_LEASE_TTL_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_CANARY_STATE_POLL_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$RAW_CANARY_STOP_RETRIES" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_CANARY_STOP_POLL_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$PUBLICATION_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$WARM_NEW_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_BASELINE_RUNS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_CONCURRENCY" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_SEARCH_CURL_MAX_TIME" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_HEALTH_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_CLEANUP_RESERVE_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$CANARY_MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_START_POLL_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_START_POLL_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_ATTEMPTS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$RAW_BUILDER_SSH_READY_INTERVAL_SECONDS" =~ ^[0-9]+$ ]] \
  || [[ ! "$MAX_ACCEPTABLE_MS" =~ ^[1-9][0-9]*$ ]] \
  || [[ ! "$MAX_RSS_BYTES" =~ ^[1-9][0-9]*$ ]]; then
  printf 'canary lease and polling settings must be valid integers\n' >&2
  exit 1
fi
for command in awk curl flyctl git jq timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done
if [[ ! -s "$CANARY_CASES_FILE" ]]; then
  printf 'canary benchmark case file is missing or empty: %s\n' "$CANARY_CASES_FILE" >&2
  exit 1
fi
for executable in "$CANARY_PROTOCOL_CHECK_BIN" "$CANARY_BENCHMARK_BIN" \
  "$CANARY_PUBLISH_BIN" "$CANARY_READER_RESOURCE_PROBE" \
  "$CANARY_BUILDER_RESOURCE_PROBE"; do
  if [[ ! -x "$executable" ]]; then
    printf 'canary component is not executable: %s\n' "$executable" >&2
    exit 1
  fi
done

source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
if [[ ! "$source_commit" =~ ^[0-9a-f]{40}$ ]]; then
  printf 'unexpected Git commit: %s\n' "$source_commit" >&2
  exit 1
fi
if [[ "$RAW_CANARY_TEST_MODE" == 0 ]]; then
  gate_inputs=(
    config/fly-production-resources.json
    scripts/run-raw-publication-canary.sh
    scripts/check-raw-publication-protocol.sh
    scripts/benchmark-publication-isolation.sh
    scripts/benchmark-search-engine.sh
    scripts/benchmark-search-cases.tsv
    scripts/benchmark-search-contract.json
    scripts/fly-publish-raw-generation.sh
    scripts/fly-machine-resource-snapshot.sh
    scripts/fly-raw-canary-resource-snapshot.sh
    scripts/fly-raw-builder-resource-snapshot.sh
    scripts/summarize-publication-load.sh
    scripts/lib/fly-machine-session.sh
    scripts/lib/fly-production-inventory.sh
  )
  for gate_input in "${gate_inputs[@]}"; do
    git -C "$REPOSITORY_ROOT" ls-files --error-unmatch "$gate_input" >/dev/null 2>&1 || {
      printf 'production canary input is not committed: %s\n' "$gate_input" >&2
      exit 1
    }
  done
  if [[ -n "$(git -C "$REPOSITORY_ROOT" status --porcelain=v1 --untracked-files=all -- \
    "${gate_inputs[@]}")" ]]; then
    printf 'production canary inputs differ from source commit %s\n' "$source_commit" >&2
    exit 1
  fi
fi
case_count="$(awk -F '\t' 'NF >= 2 && $1 !~ /^#/ && $1 != "" { count += 1 } END { print count + 0 }' \
  "$CANARY_CASES_FILE")"
if (( case_count < 1 )); then
  printf 'canary benchmark case file has no queries\n' >&2
  exit 1
fi
search_sweep_budget=$(( case_count * CANARY_SEARCH_CURL_MAX_TIME ))
start_phase_budget=$((
  (2 * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
  (RAW_CANARY_STATE_POLL_ATTEMPTS *
    (FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS + RAW_CANARY_STATE_POLL_INTERVAL_SECONDS))
))
stop_phase_budget=$((
  RAW_CANARY_STOP_RETRIES *
  (((RAW_CANARY_STOP_POLL_ATTEMPTS + 2) * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
    (RAW_CANARY_STOP_POLL_ATTEMPTS * RAW_CANARY_STATE_POLL_INTERVAL_SECONDS))
))
builder_cleanup_budget=$((
  RAW_CANARY_STOP_RETRIES *
  (((RAW_CANARY_STOP_POLL_ATTEMPTS + 2) * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
    (RAW_CANARY_STOP_POLL_ATTEMPTS * RAW_CANARY_STATE_POLL_INTERVAL_SECONDS)) +
  FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS + CANARY_CLEANUP_RESERVE_SECONDS
))
builder_start_budget=$((
  (2 * FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS) +
  (RAW_BUILDER_START_POLL_ATTEMPTS *
    (FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS +
      RAW_BUILDER_START_POLL_INTERVAL_SECONDS))
))
builder_ssh_ready_budget=$((
  RAW_BUILDER_SSH_READY_ATTEMPTS *
  (RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS +
    RAW_BUILDER_SSH_READY_INTERVAL_SECONDS)
))
builder_leased_execution_budget=$((
  builder_start_budget + builder_ssh_ready_budget +
  (2 * RAW_BUILDER_COMMAND_TIMEOUT_SECONDS) +
  PUBLICATION_TIMEOUT_SECONDS + builder_cleanup_budget
))
builder_preflight_budget=$(( 2 * RAW_BUILDER_COMMAND_TIMEOUT_SECONDS ))
publisher_process_timeout=$((
  builder_preflight_budget + builder_leased_execution_budget +
  CANARY_CLEANUP_RESERVE_SECONDS
))
publisher_process_kill_after="$builder_cleanup_budget"
benchmark_phase_budget=$((
  publisher_process_timeout + WARM_NEW_SECONDS +
  ((CANARY_BASELINE_RUNS + 6) * search_sweep_budget)
))
health_phase_budget=$(( CANARY_HEALTH_ATTEMPTS * 3 ))
minimum_lease_seconds=$((
  benchmark_phase_budget + publisher_process_timeout +
  (3 * (start_phase_budget + stop_phase_budget + health_phase_budget)) +
  CANARY_CLEANUP_RESERVE_SECONDS
))
if (( RAW_CANARY_LEASE_TTL_SECONDS < minimum_lease_seconds )); then
  printf 'Raw canary lease TTL is below the derived worst-case phase budget: ttl=%ss required=%ss\n' \
    "$RAW_CANARY_LEASE_TTL_SECONDS" "$minimum_lease_seconds" >&2
  exit 1
fi
run_id="${CANARY_RUN_ID:-${source_commit:0:12}-$(date -u +%s)}"
if [[ ! "$run_id" =~ ^[a-z0-9][a-z0-9-]{0,47}$ ]]; then
  printf 'CANARY_RUN_ID must contain 1-48 lowercase letters, digits, or hyphens\n' >&2
  exit 1
fi

canary="$(flyctl machine list --app "$RAW_APP" --json |
  jq -ce --arg id "$RAW_CANARY_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact Raw canary not found") end
  ')"
canary_is_safe() {
  jq -e --arg volume "$RAW_CANARY_VOLUME_ID" --arg source "$source_commit" \
    --arg expected_resolved_image "$EXPECTED_RAW_READER_IMAGE" '
    .state == "stopped" and
    .config.metadata.lifecycle == "temporary-canary" and
    .config.metadata.source_commit == $source and
    (.config.metadata.base_image_digest | test("^sha256:[0-9a-f]{64}$")) and
    (.image_ref.digest | test("^sha256:[0-9a-f]{64}$")) and
    .config.restart.policy == "no" and
    .config.dns.skip_registration == true and
    ((.config.services // []) | length) == 0 and
    ((.config.checks // {}) | length) == 0 and
    ((.config.mounts // []) | length) == 1 and
    .config.mounts[0].volume == $volume and
    .config.mounts[0].path == $mount_path and
    .config.guest.cpu_kind == "shared" and
    .config.guest.cpus == 2 and
    .config.guest.memory_mb == 2048 and
    .config.env.RAW_SEARCH_BASE_CHANNELS_GZIP == "/data/source/youtube_youtubers.jsonl.gz" and
    .config.env.RAW_SEARCH_BASE_CHUNKS_GZIP == "/data/source/youtube_transcript_chunks.jsonl.gz" and
    .config.env.RAW_SEARCH_BIND == "[::]:8108" and
    .config.env.RAW_SEARCH_INDEX_PATH == "/data/index" and
    .config.env.RAW_SEARCH_REQUIRE_FILE_SECRETS == "1" and
    .config.env.RAW_SEARCH_STATE_DIRECTORY == "/data/mutable" and
    .config.env.RUST_LOG == "youtube_raw_search=info" and
    ((.config.env | keys | sort) == [
      "RAW_SEARCH_BASE_CHANNELS_GZIP",
      "RAW_SEARCH_BASE_CHUNKS_GZIP",
      "RAW_SEARCH_BIND",
      "RAW_SEARCH_INDEX_PATH",
      "RAW_SEARCH_REQUIRE_FILE_SECRETS",
      "RAW_SEARCH_STATE_DIRECTORY",
      "RUST_LOG"
    ]) and
    .config.init.swap_size_mb == 512 and
    ((.config.init.entrypoint // []) | length) == 0 and
    ((.config.init.cmd // []) | length) == 0 and
    ((.config.init.exec // []) | length) == 0 and
    .config.files == [
      {guest_path:"/run/secrets/raw-search-admin-api-key",secret_name:"RAW_SEARCH_ADMIN_API_KEY"},
      {guest_path:"/run/secrets/raw-search-api-key",secret_name:"RAW_SEARCH_API_KEY"}
    ] and
    (.config.image | contains("raw-overlay-" + ($source[0:12]) + "-")) and
    ((if (.config.image | contains("@sha256:")) then .config.image
      else .config.image + "@" + .image_ref.digest end) == $expected_resolved_image)
  ' --arg mount_path "$RAW_CANARY_MOUNT_PATH" >/dev/null
}
if ! printf '%s' "$canary" | canary_is_safe; then
  printf 'refusing to start an unstaged or non-isolated Raw canary\n' >&2
  exit 1
fi
private_ip="$(jq -er '.private_ip | select(type == "string" and length > 0)' <<<"$canary")"
base_image_digest="$(jq -er '.config.metadata.base_image_digest | select(test("^sha256:[0-9a-f]{64}$"))' <<<"$canary")"
reader_resolved_image="$(jq -er '
  if (.config.image | contains("@sha256:")) then .config.image
  else .config.image + "@" + .image_ref.digest end
' <<<"$canary")"
if [[ "$reader_resolved_image" != "$EXPECTED_RAW_READER_IMAGE" ]]; then
  printf 'Raw canary resolved image differs from the reviewed staging output\n' >&2
  exit 1
fi
private_target="http://[$private_ip]:8108"

builder="$(flyctl machine list --app "$RAW_BUILDER_APP" --json |
  jq -ce --arg id "$RAW_BUILDER_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact Raw publication builder not found") end
  ')"
if ! jq -e --arg source "$source_commit" --arg target "$private_target" \
  --arg target_machine "$RAW_CANARY_MACHINE_ID" --arg volume "$RAW_BUILDER_VOLUME_ID" \
  --arg base_image_digest "$base_image_digest" '
    .state == "stopped" and
    .config.env.YTSEARCH_MACHINE_ROLE == "publication-builder" and
    .config.env.RAW_SEARCH_API_URL == $target and
    .config.init.entrypoint == ["/usr/bin/sleep"] and
    .config.init.cmd == ["infinity"] and
    .config.metadata.lifecycle == "on-demand-publication-builder" and
    .config.metadata.source_commit == $source and
    .config.metadata.target_machine == $target_machine and
    .config.metadata.base_image_digest == $base_image_digest and
    (.image_ref.digest | test("^sha256:[0-9a-f]{64}$")) and
    .config.restart.policy == "no" and
    .config.guest.cpu_kind == "shared" and
    .config.guest.cpus == 2 and
    .config.guest.memory_mb == 2048 and
    ((.config.services // []) | length) == 0 and
    ((.config.checks // {}) | length) == 0 and
    ((.config.files // []) | length) == 0 and
    .config.dns.skip_registration == true and
    ((.config.env | keys | sort) == [
      "RAW_PUBLICATION_MIN_FREE_BYTES",
      "RAW_PUBLICATION_WORK_DIRECTORY",
      "RAW_SEARCH_API_URL",
      "YTSEARCH_MACHINE_ROLE"
    ]) and
    ((.config.mounts // []) | length) == 1 and
    .config.mounts[0].volume == $volume and
    .config.mounts[0].path == $mount_path and
    (.config.image | contains("raw-indexer-" + ($source[0:12]) + "-"))
  ' --arg mount_path "$RAW_BUILDER_MOUNT_PATH" <<<"$builder" >/dev/null; then
  printf 'refusing to run with an unstaged or mismatched publication builder\n' >&2
  exit 1
fi
builder_resolved_image="$(jq -er '
  if (.config.image | contains("@sha256:")) then .config.image
  else .config.image + "@" + .image_ref.digest end
' <<<"$builder")"
if [[ "$builder_resolved_image" != "$EXPECTED_RAW_BUILDER_IMAGE" ]]; then
  printf 'publication builder resolved image differs from the reviewed staging output\n' >&2
  exit 1
fi
if [[ "$(jq -r '.config.metadata.base_image_digest // ""' <<<"$builder")" != "$base_image_digest" ]]; then
  printf 'reader and builder are not bound to the same corpus image digest\n' >&2
  exit 1
fi

output_directory="${OUTPUT_DIR:-.search-experiment/results/raw-canary-${run_id}}"
mkdir -p "$(dirname -- "$output_directory")"
if ! mkdir "$output_directory"; then
  printf 'refusing to overwrite canary evidence directory: %s\n' "$output_directory" >&2
  exit 1
fi
secret_directory="$(mktemp -d)"
chmod 700 "$secret_directory"
admin_header="$secret_directory/admin.header"
session_directory="$secret_directory/reader-session"
lease_response="$secret_directory/lease-response.json"
fresh_canary="$secret_directory/fresh-canary.json"
printf 'x-raw-admin-key: %s\n' "$RAW_ADMIN_API_KEY" > "$admin_header"
chmod 600 "$admin_header"
proxy_pid=""
lease_acquired=0

start_canary() {
  FLY_MACHINE_SESSION_START_POLL_ATTEMPTS="$RAW_CANARY_STATE_POLL_ATTEMPTS" \
  FLY_MACHINE_SESSION_START_POLL_INTERVAL_SECONDS="$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS" \
    fly_machine_session_start "$session_directory"
}

restart_canary() {
  fly_machine_session_stop "$session_directory" "$RAW_CANARY_STOP_RETRIES" \
    "$RAW_CANARY_STOP_POLL_ATTEMPTS" "$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS" \
    && start_canary
}

cleanup() {
  local prior_status="$1"
  local cleanup_status="$prior_status"
  local release_allowed=1
  local preserve_session=0
  local recovery_root="${RAW_RELEASE_RECOVERY_ROOT:-$(fly_machine_session_default_recovery_root)}"
  local recovery_destination="$recovery_root/reader-canary-${RAW_CANARY_MACHINE_ID}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
  trap - EXIT
  trap '' INT TERM
  if [[ -n "$proxy_pid" ]]; then
    kill "$proxy_pid" 2>/dev/null || true
    wait "$proxy_pid" 2>/dev/null || true
  fi
  if (( lease_acquired == 1 )) && ! fly_machine_session_stop "$session_directory" \
    "$RAW_CANARY_STOP_RETRIES" "$RAW_CANARY_STOP_POLL_ATTEMPTS" \
    "$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS"; then
    printf 'Raw canary stop could not be confirmed; retaining lease until TTL expiry\n' >&2
    cleanup_status=1
    release_allowed=0
    preserve_session=1
  fi
  if (( lease_acquired == 1 && release_allowed == 1 )); then
    if ! fly_machine_session_release "$session_directory"; then
      printf 'Raw canary lease release failed\n' >&2
      cleanup_status=1
      preserve_session=1
    fi
  fi
  if (( preserve_session == 1 )); then
    if fly_machine_session_preserve "$session_directory" "$recovery_destination" >/dev/null; then
      printf 'Raw canary recovery session preserved at %s\n' "$recovery_destination" >&2
    else
      printf 'Raw canary recovery session remains at %s\n' "$session_directory" >&2
      exit "$cleanup_status"
    fi
  fi
  rm -rf "$secret_directory"
  exit "$cleanup_status"
}
trap 'cleanup $?' EXIT
trap 'exit 130' INT TERM

fly_machine_session_create "$session_directory" "$RAW_APP" "$RAW_CANARY_MACHINE_ID"
fly_machine_session_acquire "$session_directory" 'ytsearch isolated Raw publication canary' \
  "$RAW_CANARY_LEASE_TTL_SECONDS"
lease_acquired=1
fly_machine_session_lease_response "$session_directory" "$lease_response"
lease_expires_at="$(jq -er '.data.expires_at | select(type == "number" and floor == .)' \
  "$lease_response")"
lease_seconds_remaining=$(( lease_expires_at - $(date +%s) ))
if (( lease_seconds_remaining < minimum_lease_seconds )); then
  printf 'Raw canary lease is too short: remaining=%ss required=%ss\n' \
    "$lease_seconds_remaining" "$minimum_lease_seconds" >&2
  exit 1
fi
require_lease_budget() {
  local required_seconds="$1"
  local phase="$2"
  local remaining_seconds=$(( lease_expires_at - $(date +%s) ))
  if (( remaining_seconds < required_seconds )); then
    printf 'Raw canary lease cannot safely enter %s: remaining=%ss required=%ss\n' \
      "$phase" "$remaining_seconds" "$required_seconds" >&2
    return 1
  fi
}
fly_machine_session_get "$session_directory" "$fresh_canary"
if ! canary_is_safe < "$fresh_canary" \
  || [[ "$(jq -r '.private_ip // ""' "$fresh_canary")" != "$private_ip" ]] \
  || [[ "$(jq -r '.config.metadata.base_image_digest // ""' "$fresh_canary")" \
    != "$base_image_digest" ]]; then
  printf 'Raw canary changed before its exclusive lease was acquired\n' >&2
  exit 1
fi
start_canary
flyctl proxy "$LOCAL_PORT:8108" "$private_ip" --app "$RAW_APP" --quiet \
  >"$output_directory/proxy.log" 2>&1 &
proxy_pid="$!"
search_url="http://127.0.0.1:$LOCAL_PORT"

wait_for_health() {
  for ((attempt = 1; attempt <= CANARY_HEALTH_ATTEMPTS; attempt += 1)); do
    if kill -0 "$proxy_pid" 2>/dev/null \
      && curl --fail --silent --show-error --connect-timeout 1 --max-time 2 \
        "$search_url/health" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  printf 'isolated Raw canary did not become healthy\n' >&2
  return 1
}
wait_for_health

reader_provenance="$(timeout --signal=TERM --kill-after=10 60 \
  flyctl ssh console --app "$RAW_APP" --machine "$RAW_CANARY_MACHINE_ID" \
  --command \
  "sh -c 'cat /usr/local/share/ytsearch-release/source-commit; cat /usr/local/share/ytsearch-release/base-image'")"
reader_provenance_source="$(sed -n '1p' <<<"$reader_provenance")"
reader_provenance_base="$(sed -n '2p' <<<"$reader_provenance")"
if [[ "$reader_provenance_source" != "$source_commit" ]] \
  || [[ "$reader_provenance_base" != *@"$base_image_digest" ]]; then
  printf 'Raw reader image provenance does not match the source/base release\n' >&2
  exit 1
fi

SEARCH_URL="$search_url" RAW_ADMIN_API_KEY="$RAW_ADMIN_API_KEY" \
EXPECTED_SOURCE_COMMIT="$source_commit" ALLOW_PROTOCOL_MUTATION=1 \
PROTOCOL_RUN_ID="$run_id" "$CANARY_PROTOCOL_CHECK_BIN" \
  > "$output_directory/protocol.json"
channel_id="$(jq -er '.channel_id' "$output_directory/protocol.json")"
upsert_operation="$(jq -er '.operation_id' "$output_directory/protocol.json")"
upsert_sequence="$(jq -er '.sequence' "$output_directory/protocol.json")"

require_lease_budget "$((
  benchmark_phase_budget + publisher_process_timeout +
  (2 * (start_phase_budget + stop_phase_budget + health_phase_budget)) +
  stop_phase_budget + CANARY_CLEANUP_RESERVE_SECONDS
))" 'publication-under-load'

SEARCH_URL="$search_url" SEARCH_API_KEY="$SEARCH_API_KEY" \
RAW_ADMIN_API_KEY="$RAW_ADMIN_API_KEY" ALLOW_PUBLISH_BENCHMARK=1 \
OUTPUT_DIR="$output_directory/publication" \
PUBLISH_BIN="$CANARY_PUBLISH_BIN" \
RAW_APP="$RAW_APP" RAW_CANARY_MACHINE_ID="$RAW_CANARY_MACHINE_ID" \
RAW_BUILDER_APP="$RAW_BUILDER_APP" RAW_BUILDER_MACHINE_ID="$RAW_BUILDER_MACHINE_ID" \
RAW_BUILDER_VOLUME_ID="$RAW_BUILDER_VOLUME_ID" \
RAW_TARGET_APP="$RAW_APP" RAW_PUBLICATION_TARGET_MACHINE_ID="$RAW_CANARY_MACHINE_ID" \
RAW_PUBLICATION_TARGET_URL="$private_target" RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_image_digest" \
RAW_BUILDER_EXPECTED_IMAGE="$EXPECTED_RAW_BUILDER_IMAGE" \
RAW_BUILDER_LEASE_TTL_SECONDS="$RAW_CANARY_LEASE_TTL_SECONDS" \
RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS="$builder_leased_execution_budget" \
RAW_BUILDER_COMMAND_TIMEOUT_SECONDS="$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS="$RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS" \
RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS="$PUBLICATION_TIMEOUT_SECONDS" \
RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS="$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" \
RAW_BUILDER_START_POLL_ATTEMPTS="$RAW_BUILDER_START_POLL_ATTEMPTS" \
RAW_BUILDER_START_POLL_INTERVAL_SECONDS="$RAW_BUILDER_START_POLL_INTERVAL_SECONDS" \
RAW_BUILDER_SSH_READY_ATTEMPTS="$RAW_BUILDER_SSH_READY_ATTEMPTS" \
RAW_BUILDER_SSH_READY_INTERVAL_SECONDS="$RAW_BUILDER_SSH_READY_INTERVAL_SECONDS" \
RAW_BUILDER_STOP_RETRIES="$RAW_CANARY_STOP_RETRIES" \
RAW_BUILDER_STOP_POLL_ATTEMPTS="$RAW_CANARY_STOP_POLL_ATTEMPTS" \
RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS="$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS" \
FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS="$FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS" \
MACHINE_RESOURCE_PROBE="$CANARY_READER_RESOURCE_PROBE" \
BUILDER_RESOURCE_PROBE="$CANARY_BUILDER_RESOURCE_PROBE" \
MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS="$CANARY_MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS" \
MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS="$CANARY_MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS" \
PUBLICATION_TIMEOUT_SECONDS="$PUBLICATION_TIMEOUT_SECONDS" \
PUBLISH_PROCESS_TIMEOUT_SECONDS="$publisher_process_timeout" \
PUBLISH_PROCESS_KILL_AFTER_SECONDS="$publisher_process_kill_after" \
WARM_NEW_SECONDS="$WARM_NEW_SECONDS" \
CASES_FILE="$CANARY_CASES_FILE" BASELINE_RUNS="$CANARY_BASELINE_RUNS" \
CONCURRENCY="$CANARY_CONCURRENCY" \
CURL_MAX_TIME="$CANARY_SEARCH_CURL_MAX_TIME" \
MAX_ACCEPTABLE_MS="$MAX_ACCEPTABLE_MS" MAX_RSS_BYTES="$MAX_RSS_BYTES" \
  "$CANARY_BENCHMARK_BIN"

verify_published_operation() {
  operation_id="$1"
  minimum_sequence="$2"
  readiness_file="$3"
  curl --fail --silent --show-error --connect-timeout 3 --max-time 10 --header "@$admin_header" \
    "$search_url/admin/readiness" > "$readiness_file"
  jq -e --arg source "$source_commit" --argjson sequence "$minimum_sequence" '
    .publication_api_version == 1 and .source_commit == $source and
    (.generation | tonumber) == $sequence
  ' "$readiness_file" >/dev/null
  curl --fail --silent --show-error --connect-timeout 3 --max-time 10 --header "@$admin_header" \
    "$search_url/admin/operations/$operation_id" | jq -e \
    --argjson sequence "$minimum_sequence" '
      .state == "Published" and .sequence == $sequence and .acknowledged_readers == 1
    ' >/dev/null
}

verify_channel_visible() {
  channel_file="$1"
  curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
    --header "@$admin_header" \
    "$search_url/admin/active/channels/$channel_id" > "$channel_file"
  jq -e --arg id "$channel_id" --arg run "$run_id" '
    .id == $id and
    .channel_id == $id and
    .handle == ("@ytsearch_protocol_" + $run) and
    .name == ("ytsearch protocol " + $run) and
    .uploads_playlist == null and
    .public_video_count == 0 and
    .indexed_video_count == 0 and
    .indexed_chunk_count == 0
  ' "$channel_file" >/dev/null
}

verify_resource_snapshot() {
  label="$1"
  resource_file="$2"
  jq -e --argjson maximum_memory "$MAX_CGROUP_MEMORY_BYTES" \
    --argjson maximum_swap "$MAX_SWAP_BYTES" \
    --argjson maximum_disk "$MAX_DISK_PERCENT" '
    .state == "started" and
    .memory_peak_bytes <= $maximum_memory and
    .swap_peak_bytes <= $maximum_swap and
    .disk_percent < $maximum_disk and
    (.cgroup_version == 1 or .cgroup_version == 2) and
    (.memory_peak_source == "kernel" or .memory_peak_source == "sampled_current") and
    (.swap_peak_source == "kernel" or .swap_peak_source == "sampled_current") and
    .oom == 0 and
    .oom_kill == 0
  ' "$resource_file" >/dev/null || {
    printf '%s resource snapshot failed its 2GB canary gate\n' "$label" >&2
    return 1
  }
}

capture_reader_resources() {
  resource_file="$1"
  "$CANARY_READER_RESOURCE_PROBE" > "$resource_file"
  verify_resource_snapshot reader "$resource_file"
}

verify_published_operation "$upsert_operation" "$upsert_sequence" \
  "$output_directory/readiness-after-upsert.json"
verify_channel_visible "$output_directory/channel-after-upsert.json"

restart_canary
wait_for_health
verify_published_operation "$upsert_operation" "$upsert_sequence" \
  "$output_directory/readiness-after-upsert-restart.json"
verify_channel_visible "$output_directory/channel-after-upsert-restart.json"
capture_reader_resources "$output_directory/reader-after-upsert-restart.json"

delete_key="protocol-channel-delete-${run_id}"
delete_response="$output_directory/delete-operation.json"
delete_status="$(curl --silent --show-error --output "$delete_response" --write-out '%{http_code}' \
  --connect-timeout 3 --max-time 10 --request DELETE --header "@$admin_header" \
  "$search_url/admin/channels/$channel_id?publication_api_version=1&idempotency_key=$delete_key&cascade=false")"
if [[ "$delete_status" != 202 ]]; then
  printf 'canary channel deletion returned HTTP %s\n' "$delete_status" >&2
  exit 1
fi
delete_operation="$(jq -er 'select(.state == "Durable") | .id' "$delete_response")"
delete_sequence="$(jq -er '.sequence' "$delete_response")"
if (( delete_sequence != upsert_sequence + 1 )); then
  printf 'canary deletion sequence was not contiguous\n' >&2
  exit 1
fi
verify_channel_visible "$output_directory/channel-before-delete-publication.json"

require_lease_budget "$((
  publisher_process_timeout + start_phase_budget +
  (2 * stop_phase_budget) + health_phase_budget + CANARY_CLEANUP_RESERVE_SECONDS
))" 'delete-publication'

RAW_BUILDER_APP="$RAW_BUILDER_APP" RAW_BUILDER_MACHINE_ID="$RAW_BUILDER_MACHINE_ID" \
RAW_BUILDER_VOLUME_ID="$RAW_BUILDER_VOLUME_ID" \
RAW_TARGET_APP="$RAW_APP" RAW_PUBLICATION_TARGET_MACHINE_ID="$RAW_CANARY_MACHINE_ID" \
RAW_PUBLICATION_TARGET_URL="$private_target" RAW_PUBLICATION_SOURCE_COMMIT="$source_commit" \
RAW_PUBLICATION_BASE_IMAGE_DIGEST="$base_image_digest" \
RAW_BUILDER_EXPECTED_IMAGE="$EXPECTED_RAW_BUILDER_IMAGE" \
RAW_BUILDER_LEASE_TTL_SECONDS="$RAW_CANARY_LEASE_TTL_SECONDS" \
RAW_BUILDER_MIN_LEASE_REMAINING_SECONDS="$builder_leased_execution_budget" \
RAW_BUILDER_COMMAND_TIMEOUT_SECONDS="$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS" \
RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS="$RAW_BUILDER_SSH_READY_TIMEOUT_SECONDS" \
RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS="$PUBLICATION_TIMEOUT_SECONDS" \
RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS="$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" \
RAW_BUILDER_START_POLL_ATTEMPTS="$RAW_BUILDER_START_POLL_ATTEMPTS" \
RAW_BUILDER_START_POLL_INTERVAL_SECONDS="$RAW_BUILDER_START_POLL_INTERVAL_SECONDS" \
RAW_BUILDER_SSH_READY_ATTEMPTS="$RAW_BUILDER_SSH_READY_ATTEMPTS" \
RAW_BUILDER_SSH_READY_INTERVAL_SECONDS="$RAW_BUILDER_SSH_READY_INTERVAL_SECONDS" \
RAW_BUILDER_STOP_RETRIES="$RAW_CANARY_STOP_RETRIES" \
RAW_BUILDER_STOP_POLL_ATTEMPTS="$RAW_CANARY_STOP_POLL_ATTEMPTS" \
RAW_BUILDER_STOP_POLL_INTERVAL_SECONDS="$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS" \
FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS="$FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS" \
RAW_BUILDER_RESOURCE_PROBE="$CANARY_BUILDER_RESOURCE_PROBE" \
RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$output_directory/builder-after-delete.json" \
PUBLISH_PROCESS_TIMEOUT_SECONDS="$publisher_process_timeout" \
PUBLISH_PROCESS_KILL_AFTER_SECONDS="$publisher_process_kill_after" \
  timeout --signal=TERM --kill-after="$publisher_process_kill_after" \
  "$publisher_process_timeout" \
  "$CANARY_PUBLISH_BIN" publish --backend raw \
  > "$output_directory/delete-publish.log" 2>&1
verify_resource_snapshot builder "$output_directory/builder-after-delete.json"
verify_published_operation "$delete_operation" "$delete_sequence" \
  "$output_directory/readiness-after-delete.json"

restart_canary
wait_for_health
verify_published_operation "$delete_operation" "$delete_sequence" \
  "$output_directory/readiness-after-delete-restart.json"
capture_reader_resources "$output_directory/reader-after-delete-restart.json"
channel_after_delete_file="$output_directory/channel-after-delete-restart.json"
curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
  --header "@$admin_header" \
  "$search_url/admin/active/channels/$channel_id" > "$channel_after_delete_file"
if ! jq -e '. == null' "$channel_after_delete_file" >/dev/null; then
  printf 'deleted canary channel reappeared after publication and restart\n' >&2
  exit 1
fi

fly_machine_session_stop "$session_directory" "$RAW_CANARY_STOP_RETRIES" \
  "$RAW_CANARY_STOP_POLL_ATTEMPTS" "$RAW_CANARY_STATE_POLL_INTERVAL_SECONDS"
fly_machine_session_release "$session_directory"
lease_acquired=0

attestation_scope=production
result_filename=result.json
if [[ "$RAW_CANARY_TEST_MODE" == fixture-only ]]; then
  attestation_scope=fixture-only
  result_filename=test-result.json
fi
jq -cn --arg source_commit "$source_commit" --arg run_id "$run_id" \
  --arg canary_machine "$RAW_CANARY_MACHINE_ID" --arg builder_machine "$RAW_BUILDER_MACHINE_ID" \
  --arg base_image_digest "$base_image_digest" \
  --arg reader_image "$reader_resolved_image" --arg builder_image "$builder_resolved_image" \
  --arg attestation_scope "$attestation_scope" \
  --argjson upsert_sequence "$upsert_sequence" --argjson delete_sequence "$delete_sequence" \
  --argjson maximum_ms "$MAX_ACCEPTABLE_MS" --argjson maximum_rss "$MAX_RSS_BYTES" \
  --argjson maximum_cgroup_memory "$MAX_CGROUP_MEMORY_BYTES" \
  --argjson required_concurrency "$CANARY_CONCURRENCY" \
  --slurpfile load "$output_directory/publication/load-evidence.json" \
  --slurpfile upsert_restart "$output_directory/readiness-after-upsert-restart.json" \
  --slurpfile delete_restart "$output_directory/readiness-after-delete-restart.json" \
  --slurpfile deleted_channel "$channel_after_delete_file" \
  --arg results "$output_directory" '
  {
    schema_version:1,
    attestation_scope:$attestation_scope,
    source_commit:$source_commit,
    run_id:$run_id,
    canary_machine:$canary_machine,
    builder_machine:$builder_machine,
    base_image_digest:$base_image_digest,
    reader_image:$reader_image,
    builder_image:$builder_image,
    upsert_sequence:$upsert_sequence,
    delete_sequence:$delete_sequence,
    policy:{
      maximum_search_ms:$maximum_ms,
      maximum_process_rss_bytes:$maximum_rss,
      maximum_cgroup_memory_bytes:$maximum_cgroup_memory,
      machine_memory_mb:2048,
      publication_search_concurrency:$required_concurrency
    },
    publication_under_load:(
      $load[0].errors == 0 and
      $load[0].workers == $required_concurrency and
      $load[0].workers_observed_across_publication == $required_concurrency and
      $load[0].peak_in_flight_requests >= $required_concurrency
    ),
    restart_after_upsert:(
      ($upsert_restart[0].generation | tonumber) == $upsert_sequence and
      $upsert_restart[0].source_commit == $source_commit
    ),
    restart_after_delete:(
      ($delete_restart[0].generation | tonumber) == $delete_sequence and
      $delete_restart[0].source_commit == $source_commit
    ),
    deleted_channel_absent:($deleted_channel[0] == null),
    results:$results
  } |
  select(
    .publication_under_load and .restart_after_upsert and
    .restart_after_delete and .deleted_channel_absent
  )' | tee "$output_directory/$result_filename"
