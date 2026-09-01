#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# shellcheck disable=SC1091
. "$SCRIPT_DIR/search-benchmark-cases.sh"
validate_search_benchmark_cases "$SCRIPT_DIR/benchmark-search-cases.tsv"
validate_search_benchmark_contract "$SCRIPT_DIR/benchmark-search-contract.json"

mkdir -p "$tmp/bin"
cat > "$tmp/bin/curl" <<'EOF'
#!/usr/bin/env sh
output=/dev/null
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output) output="$2"; shift 2 ;;
    *) shift ;;
  esac
done
case "${MOCK_CURL_MODE:-valid}" in
  timeout)
    printf '{"videos":[' > "$output"
    printf '200\t5.000\t11'
    exit 28
    ;;
  malformed)
    printf '{}' > "$output"
    printf '200\t0.010\t2'
    ;;
  valid)
    printf '{"videos":[],"total_videos":0,"total_moments":0}' > "$output"
    printf '200\t0.010\t49'
    ;;
  rawvalid)
    video_id="${MOCK_RAW_VIDEO_ID:-fixture-video}"
    printf '{"found":1,"grouped_hits":[{"group_key":["%s"],"hits":[{"document":{"youtuber_handle":"@fixture"},"highlights":[{"snippet":"<mark>fixture</mark>"}]}]}]}' "$video_id" > "$output"
    printf '200\t0.010\t180'
    ;;
  rawempty)
    printf '{"found":0,"grouped_hits":[]}' > "$output"
    printf '200\t0.010\t29'
    ;;
esac
EOF
chmod +x "$tmp/bin/curl"

run_fixture() {
  mode="$1"
  results="$tmp/$mode.log"
  PATH="$tmp/bin:$PATH" MOCK_CURL_MODE="$mode" API_BASE_URL=https://fixture.invalid \
    CASE_LABEL="$mode" RESULTS_FILE="$results" RUNS=1 QUIET=1 ACCEPT_ENCODING=identity \
    "$SCRIPT_DIR/benchmark-search-backends.sh"
  "$SCRIPT_DIR/summarize-benchmark-results.sh" "$results" > "$tmp/$mode.summary"
}

run_fixture timeout
grep -F 'timeout/raw' "$tmp/timeout.summary" | grep -F "$(printf '\t0\t1\t')" >/dev/null

run_fixture malformed
grep -F 'malformed/raw' "$tmp/malformed.summary" | grep -F "$(printf '\t0\t1\t')" >/dev/null

run_fixture valid
grep -F 'valid/raw' "$tmp/valid.summary" | grep -F "$(printf '\t1\t0\t')" >/dev/null

printf 'comparison\treact\t\n' > "$tmp/comparison-cases.tsv"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=valid \
  BEFORE_API_BASE_URL=https://before.invalid AFTER_API_BASE_URL=https://after.invalid \
  CASES_FILE="$tmp/comparison-cases.tsv" RESULTS_FILE="$tmp/comparison.log" \
  RUNS=1 WARMUP_RUNS=0 "$SCRIPT_DIR/benchmark-api-comparison.sh" > "$tmp/comparison.summary"
[ "$(wc -l < "$tmp/comparison.log" | tr -d ' ')" -eq 6 ] || {
  echo "API comparison did not record both variants for every contract case" >&2
  exit 1
}
grep -F 'comparison/before' "$tmp/comparison.summary" >/dev/null
grep -F 'comparison/after' "$tmp/comparison.summary" >/dev/null

printf 'fixture_query\tfixture\t\n' > "$tmp/raw-cases.tsv"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=timeout SEARCH_URL=https://fixture.invalid \
  CASES_FILE="$tmp/raw-cases.tsv" \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 OUTPUT="$tmp/raw-timeout.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-timeout.summary"
jq -se 'length > 0 and all(.[]; .ok == false)' "$tmp/raw-timeout.jsonl" >/dev/null
jq -e 'length > 0 and all(.[]; .errors == .runs)' "$tmp/raw-timeout.summary" >/dev/null

PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid SEARCH_URL=https://fixture.invalid \
  CASES_FILE="$tmp/raw-cases.tsv" \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 OUTPUT="$tmp/raw-valid.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-valid.summary"
jq -se 'length > 0 and all(.[]; .ok == true)' "$tmp/raw-valid.jsonl" >/dev/null
jq -e 'length > 0 and all(.[]; .errors == 0)' "$tmp/raw-valid.summary" >/dev/null
initial_rows="$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid SEARCH_URL=https://fixture.invalid \
  CASES_FILE="$tmp/raw-cases.tsv" \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 APPEND_OUTPUT=1 \
  OUTPUT="$tmp/raw-valid.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-append.summary"
