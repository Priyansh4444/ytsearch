#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
runner="$SCRIPT_DIR/run-raw-publication-canary.sh"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir -p "$tmp/bin"

source_commit="$(git -C "$SCRIPT_DIR/.." rev-parse HEAD)"
base_digest="sha256:$(printf '%064d' 3)"
reader_image="registry.fly.io/ytsearch-raw-production:raw-overlay-${source_commit:0:12}-$(printf '%.12s' "${base_digest#sha256:}")"
builder_image="registry.fly.io/ytsearch-indexer-raw-production:raw-indexer-${source_commit:0:12}-$(printf '%.12s' "${base_digest#sha256:}")"

jq -n --arg source "$source_commit" --arg base "$base_digest" \
  --arg image "$reader_image" '[{
    id:"22222222222222",instance_id:"reader-version-1",state:"stopped",
    host_status:"ok",incomplete_config:false,private_ip:"fdaa::2",
    image_ref:{digest:("sha256:" + ("4" * 64))},
    config:{
      image:$image,
      env:{
        RAW_SEARCH_BASE_CHANNELS_GZIP:"/data/source/youtube_youtubers.jsonl.gz",
        RAW_SEARCH_BASE_CHUNKS_GZIP:"/data/source/youtube_transcript_chunks.jsonl.gz",
        RAW_SEARCH_BIND:"[::]:8108",RAW_SEARCH_INDEX_PATH:"/data/index",
        RAW_SEARCH_REQUIRE_FILE_SECRETS:"1",
        RAW_SEARCH_STATE_DIRECTORY:("/data/mutable/trials/" + $source[0:12]),RUST_LOG:"youtube_raw_search=info"
      },
      init:{swap_size_mb:512},guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      metadata:{lifecycle:"temporary-canary",source_commit:$source,base_image_digest:$base},
      mounts:[{volume:"vol_1111111111111111",path:"/data/mutable"}],
      restart:{policy:"no"},dns:{skip_registration:true},services:[],checks:{},files:[
        {guest_path:"/run/secrets/raw-search-admin-api-key",secret_name:"RAW_SEARCH_ADMIN_API_KEY"},
        {guest_path:"/run/secrets/raw-search-api-key",secret_name:"RAW_SEARCH_API_KEY"}
      ]
    }
  }]' > "$tmp/reader-list.json"
jq '.[0]' "$tmp/reader-list.json" > "$tmp/reader-state.json"

jq -n --arg source "$source_commit" --arg base "$base_digest" \
  --arg image "$builder_image" '[{
    id:"33333333333333",instance_id:"builder-version-1",state:"stopped",
    host_status:"ok",incomplete_config:false,
    image_ref:{digest:("sha256:" + ("5" * 64))},
    config:{
      image:$image,
      env:{
        RAW_PUBLICATION_MIN_FREE_BYTES:"8000000000",
        RAW_PUBLICATION_WORK_DIRECTORY:"/var/lib/ytsearch/raw-generation-work",
        RAW_SEARCH_API_URL:"http://[fdaa::2]:8108",
        YTSEARCH_MACHINE_ROLE:"publication-builder"
      },
      init:{entrypoint:["/usr/bin/sleep"],cmd:["infinity"],swap_size_mb:512},
      guest:{cpu_kind:"shared",cpus:2,memory_mb:2048},
      metadata:{
        lifecycle:"on-demand-publication-builder",source_commit:$source,
        base_image_digest:$base,builder_profile:"shared-2x",
        target_machine:"22222222222222"
      },
      mounts:[{volume:"vol_2222222222222222",path:"/var/lib/ytsearch"}],
      restart:{policy:"no"},dns:{skip_registration:true},services:[],checks:{},files:[]
    }
  }]' > "$tmp/builder-list.json"

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
case "$1 $2" in
  "machine list")
    case " $* " in
      *" --app ytsearch-raw-production "*) cat "$FAKE_READER_LIST" ;;
      *" --app ytsearch-indexer-raw-production "*) cat "$FAKE_BUILDER_LIST" ;;
      *) exit 1 ;;
    esac
    ;;
  "ssh console")
    printf '%s\nregistry.fly.io/ytsearch-raw-production:base@%s\n' \
      "$FAKE_SOURCE_COMMIT" "$FAKE_BASE_DIGEST"
    ;;
  "proxy 18108:8108")
    trap 'exit 0' TERM INT
    while :; do sleep 1; done
    ;;
  *) printf 'unexpected flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$tmp/bin/flyctl"

