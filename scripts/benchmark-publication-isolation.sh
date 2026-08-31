#!/usr/bin/env bash
set -euo pipefail

# Measure the complete search lifecycle around an already-pending Raw publish.
# This script never creates mutations or changes deployment configuration.

: "${SEARCH_URL:?set SEARCH_URL to the Raw engine base URL}"
: "${SEARCH_API_KEY:?set SEARCH_API_KEY}"
: "${RAW_ADMIN_API_KEY:?set RAW_ADMIN_API_KEY}"
[[ "${ALLOW_PUBLISH_BENCHMARK:-0}" == 1 ]] || {
  printf 'set ALLOW_PUBLISH_BENCHMARK=1; this starts a pending publication\n' >&2
  exit 2
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CASES_FILE="${CASES_FILE:-$SCRIPT_DIR/benchmark-search-cases.tsv}"
OUTPUT_DIR="${OUTPUT_DIR:-.search-experiment/results/publication-$(date -u +%Y%m%dT%H%M%SZ)}"
CONCURRENCY="${CONCURRENCY:-4}"
BASELINE_RUNS="${BASELINE_RUNS:-3}"
WARM_NEW_SECONDS="${WARM_NEW_SECONDS:-60}"
PUBLICATION_TIMEOUT_SECONDS="${PUBLICATION_TIMEOUT_SECONDS:-2400}"
PUBLISH_PROCESS_TIMEOUT_SECONDS="${PUBLISH_PROCESS_TIMEOUT_SECONDS:-}"
PUBLISH_PROCESS_KILL_AFTER_SECONDS="${PUBLISH_PROCESS_KILL_AFTER_SECONDS:-}"
MAX_ACCEPTABLE_MS="${MAX_ACCEPTABLE_MS:-200}"
MAX_RSS_BYTES="${MAX_RSS_BYTES:-1610612736}"
RESOURCE_SAMPLE_INTERVAL_SECONDS="${RESOURCE_SAMPLE_INTERVAL_SECONDS:-1}"
MACHINE_RESOURCE_PROBE="${MACHINE_RESOURCE_PROBE:-}"
BUILDER_RESOURCE_PROBE="${BUILDER_RESOURCE_PROBE:-}"
MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS="${MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS:-5}"
MACHINE_RESOURCE_PROBE_TIMEOUT_SECONDS="${MACHINE_RESOURCE_PROBE_TIMEOUT_SECONDS:-20}"
MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS="${MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS:-6000}"
MAX_CGROUP_MEMORY_BYTES="${MAX_CGROUP_MEMORY_BYTES:-1986422374}"
MAX_SWAP_BYTES="${MAX_SWAP_BYTES:-0}"
MAX_DISK_PERCENT="${MAX_DISK_PERCENT:-80}"
PUBLISH_BIN="${PUBLISH_BIN:-$SCRIPT_DIR/../target/release/ytsearch}"

for command in curl jq timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done
[[ "$CONCURRENCY" =~ ^[1-9][0-9]*$ ]]
[[ "$MAX_RSS_BYTES" =~ ^[1-9][0-9]*$ ]]
[[ "$RESOURCE_SAMPLE_INTERVAL_SECONDS" =~ ^[1-9][0-9]*$ ]]
[[ "$MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS" =~ ^[1-9][0-9]*$ ]]
[[ "$MACHINE_RESOURCE_PROBE_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]
[[ "$MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS" =~ ^[1-9][0-9]*$ ]]
[[ "$MAX_CGROUP_MEMORY_BYTES" =~ ^[1-9][0-9]*$ ]]
[[ "$MAX_SWAP_BYTES" =~ ^[0-9]+$ ]]
[[ "$MAX_DISK_PERCENT" =~ ^[1-9][0-9]*$ ]]
[[ "$WARM_NEW_SECONDS" =~ ^[1-9][0-9]*$ ]]
[[ "$PUBLICATION_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]
if [[ -z "$PUBLISH_PROCESS_TIMEOUT_SECONDS" ]]; then
  PUBLISH_PROCESS_TIMEOUT_SECONDS=$(( PUBLICATION_TIMEOUT_SECONDS + 600 ))
fi
if [[ -z "$PUBLISH_PROCESS_KILL_AFTER_SECONDS" ]]; then
  PUBLISH_PROCESS_KILL_AFTER_SECONDS=600
fi
[[ "$PUBLISH_PROCESS_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]
[[ "$PUBLISH_PROCESS_KILL_AFTER_SECONDS" =~ ^[1-9][0-9]*$ ]]
if (( PUBLISH_PROCESS_TIMEOUT_SECONDS <= PUBLICATION_TIMEOUT_SECONDS )); then
  printf 'publisher process timeout must exceed its publication-command timeout\n' >&2
  exit 1
fi
[[ -x "$PUBLISH_BIN" ]] || {
  printf 'publication binary is not executable: %s\n' "$PUBLISH_BIN" >&2
  exit 1
}
if [[ -n "$MACHINE_RESOURCE_PROBE" && ! -x "$MACHINE_RESOURCE_PROBE" ]]; then
  printf 'Machine resource probe is not executable: %s\n' "$MACHINE_RESOURCE_PROBE" >&2
  exit 1
fi
if [[ -n "$BUILDER_RESOURCE_PROBE" && ! -x "$BUILDER_RESOURCE_PROBE" ]]; then
  printf 'Builder resource probe is not executable: %s\n' "$BUILDER_RESOURCE_PROBE" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"
: > "$OUTPUT_DIR/build.jsonl"
: > "$OUTPUT_DIR/rounds.jsonl"
: > "$OUTPUT_DIR/resources.jsonl"
: > "$OUTPUT_DIR/machine-resources.jsonl"
: > "$OUTPUT_DIR/builder-resources.jsonl"
LOAD_STOP_FILE="$OUTPUT_DIR/stop-load"
LOAD_READY_DIRECTORY="$OUTPUT_DIR/load-ready"
RESOURCE_STOP_FILE="$OUTPUT_DIR/stop-resources"
rm -f "$LOAD_STOP_FILE" "$RESOURCE_STOP_FILE"
mkdir -p "$LOAD_READY_DIRECTORY"
load_pids=()
load_outputs=()
resource_pids=()
secret_directory="$(mktemp -d)"
chmod 700 "$secret_directory"
admin_header="$secret_directory/admin.header"
if [[ "$(printf '%s' "$RAW_ADMIN_API_KEY" | tr -d '\r\n')" != "$RAW_ADMIN_API_KEY" ]]; then
  printf 'RAW_ADMIN_API_KEY must be a single-line value\n' >&2
  exit 1
fi
printf 'x-raw-admin-key: %s\n' "$RAW_ADMIN_API_KEY" > "$admin_header"
chmod 600 "$admin_header"

stop_load_workers() {
  touch "$LOAD_STOP_FILE"
  worker_failed=0
  for pid in "${load_pids[@]}"; do
    if ! wait "$pid"; then
      worker_failed=1
    fi
  done
  load_pids=()
  (( worker_failed == 0 ))
}

stop_resource_workers() {
  touch "$RESOURCE_STOP_FILE"
  resource_failed=0
  for pid in "${resource_pids[@]}"; do
    if ! wait "$pid"; then
      resource_failed=1
    fi
  done
  resource_pids=()
  (( resource_failed == 0 ))
}

cleanup() {
  stop_load_workers 2>/dev/null || true
  stop_resource_workers 2>/dev/null || true
  rm -rf "$secret_directory"
}

interrupt_load() {
  touch "$LOAD_STOP_FILE" "$RESOURCE_STOP_FILE"
  for pid in "${load_pids[@]}" "${resource_pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  printf 'load stopped; publication state is durable and the command may be retried\n' >&2
  exit 130
}
trap cleanup EXIT
trap interrupt_load INT TERM

admin_request() {
  curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
    --header "@$admin_header" "$@"
}

active_sequence() {
  admin_request "${SEARCH_URL%/}/admin/readiness" | jq -er '.generation | tonumber'
}

last_sequence() {
  admin_request "${SEARCH_URL%/}/admin/readiness" | jq -er '.last_sequence'
}

run_engine() {
  output="$1"
  runs="$2"
  warmups="$3"
  SEARCH_URL="$SEARCH_URL" SEARCH_API_KEY="$SEARCH_API_KEY" CASES_FILE="$CASES_FILE" \
    OUTPUT="$output" RUNS="$runs" WARMUP_RUNS="$warmups" CAPTURE_RESOURCES=0 \
    "$SCRIPT_DIR/benchmark-search-engine.sh" >/dev/null
}

run_timed_engine() {
  output="$1"
  duration_seconds="$2"
  : > "$output"
  deadline=$(( $(date +%s) + duration_seconds ))
  while (( $(date +%s) < deadline )); do
    SEARCH_URL="$SEARCH_URL" SEARCH_API_KEY="$SEARCH_API_KEY" CASES_FILE="$CASES_FILE" \
      OUTPUT="$output" RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 APPEND_OUTPUT=1 \
      "$SCRIPT_DIR/benchmark-search-engine.sh" >/dev/null
  done
}

load_worker() {
  output="$1"
  worker_number="$2"
  : > "$output"
  first_sweep=1
  while [[ ! -e "$LOAD_STOP_FILE" ]]; do
    SEARCH_URL="$SEARCH_URL" SEARCH_API_KEY="$SEARCH_API_KEY" CASES_FILE="$CASES_FILE" \
      OUTPUT="$output" RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 \
      APPEND_OUTPUT=1 STOP_FILE="$LOAD_STOP_FILE" \
      "$SCRIPT_DIR/benchmark-search-engine.sh" >/dev/null
    if (( first_sweep == 1 )); then
      touch "$LOAD_READY_DIRECTORY/$worker_number"
      first_sweep=0
    fi
  done
}

resource_worker() {
  while [[ ! -e "$RESOURCE_STOP_FILE" ]]; do
    sampled_at_ms="$(date +%s%3N)"
    if metrics="$(curl --fail --silent --show-error --connect-timeout 3 --max-time 5 \
      "${SEARCH_URL%/}/metrics")" \
      && resource_line="$(printf '%s' "$metrics" | jq -ce \
        --argjson sampled_at_ms "$sampled_at_ms" '
        select(
          (.rss_bytes | type) == "number" and
          (.errors | type) == "number" and
          (.searches | type) == "number"
        ) |
        {
          ok:true,
          sampled_at_ms:$sampled_at_ms,
          rss_bytes:.rss_bytes,
          errors:.errors,
          searches:.searches
        }
      ')"; then
      printf '%s\n' "$resource_line" >> "$OUTPUT_DIR/resources.jsonl"
    else
      jq -cn --argjson sampled_at_ms "$sampled_at_ms" \
        '{ok:false,sampled_at_ms:$sampled_at_ms}' >> "$OUTPUT_DIR/resources.jsonl"
    fi
    sleep "$RESOURCE_SAMPLE_INTERVAL_SECONDS"
  done
}

machine_resource_worker() {
  probe="$1"
  output="$2"
  while [[ ! -e "$RESOURCE_STOP_FILE" ]]; do
    if snapshot="$(timeout "$MACHINE_RESOURCE_PROBE_TIMEOUT_SECONDS" "$probe")" \
      && snapshot="$(jq -ce '. + {ok:true}' <<<"$snapshot")"; then
      printf '%s\n' "$snapshot" >> "$output"
    else
      jq -cn --argjson sampled_at_ms "$(date +%s%3N)" \
        '{ok:false,sampled_at_ms:$sampled_at_ms}' >> "$output"
    fi
    sleep "$MACHINE_RESOURCE_SAMPLE_INTERVAL_SECONDS"
  done
}

summarize_machine_resources() {
  input="$1"
  output="$2"
  jq -s '
    [.[] | select(.ok == true and (.state // "started") == "started")] as $samples |
    ($samples | sort_by(.sampled_at_ms)) as $ordered_samples |
    [range(1; ($ordered_samples | length)) as $index |
      ($ordered_samples[$index].sampled_at_ms -
        $ordered_samples[$index - 1].sampled_at_ms)] as $sample_gaps |
    {
      samples:($samples | length),
      probe_errors:([.[] | select(.ok != true)] | length),
      max_memory_peak_bytes:([$samples[].memory_peak_bytes] | max // null),
      max_swap_peak_bytes:([$samples[].swap_peak_bytes] | max // null),
      max_disk_percent:([$samples[].disk_percent] | max // null),
      max_oom:([$samples[].oom] | max // null),
      max_oom_kill:([$samples[].oom_kill] | max // null),
      cgroup_versions:([$samples[].cgroup_version] | unique),
      memory_peak_sources:([$samples[].memory_peak_source] | unique),
      swap_peak_sources:([$samples[].swap_peak_source] | unique),
      sample_timestamps_valid:all($samples[]; (.sampled_at_ms | type) == "number"),
      sample_span_ms:(if ($ordered_samples | length) < 2 then null else
        ($ordered_samples[-1].sampled_at_ms - $ordered_samples[0].sampled_at_ms) end),
      max_sample_gap_ms:($sample_gaps | max // null),
      oom_delta:(if ($samples | length) == 0 then null else ($samples[-1].oom - $samples[0].oom) end),
      oom_kill_delta:(if ($samples | length) == 0 then null else ($samples[-1].oom_kill - $samples[0].oom_kill) end)
    }
  ' "$input" | tee "$output"
}

gate_machine_resources() {
  label="$1"
  summary="$2"
  jq -e \
    --argjson maximum_memory "$MAX_CGROUP_MEMORY_BYTES" \
    --argjson maximum_swap "$MAX_SWAP_BYTES" \
    --argjson maximum_disk "$MAX_DISK_PERCENT" \
    --argjson maximum_sample_gap "$MAX_MACHINE_RESOURCE_SAMPLE_GAP_MS" '
      .samples > 0 and
      .probe_errors == 0 and
      .max_memory_peak_bytes != null and
      .max_memory_peak_bytes <= $maximum_memory and
      .max_swap_peak_bytes != null and
      .max_swap_peak_bytes <= $maximum_swap and
      .max_disk_percent != null and
      .max_disk_percent < $maximum_disk and
      (.cgroup_versions | length) == 1 and
      (.cgroup_versions[0] == 1 or .cgroup_versions[0] == 2) and
      (.memory_peak_sources | length) == 1 and
      (.memory_peak_sources[0] == "kernel" or
        .memory_peak_sources[0] == "sampled_current") and
      (.swap_peak_sources | length) == 1 and
      (.swap_peak_sources[0] == "kernel" or
        .swap_peak_sources[0] == "sampled_current") and
      (
        (.memory_peak_sources[0] == "kernel" and .swap_peak_sources[0] == "kernel") or
        (
          .samples >= 2 and .sample_timestamps_valid == true and
          .sample_span_ms != null and .sample_span_ms >= 1 and
          .max_sample_gap_ms != null and .max_sample_gap_ms <= $maximum_sample_gap
        )
      ) and
      .max_oom == 0 and
      .max_oom_kill == 0 and
      .oom_delta == 0 and
      .oom_kill_delta == 0
    ' "$summary" >/dev/null || {
    printf '%s resource gate failed; inspect %s\n' "$label" "$summary" >&2
    exit 1
  }
}

summarize_phase() {
  phase="$1"
  file="$2"
  jq -sc --arg phase "$phase" '
    . as $rows |
    [$rows[] | select(.ok) | .seconds * 1000] | sort as $times |
    {
      phase:$phase,
      samples:($rows | length),
      errors:([$rows[] | select(.ok | not)] | length),
      p50_ms:(if ($times|length)==0 then null else $times[((($times|length)*0.50|ceil)-1)] end),
      p95_ms:(if ($times|length)==0 then null else $times[((($times|length)*0.95|ceil)-1)] end),
      p99_ms:(if ($times|length)==0 then null else $times[((($times|length)*0.99|ceil)-1)] end),
      max_ms:(if ($times|length)==0 then null else ($times|max) end)
    }
  ' "$file"
}

initial_active="$(active_sequence)"
initial_last="$(last_sequence)"
if (( initial_last <= initial_active )); then
  printf 'publication benchmark needs pending durable work: active=%s last=%s\n' \
    "$initial_active" "$initial_last" >&2
  exit 2
fi

curl --fail --silent --show-error --max-time 5 "${SEARCH_URL%/}/metrics" \
  > "$OUTPUT_DIR/metrics-before.json"
run_engine "$OUTPUT_DIR/baseline.jsonl" "$BASELINE_RUNS" 1

target_sequence="$initial_last"
resource_worker &
resource_pids+=("$!")
if [[ -n "$MACHINE_RESOURCE_PROBE" ]]; then
  machine_resource_worker "$MACHINE_RESOURCE_PROBE" "$OUTPUT_DIR/machine-resources.jsonl" &
  resource_pids+=("$!")
fi
if [[ -n "$BUILDER_RESOURCE_PROBE" ]]; then
  machine_resource_worker "$BUILDER_RESOURCE_PROBE" "$OUTPUT_DIR/builder-resources.jsonl" &
  resource_pids+=("$!")
fi
for ((worker = 1; worker <= CONCURRENCY; worker += 1)); do
  worker_output="$OUTPUT_DIR/build-worker-$worker.jsonl"
  load_outputs+=("$worker_output")
  load_worker "$worker_output" "$worker" &
  load_pids+=("$!")
done
load_ready=0
for attempt in {1..120}; do
  ready_count="$(find "$LOAD_READY_DIRECTORY" -mindepth 1 -maxdepth 1 -type f | wc -l)"
  live_count=0
  for pid in "${load_pids[@]}"; do
    kill -0 "$pid" 2>/dev/null && ((live_count += 1))
  done
  if (( ready_count == CONCURRENCY && live_count == CONCURRENCY )); then
    load_ready=1
    break
  fi
  if (( live_count != CONCURRENCY )); then
    break
  fi
  sleep 1
done
if (( load_ready == 0 )); then
  printf 'publication load workers did not complete their ready sweeps\n' >&2
  exit 1
fi
publication_started_ms="$(date +%s%3N)"
set +e
RAW_SEARCH_API_URL="$SEARCH_URL" RAW_SEARCH_ADMIN_API_KEY="$RAW_ADMIN_API_KEY" \
  RAW_BUILDER_RESOURCE_PROBE="$BUILDER_RESOURCE_PROBE" \
  RAW_BUILDER_FINAL_RESOURCE_OUTPUT="$OUTPUT_DIR/builder-final-resource.json" \
  timeout --signal=TERM --kill-after="$PUBLISH_PROCESS_KILL_AFTER_SECONDS" \
    "$PUBLISH_PROCESS_TIMEOUT_SECONDS" \
    "$PUBLISH_BIN" publish --backend raw \
  > "$OUTPUT_DIR/publish.log" 2>&1
publish_exit=$?
set -e
publication_ended_ms="$(date +%s%3N)"
if ! stop_load_workers; then
  printf 'one or more publication load workers failed\n' >&2
  exit 1
fi
for worker_index in "${!load_outputs[@]}"; do
  jq -c --argjson worker "$(( worker_index + 1 ))" '. + {worker:$worker}' \
    "${load_outputs[$worker_index]}" >> "$OUTPUT_DIR/build.jsonl"
done
jq -cn --argjson started_ms "$publication_started_ms" \
  --argjson ended_ms "$publication_ended_ms" --argjson exit "$publish_exit" \
  --argjson target "$target_sequence" \
  '{phase:"external_publication",started_ms:$started_ms,ended_ms:$ended_ms,exit:$exit,target_sequence:$target}' \
  >> "$OUTPUT_DIR/rounds.jsonl"
if (( publish_exit != 0 )); then
  printf 'external publication failed with exit %s; see %s\n' \
    "$publish_exit" "$OUTPUT_DIR/publish.log" >&2
  exit 1
fi
if [[ -n "$BUILDER_RESOURCE_PROBE" ]]; then
  if ! jq -ce '. + {ok:true}' "$OUTPUT_DIR/builder-final-resource.json" \
    >> "$OUTPUT_DIR/builder-resources.jsonl"; then
    printf 'publication builder did not produce a valid final resource snapshot\n' >&2
    exit 1
  fi
fi

"$SCRIPT_DIR/summarize-publication-load.sh" "$OUTPUT_DIR/build.jsonl" \
  "$publication_started_ms" "$publication_ended_ms" "$CONCURRENCY" \
  | tee "$OUTPUT_DIR/load-evidence.json"

final_active="$(active_sequence)"
if (( final_active != target_sequence )); then
  printf 'publication activated an unexpected sequence: target=%s active=%s\n' \
    "$target_sequence" "$final_active" >&2
  exit 1
fi

run_engine "$OUTPUT_DIR/post-swap.jsonl" 1 0
run_engine "$OUTPUT_DIR/warmup-new.jsonl" 0 1
run_timed_engine "$OUTPUT_DIR/warm-new.jsonl" "$WARM_NEW_SECONDS"
curl --fail --silent --show-error --max-time 5 "${SEARCH_URL%/}/metrics" \
  > "$OUTPUT_DIR/metrics-after.json"
if ! stop_resource_workers; then
  printf 'one or more publication resource samplers failed\n' >&2
  exit 1
fi

jq -s \
  --slurpfile before "$OUTPUT_DIR/metrics-before.json" \
  --slurpfile after "$OUTPUT_DIR/metrics-after.json" '
  {
    samples:length,
    scrape_errors:([.[] | select(.ok != true)] | length),
    max_rss_bytes:([.[] | select(.ok == true) | .rss_bytes] | max // null),
    search_error_delta:(($after[0].errors // 0) - ($before[0].errors // 0))
  }
' "$OUTPUT_DIR/resources.jsonl" | tee "$OUTPUT_DIR/resource-summary.json"

if [[ -n "$MACHINE_RESOURCE_PROBE" ]]; then
  summarize_machine_resources "$OUTPUT_DIR/machine-resources.jsonl" \
    "$OUTPUT_DIR/machine-resource-summary.json"
fi
if [[ -n "$BUILDER_RESOURCE_PROBE" ]]; then
  summarize_machine_resources "$OUTPUT_DIR/builder-resources.jsonl" \
    "$OUTPUT_DIR/builder-resource-summary.json"
fi

{
  summarize_phase baseline "$OUTPUT_DIR/baseline.jsonl"
  summarize_phase build "$OUTPUT_DIR/build.jsonl"
  summarize_phase post_swap "$OUTPUT_DIR/post-swap.jsonl"
  summarize_phase warm_new "$OUTPUT_DIR/warm-new.jsonl"
} | jq -s '.' | tee "$OUTPUT_DIR/summary.json"

jq -e --argjson maximum "$MAX_ACCEPTABLE_MS" '
  all(.[];
    .samples > 0 and .errors == 0 and .max_ms != null and .max_ms <= $maximum)
' "$OUTPUT_DIR/summary.json" >/dev/null || {
  printf 'publication isolation gate failed; required zero errors and max <= %sms\n' \
    "$MAX_ACCEPTABLE_MS" >&2
  exit 1
}

jq -e --argjson maximum "$MAX_RSS_BYTES" '
  .samples > 0 and
  .scrape_errors == 0 and
  .max_rss_bytes != null and
  .max_rss_bytes <= $maximum and
  .search_error_delta == 0
' "$OUTPUT_DIR/resource-summary.json" >/dev/null || {
  printf 'publication resource gate failed; required zero scrape/search errors and peak RSS <= %s bytes\n' \
    "$MAX_RSS_BYTES" >&2
  exit 1
}

if [[ -n "$MACHINE_RESOURCE_PROBE" ]]; then
  gate_machine_resources reader "$OUTPUT_DIR/machine-resource-summary.json"
fi
if [[ -n "$BUILDER_RESOURCE_PROBE" ]]; then
  gate_machine_resources builder "$OUTPUT_DIR/builder-resource-summary.json"
fi

printf 'publication isolation gate passed: target=%s max_rss_bytes=%s results=%s\n' \
  "$target_sequence" \
  "$(jq -r '.max_rss_bytes' "$OUTPUT_DIR/resource-summary.json")" \
  "$OUTPUT_DIR"