[ "$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')" -eq $(( initial_rows * 2 )) ] || {
  echo "append mode replaced prior benchmark samples" >&2
  exit 1
}
touch "$tmp/stop-load"
rows_before_stop="$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid SEARCH_URL=https://fixture.invalid \
  CASES_FILE="$tmp/raw-cases.tsv" \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 APPEND_OUTPUT=1 \
  STOP_FILE="$tmp/stop-load" OUTPUT="$tmp/raw-valid.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-stop.summary"
[ "$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')" -eq "$rows_before_stop" ] || {
  echo "stop file allowed additional benchmark requests" >&2
  exit 1
}

PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawempty SEARCH_URL=https://fixture.invalid \
  CASES_FILE="$tmp/raw-cases.tsv" RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 \
  OUTPUT="$tmp/raw-empty.jsonl" "$SCRIPT_DIR/benchmark-search-engine.sh" \
  > "$tmp/raw-empty.summary"
jq -se 'length > 0 and all(.[]; .ok == false)' "$tmp/raw-empty.jsonl" >/dev/null

cat > "$tmp/contract.json" <<'EOF'
{"schema_version":1,"corpus":"fixture","cases":{"fixture_contract":{"top_video_id":"fixture0001","top_handle":"@fixture","required_highlight_tokens":["fixture"]}}}
EOF
printf 'fixture_contract\tfixture\t@fixture\n' > "$tmp/contract-cases.tsv"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid MOCK_RAW_VIDEO_ID=fixture0001 \
  SEARCH_URL=https://fixture.invalid CASES_FILE="$tmp/contract-cases.tsv" \
  CONTRACT_FILE="$tmp/contract.json" RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 \
  OUTPUT="$tmp/contract-pass.jsonl" "$SCRIPT_DIR/benchmark-search-engine.sh" >/dev/null
jq -se 'all(.[]; .ok == true)' "$tmp/contract-pass.jsonl" >/dev/null
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid MOCK_RAW_VIDEO_ID=wrongvideo1 \
  SEARCH_URL=https://fixture.invalid CASES_FILE="$tmp/contract-cases.tsv" \
  CONTRACT_FILE="$tmp/contract.json" RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 \
  OUTPUT="$tmp/contract-fail.jsonl" "$SCRIPT_DIR/benchmark-search-engine.sh" >/dev/null
jq -se 'all(.[]; .ok == false)' "$tmp/contract-fail.jsonl" >/dev/null

publication_benchmark="$SCRIPT_DIR/benchmark-publication-isolation.sh"
printf 'fixture\tfixture query\t\n' > "$tmp/publication-cases.tsv"
printf '9\n' > "$tmp/active-sequence"
cat > "$tmp/bin/publication-curl" <<'EOF'
#!/usr/bin/env sh
set -eu
output=""
write_out=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --write-out) write_out="$2"; shift 2 ;;
    --connect-timeout|--max-time|--data-urlencode|--header) shift 2 ;;
    --fail|--silent|--show-error|--get) shift ;;
    http://*|https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
case "$url" in
  */admin/readiness)
    active="$(cat "$FAKE_ACTIVE_SEQUENCE")"
    printf '{"generation":"%020d","last_sequence":10}\n' "$active"
    ;;
  */metrics)
    printf '{"rss_bytes":1024,"errors":0,"searches":100}\n'
    ;;
  */documents/search)
    # Keep both fixture workers in flight long enough for a deterministic
    # overlap proof; reported engine latency remains the independent 10 ms.
    sleep 0.5
    body='{"found":1,"grouped_hits":[{"group_key":["fixture-video"],"hits":[{"document":{"youtuber_handle":"@fixture"},"highlights":[{"snippet":"<mark>fixture</mark>"}]}]}]}'
    if [ -n "$output" ]; then printf '%s\n' "$body" > "$output"; else printf '%s\n' "$body"; fi
    [ -z "$write_out" ] || printf '200\t%s\t180' "${MOCK_PUBLICATION_SECONDS:-0.010}"
    ;;
  *) printf 'unexpected publication fixture URL: %s\n' "$url" >&2; exit 1 ;;
esac
EOF
cat > "$tmp/bin/publication-publish" <<'EOF'
#!/usr/bin/env sh
set -eu
[ "$*" = 'publish --backend raw' ]
[ "$PUBLISH_PROCESS_TIMEOUT_SECONDS" = 20 ]
[ "$PUBLISH_PROCESS_KILL_AFTER_SECONDS" = 5 ]
sleep 2
printf '10\n' > "$FAKE_ACTIVE_SEQUENCE"
mkdir -p "$(dirname -- "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT")"
printf '{"state":"started","sampled_at_ms":%s,"memory_peak_bytes":2048,"swap_peak_bytes":0,"disk_percent":1,"oom":0,"oom_kill":0,"cgroup_version":1,"memory_peak_source":"sampled_current","swap_peak_source":"sampled_current"}\n' \
  "$(date +%s%3N)" > "$RAW_BUILDER_FINAL_RESOURCE_OUTPUT"