cat > "$tmp/bin/curl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
method=GET
output=""
url=""
write_status=0
header_files=()
printf '%q ' "$@" >> "$FAKE_CURL_ARGV_LOG"
printf '\n' >> "$FAKE_CURL_ARGV_LOG"
while (( $# > 0 )); do
  case "$1" in
    --request) method="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    --write-out) write_status=1; shift 2 ;;
    --header)
      [[ "$2" == @* ]] && header_files+=("${2#@}")
      shift 2
      ;;
    --data-binary|--connect-timeout|--max-time) shift 2 ;;
    --silent|--show-error|--fail) shift ;;
    http://*|https://*) url="$1"; shift ;;
    *) printf 'unexpected curl argument: %s\n' "$1" >&2; exit 1 ;;
  esac
done
case "$url" in
  */admin/*)
    admin_header_ok=0
    for header_file in "${header_files[@]}"; do
      if [[ "$(stat -c '%a' "$header_file")" == 600 ]] \
        && [[ "$(cat "$header_file")" == "x-raw-admin-key: $RAW_ADMIN_API_KEY" ]]; then
        admin_header_ok=1
      fi
    done
    if (( admin_header_ok == 0 )); then
      printf 'admin request is missing its mode-600 header file\n' >&2
      exit 1
    fi
    ;;
esac
printf '%s %s\n' "$method" "$url" >> "$FAKE_CURL_LOG"
body='{}'
status=200
case "$method:$url" in
  POST:*/machines/22222222222222/lease)
    expires="${FAKE_LEASE_EXPIRES_AT:-$(( $(date +%s) + 20000 ))}"
    body="$(jq -cn --argjson expires "$expires" '{data:{nonce:"fake-nonce",expires_at:$expires}}')"
    status=201
    ;;
  GET:*/machines/22222222222222)
    if [[ "${FAKE_READER_DIGEST_DRIFT:-0}" == 1 ]]; then
      body="$(jq '.image_ref.digest = ("sha256:" + ("9" * 64))' "$FAKE_READER_STATE")"
    else
      body="$(cat "$FAKE_READER_STATE")"
    fi
    ;;
  POST:*/machines/22222222222222/start)
    jq '.state = "started"' "$FAKE_READER_STATE" > "$FAKE_READER_STATE.next"
    mv "$FAKE_READER_STATE.next" "$FAKE_READER_STATE"
    body="$(cat "$FAKE_READER_STATE")"
    ;;
  POST:*/machines/22222222222222/stop)
    stop_count="$(( $(cat "$FAKE_STOP_COUNT") + 1 ))"
    printf '%s\n' "$stop_count" > "$FAKE_STOP_COUNT"
    if [[ "${FAKE_FAIL_STOP_AT:-0}" -gt 0 && "$stop_count" -ge "$FAKE_FAIL_STOP_AT" ]]; then
      status=500
      body='{"error":"stop failed"}'
    else
      jq '.state = "stopped"' "$FAKE_READER_STATE" > "$FAKE_READER_STATE.next"
      mv "$FAKE_READER_STATE.next" "$FAKE_READER_STATE"
      body="$(cat "$FAKE_READER_STATE")"
    fi
    ;;
  DELETE:*/machines/22222222222222/lease) status=204 ;;
  GET:*/health) body='{"ready":true}' ;;
  GET:*/admin/readiness)
    active="$(cat "$FAKE_ACTIVE_SEQUENCE")"
    body="$(jq -cn --arg source "$FAKE_SOURCE_COMMIT" --arg generation "$active" \
      --argjson active "$active" '{publication_api_version:1,source_commit:$source,generation:$generation,last_sequence:$active,acknowledged_readers:1}')"
    ;;
  GET:*/admin/operations/operation-upsert)
    body='{"state":"Published","sequence":10,"acknowledged_readers":1}'
    ;;
  GET:*/admin/operations/operation-delete)
    body='{"state":"Published","sequence":11,"acknowledged_readers":1}'
    ;;
  GET:*/admin/active/channels/protocol-channel)
    if (( $(cat "$FAKE_ACTIVE_SEQUENCE") >= 11 )); then
      body='null'
    else
      body="$(jq -cn --arg run "$CANARY_RUN_ID" '{
        id:"protocol-channel",channel_id:"protocol-channel",
        handle:("@ytsearch_protocol_" + $run),name:("ytsearch protocol " + $run),
        uploads_playlist:null,public_video_count:0,indexed_video_count:0,indexed_chunk_count:0
      }')"
    fi
    ;;
  DELETE:*/admin/channels/protocol-channel*)
    body='{"id":"operation-delete","sequence":11,"state":"Durable"}'
    status=202
    ;;
  *) printf 'unexpected curl request: %s %s\n' "$method" "$url" >&2; exit 1 ;;
esac
if [[ -n "$output" ]]; then
  printf '%s\n' "$body" > "$output"
else
  printf '%s\n' "$body"
fi
if (( write_status == 1 )); then
  printf '%s' "$status"
fi
SH
chmod +x "$tmp/bin/curl"

cat > "$tmp/bin/protocol-check" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'protocol\n' >> "$FAKE_COMPONENT_LOG"
printf '{"channel_id":"protocol-channel","operation_id":"operation-upsert","sequence":10}\n'
SH
cat > "$tmp/bin/publication-benchmark" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf 'benchmark:%s\n' "$BASELINE_POLICY" >> "$FAKE_COMPONENT_LOG"
[[ "$CONCURRENCY" == 4 ]]
(( PUBLISH_PROCESS_TIMEOUT_SECONDS > PUBLICATION_TIMEOUT_SECONDS ))
(( PUBLISH_PROCESS_KILL_AFTER_SECONDS > 30 ))
printf '10\n' > "$FAKE_ACTIVE_SEQUENCE"
mkdir -p "$OUTPUT_DIR"
printf '{"ok":true}\n' > "$OUTPUT_DIR/fake-benchmark.json"
jq -cn --argjson concurrency "$CONCURRENCY" '{
  build:{samples:$concurrency,errors:0,workers:$concurrency,
    workers_observed_across_publication:$concurrency,
    requests_overlapping_publication:$concurrency,peak_in_flight_requests:$concurrency,
    minimum_worker_covered_ms:120000,required_concurrency_covered_ms:120000},
  activation:{samples:$concurrency,errors:0,workers:$concurrency,
    workers_observed_across_publication:$concurrency,
    requests_overlapping_publication:$concurrency,peak_in_flight_requests:$concurrency,
    minimum_worker_covered_ms:10000,required_concurrency_covered_ms:10000},
  activation_call:{samples:$concurrency,errors:0,workers:$concurrency,
    workers_observed_across_publication:$concurrency,
    requests_overlapping_publication:$concurrency,peak_in_flight_requests:$concurrency,
    minimum_worker_covered_ms:50,required_concurrency_covered_ms:50},
  handshake:{activation_ready_ms:100,activation_continue_ms:200,
    activation_started_ms:210,activation_finished_ms:260,publication_ended_ms:300}
}' > "$OUTPUT_DIR/load-evidence.json"
printf '[{"phase":"build","samples":4,"errors":0,"max_ms":10}]\n' \
  > "$OUTPUT_DIR/summary.json"
printf '{"samples":1,"scrape_errors":0,"max_rss_bytes":1024,"search_error_delta":0}\n' \
  > "$OUTPUT_DIR/resource-summary.json"
printf '{"samples":1,"probe_errors":0,"max_memory_peak_bytes":1024}\n' \
  > "$OUTPUT_DIR/machine-resource-summary.json"
printf '{"samples":1,"probe_errors":0,"max_memory_peak_bytes":2048}\n' \
  > "$OUTPUT_DIR/builder-resource-summary.json"
SH
cat > "$tmp/bin/publisher" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ "$*" == 'publish --backend raw' ]]
(( PUBLISH_PROCESS_TIMEOUT_SECONDS > PUBLICATION_TIMEOUT_SECONDS ))
(( PUBLISH_PROCESS_KILL_AFTER_SECONDS > 30 ))
printf 'publisher\n' >> "$FAKE_COMPONENT_LOG"
printf '11\n' > "$FAKE_ACTIVE_SEQUENCE"
mkdir -p "$(dirname -- "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT")"
printf '{"state":"started","memory_peak_bytes":1024,"swap_peak_bytes":0,"disk_percent":1,"oom":0,"oom_kill":0,"cgroup_version":1,"memory_peak_source":"sampled_current","swap_peak_source":"sampled_current"}\n' \
  > "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT"
SH
cat > "$tmp/bin/resource-probe" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '{"state":"started","memory_peak_bytes":1024,"swap_peak_bytes":0,"disk_percent":1,"oom":0,"oom_kill":0,"cgroup_version":1,"memory_peak_source":"sampled_current","swap_peak_source":"sampled_current"}\n'
SH
chmod +x "$tmp/bin/protocol-check" "$tmp/bin/publication-benchmark" \
  "$tmp/bin/publisher" "$tmp/bin/resource-probe"

reset_state() {
  jq '.[0] | .state = "stopped"' "$tmp/reader-list.json" > "$tmp/reader-state.json"
  printf '9\n' > "$tmp/active-sequence"
  printf '0\n' > "$tmp/stop-count"
  : > "$tmp/fly.log"
  : > "$tmp/curl.log"
  : > "$tmp/curl-argv.log"
  : > "$tmp/components.log"
}