EOF
cat > "$tmp/bin/publication-resource" <<'EOF'
#!/usr/bin/env sh
set -eu
printf '{"state":"started","sampled_at_ms":%s,"memory_peak_bytes":2048,"swap_peak_bytes":0,"disk_percent":1,"oom":0,"oom_kill":0,"cgroup_version":1,"memory_peak_source":"sampled_current","swap_peak_source":"sampled_current"}\n' "$(date +%s%3N)"
EOF
chmod +x "$tmp/bin/publication-curl" "$tmp/bin/publication-publish" \
  "$tmp/bin/publication-resource"
mkdir -p "$tmp/publication-bin"
ln -s "$tmp/bin/publication-curl" "$tmp/publication-bin/curl"
PATH="$tmp/publication-bin:$PATH" SEARCH_URL=http://fixture.invalid \
  SEARCH_API_KEY=fixture-search RAW_ADMIN_API_KEY=fixture-admin \
  ALLOW_PUBLISH_BENCHMARK=1 CASES_FILE="$tmp/publication-cases.tsv" \
  OUTPUT_DIR="$tmp/publication-output" CONCURRENCY=2 BASELINE_RUNS=1 WARM_NEW_SECONDS=1 \
  PUBLICATION_TIMEOUT_SECONDS=10 RESOURCE_SAMPLE_INTERVAL_SECONDS=1 \
  PUBLISH_PROCESS_TIMEOUT_SECONDS=20 PUBLISH_PROCESS_KILL_AFTER_SECONDS=5 \
  MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS=1 \
  MACHINE_RESOURCE_PROBE="$tmp/bin/publication-resource" \
  BUILDER_RESOURCE_PROBE="$tmp/bin/publication-resource" \
  PUBLISH_BIN="$tmp/bin/publication-publish" FAKE_ACTIVE_SEQUENCE="$tmp/active-sequence" \
  "$publication_benchmark" > "$tmp/publication.stdout"
jq -e '.workers_observed_across_publication == 2 and .peak_in_flight_requests >= 2' \
  "$tmp/publication-output/load-evidence.json" >/dev/null
jq -e 'length == 4 and all(.[]; .errors == 0 and .max_ms <= 200)' \
  "$tmp/publication-output/summary.json" >/dev/null
jq -e '.scrape_errors == 0 and .search_error_delta == 0 and .max_rss_bytes <= 1610612736' \
  "$tmp/publication-output/resource-summary.json" >/dev/null
jq -e '.probe_errors == 0 and .max_oom_kill == 0' \
  "$tmp/publication-output/machine-resource-summary.json" >/dev/null

printf '9\n' > "$tmp/active-sequence"
if PATH="$tmp/publication-bin:$PATH" SEARCH_URL=http://fixture.invalid \
  SEARCH_API_KEY=fixture-search RAW_ADMIN_API_KEY=fixture-admin \
  ALLOW_PUBLISH_BENCHMARK=1 CASES_FILE="$tmp/publication-cases.tsv" \
  OUTPUT_DIR="$tmp/publication-baseline-fail" CONCURRENCY=2 BASELINE_RUNS=1 \
  MOCK_PUBLICATION_SECONDS=0.250 PUBLISH_BIN=/bin/false \
  FAKE_ACTIVE_SEQUENCE="$tmp/active-sequence" \
  "$publication_benchmark" >/dev/null 2>&1; then
  printf 'slow publication baseline unexpectedly reached the publisher\n' >&2
  exit 1
fi
jq -e '.errors == 0 and .max_ms == 250' \
  "$tmp/publication-baseline-fail/baseline-summary.json" >/dev/null

printf '9\n' > "$tmp/active-sequence"
if PATH="$tmp/publication-bin:$PATH" SEARCH_URL=http://fixture.invalid \
  SEARCH_API_KEY=fixture-search RAW_ADMIN_API_KEY=fixture-admin \
  ALLOW_PUBLISH_BENCHMARK=1 CASES_FILE="$tmp/publication-cases.tsv" \
  OUTPUT_DIR="$tmp/publication-gap-output" CONCURRENCY=2 BASELINE_RUNS=1 \
  WARM_NEW_SECONDS=1 PUBLICATION_TIMEOUT_SECONDS=10 \
  PUBLISH_PROCESS_TIMEOUT_SECONDS=20 PUBLISH_PROCESS_KILL_AFTER_SECONDS=5 \
  RESOURCE_SAMPLE_INTERVAL_SECONDS=1 MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS=2 \
  MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS=1 \
  MACHINE_RESOURCE_PROBE="$tmp/bin/publication-resource" \
  BUILDER_RESOURCE_PROBE="$tmp/bin/publication-resource" \
  PUBLISH_BIN="$tmp/bin/publication-publish" FAKE_ACTIVE_SEQUENCE="$tmp/active-sequence" \
  "$publication_benchmark" >/dev/null 2>&1; then
  printf 'sampled-current resource gates must reject unbounded sample gaps\n' >&2
  exit 1
fi

printf 'benchmark search tooling tests passed\n'