set_builder_profile() {
  local profile="$1"
  local cpu_kind="$2"
  local cpus="$3"
  local memory_mb="${4:-2048}"
  jq --arg profile "$profile" --arg cpu_kind "$cpu_kind" --argjson cpus "$cpus" \
    --argjson memory_mb "$memory_mb" '
    .[0].config.metadata.builder_profile = $profile |
    .[0].config.guest.cpu_kind = $cpu_kind |
    .[0].config.guest.cpus = $cpus |
    .[0].config.guest.memory_mb = $memory_mb
  ' "$tmp/builder-list.json" > "$tmp/builder-list.next.json"
  mv "$tmp/builder-list.next.json" "$tmp/builder-list.json"
}

common_env=(
  PATH="$tmp/bin:$PATH"
  FLY_API_TOKEN=fake-token
  FLY_API_HOSTNAME=https://api.test
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
  FAKE_READER_LIST="$tmp/reader-list.json"
  FAKE_BUILDER_LIST="$tmp/builder-list.json"
  FAKE_READER_STATE="$tmp/reader-state.json"
  FAKE_ACTIVE_SEQUENCE="$tmp/active-sequence"
  FAKE_STOP_COUNT="$tmp/stop-count"
  FAKE_FLY_LOG="$tmp/fly.log"
  FAKE_CURL_LOG="$tmp/curl.log"
  FAKE_CURL_ARGV_LOG="$tmp/curl-argv.log"
  FAKE_COMPONENT_LOG="$tmp/components.log"
  FAKE_SOURCE_COMMIT="$source_commit"
  FAKE_BASE_DIGEST="$base_digest"
  SEARCH_API_KEY=test-search-key
  RAW_ADMIN_API_KEY=012345678901234567890123
  EXPECTED_RAW_READER_IMAGE="$reader_image@sha256:$(printf '4%.0s' {1..64})"
  EXPECTED_RAW_BUILDER_IMAGE="$builder_image@sha256:$(printf '5%.0s' {1..64})"
  ALLOW_RAW_CANARY_GATE=run-exact-isolated-canary
  RAW_CANARY_TEST_MODE=fixture-only
  CANARY_PROTOCOL_CHECK_BIN="$tmp/bin/protocol-check"
  CANARY_BENCHMARK_BIN="$tmp/bin/publication-benchmark"
  CANARY_PUBLISH_BIN="$tmp/bin/publisher"
  CANARY_READER_RESOURCE_PROBE="$tmp/bin/resource-probe"
  CANARY_BUILDER_RESOURCE_PROBE="$tmp/bin/resource-probe"
  PUBLICATION_TIMEOUT_SECONDS=10
  WARM_NEW_SECONDS=1
  RAW_CANARY_STATE_POLL_ATTEMPTS=1
  RAW_CANARY_STATE_POLL_INTERVAL_SECONDS=0
  RAW_CANARY_STOP_RETRIES=1
  RAW_CANARY_STOP_POLL_ATTEMPTS=1
)

reset_state
env "${common_env[@]}" CONCURRENCY=1 CANARY_RUN_ID=success-run OUTPUT_DIR="$tmp/success" \
  "$runner" > "$tmp/success.stdout"
jq -e '
  .publication_under_load and .deleted_channel_absent and
  .policy.builder_profile == "shared-2x" and
  .policy.publication_timeout_seconds == 10 and
  .policy.publication_load_seconds == 120 and
  .policy.activation_load_seconds == 10 and
  .policy.load_minimum_coverage_percent == 80 and
  .policy.maximum_cgroup_memory_bytes == 1717986918 and
  .policy.maximum_builder_cgroup_memory_bytes == 1717986918 and
  .policy.reader_memory_mb == 2048 and .policy.builder_memory_mb == 2048 and
  .evidence.latency[0].max_ms == 10 and
  .evidence.reader_resource.max_memory_peak_bytes == 1024 and
  .evidence.builder_resource.max_memory_peak_bytes == 2048 and
  .policy.lease_ttl_seconds == 21600 and
  .policy.publication_search_concurrency == 4 and
  (.reader_image | test("@sha256:[0-9a-f]{64}$")) and
  (.builder_image | test("@sha256:[0-9a-f]{64}$"))
' "$tmp/success/test-result.json" >/dev/null
[[ ! -e "$tmp/success/result.json" ]]
if grep -F -e 'fake-token' -e '012345678901234567890123' "$tmp/curl-argv.log" >/dev/null; then
  printf 'secrets must not enter curl argv\n' >&2
  exit 1
fi
[[ "$(jq -r '.state' "$tmp/reader-state.json")" == stopped ]]
[[ "$(grep -c 'DELETE .*/lease' "$tmp/curl.log")" == 1 ]]
[[ "$(grep -c 'POST .*/start$' "$tmp/curl.log")" == 3 ]]
[[ "$(grep -c 'POST .*/stop$' "$tmp/curl.log")" == 3 ]]
[[ "$(grep -c '^protocol$' "$tmp/components.log")" == 1 ]]
[[ "$(grep -c '^benchmark:strict$' "$tmp/components.log")" == 1 ]]
[[ "$(grep -c '^publisher$' "$tmp/components.log")" == 1 ]]

reset_state
env "${common_env[@]}" RAW_CANARY_MIGRATION_MODE=record-old-generation \
  CANARY_RUN_ID=migration-run OUTPUT_DIR="$tmp/migration" "$runner" >/dev/null
jq -e '.policy.migration_mode == "record-old-generation"' \
  "$tmp/migration/test-result.json" >/dev/null
[[ "$(grep -c '^benchmark:record-only$' "$tmp/components.log")" == 1 ]]

set_builder_profile performance-1x-4gb performance 1 4096
reset_state
env "${common_env[@]}" RAW_PUBLICATION_BUILDER_PROFILE=performance-1x-4gb \
  CANARY_RUN_ID=performance-builder OUTPUT_DIR="$tmp/performance-builder" \
  "$runner" >/dev/null
jq -e '
  .policy.builder_profile == "performance-1x-4gb" and
  .policy.reader_memory_mb == 2048 and
  .policy.builder_memory_mb == 4096 and
  .policy.maximum_builder_cgroup_memory_bytes == 3435973836
' "$tmp/performance-builder/test-result.json" >/dev/null
set_builder_profile shared-2x shared 2

reset_state
short_expiry="$(( $(date +%s) + 60 ))"
if env "${common_env[@]}" CANARY_RUN_ID=short-lease OUTPUT_DIR="$tmp/short-lease" \
  FAKE_LEASE_EXPIRES_AT="$short_expiry" "$runner" >/dev/null 2>&1; then
  printf 'short canary leases must fail before start\n' >&2
  exit 1
fi

reset_state
formerly_accepted_expiry="$(( $(date +%s) + 6000 ))"
if env "${common_env[@]}" CANARY_RUN_ID=incomplete-builder-budget \
  OUTPUT_DIR="$tmp/incomplete-builder-budget" \
  FAKE_LEASE_EXPIRES_AT="$formerly_accepted_expiry" "$runner" >/dev/null 2>&1; then
  printf 'reader lease must budget both complete builder processes\n' >&2
  exit 1
fi
! grep -q '/start$' "$tmp/curl.log"

reset_state
if env "${common_env[@]}" CANARY_RUN_ID=reader-digest-drift \
  OUTPUT_DIR="$tmp/reader-digest-drift" FAKE_READER_DIGEST_DRIFT=1 \
  "$runner" >/dev/null 2>&1; then
  printf 'reader digest drift after lease must fail\n' >&2
  exit 1
fi
! grep -q '/start$' "$tmp/curl.log"
grep -q 'DELETE .*/lease' "$tmp/curl.log"
[[ ! -e "$tmp/short-lease/result.json" ]]
[[ ! -e "$tmp/short-lease/test-result.json" ]]
if grep -q '/start$' "$tmp/curl.log"; then
  printf 'short canary leases must not start the reader\n' >&2
  exit 1
fi

reset_state
if env "${common_env[@]}" CANARY_RUN_ID=failed-final-stop OUTPUT_DIR="$tmp/failed-stop" \
  RAW_RELEASE_RECOVERY_ROOT="$tmp/failed-recovery" FAKE_FAIL_STOP_AT=3 \
  "$runner" >/dev/null 2>&1; then
  printf 'an unconfirmed final reader stop must fail the canary\n' >&2
  exit 1
fi
[[ ! -e "$tmp/failed-stop/result.json" ]]
[[ ! -e "$tmp/failed-stop/test-result.json" ]]
test -f "$tmp"/failed-recovery/reader-canary-*/lease.header
test -f "$tmp"/failed-recovery/reader-canary-*/start.intent
if grep -q 'DELETE .*/lease' "$tmp/curl.log"; then
  printf 'an unconfirmed stop must retain the lease\n' >&2
  exit 1
fi

printf 'raw_publication_canary_runner=ok\n'
