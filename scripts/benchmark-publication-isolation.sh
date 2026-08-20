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
WARM_NEW_RUNS="${WARM_NEW_RUNS:-3}"
PUBLICATION_TIMEOUT_SECONDS="${PUBLICATION_TIMEOUT_SECONDS:-2400}"
MAX_ACCEPTABLE_MS="${MAX_ACCEPTABLE_MS:-200}"
ROUND_PAUSE_SECONDS="${ROUND_PAUSE_SECONDS:-10}"

for command in curl jq; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done
[[ "$CONCURRENCY" =~ ^[1-9][0-9]*$ ]]

mkdir -p "$OUTPUT_DIR"
: > "$OUTPUT_DIR/build.jsonl"
: > "$OUTPUT_DIR/transition.jsonl"
: > "$OUTPUT_DIR/rounds.jsonl"
active_pids=()

interrupt_load() {
  for pid in "${active_pids[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  for pid in "${active_pids[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  printf 'load stopped; the server-side publication continues independently\n' >&2
  exit 130
}
trap interrupt_load INT TERM

admin_request() {
  curl --fail --silent --show-error --connect-timeout 3 --max-time 10 \
    -H "x-raw-admin-key: $RAW_ADMIN_API_KEY" "$@"
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

publish_response="$(admin_request -X POST "${SEARCH_URL%/}/admin/publish")"
printf '%s\n' "$publish_response" > "$OUTPUT_DIR/publish.json"
target_sequence="$(printf '%s' "$publish_response" | jq -er '.last_sequence')"
scheduled="$(printf '%s' "$publish_response" | jq -r '.scheduled')"
if [[ "$scheduled" != true ]] || (( target_sequence <= initial_active )); then
  printf 'publication did not start cleanly: %s\n' "$publish_response" >&2
  exit 2
fi

deadline=$(( $(date +%s) + PUBLICATION_TIMEOUT_SECONDS ))
round=0
while (( $(date +%s) < deadline )); do
  before="$(active_sequence)"
  (( before >= target_sequence )) && break
  round=$((round + 1))
  started_ms="$(date +%s%3N)"
  pids=()
  outputs=()
  for ((worker = 1; worker <= CONCURRENCY; worker += 1)); do
    output="$OUTPUT_DIR/round-${round}-worker-${worker}.jsonl"
    outputs+=("$output")
    run_engine "$output" 1 0 &
    pids+=("$!")
  done
  active_pids=("${pids[@]}")
  for pid in "${pids[@]}"; do
    wait "$pid"
  done
  active_pids=()
  ended_ms="$(date +%s%3N)"
  after="$(active_sequence)"
  phase=build
  if (( after >= target_sequence )); then
    phase=transition
  fi
  for output in "${outputs[@]}"; do
    cat "$output" >> "$OUTPUT_DIR/$phase.jsonl"
    rm -f "$output"
  done
  jq -cn --argjson round "$round" --arg phase "$phase" \
    --argjson started_ms "$started_ms" --argjson ended_ms "$ended_ms" \
    --argjson before "$before" --argjson after "$after" \
    '{round:$round,phase:$phase,started_ms:$started_ms,ended_ms:$ended_ms,generation_before:$before,generation_after:$after}' \
    >> "$OUTPUT_DIR/rounds.jsonl"
  (( after >= target_sequence )) && break
  sleep "$ROUND_PAUSE_SECONDS"
done

final_active="$(active_sequence)"
if (( final_active < target_sequence )); then
  printf 'publication timed out: target=%s active=%s; server-side work may continue\n' \
    "$target_sequence" "$final_active" >&2
  exit 1
fi

run_engine "$OUTPUT_DIR/post-swap.jsonl" 1 0
run_engine "$OUTPUT_DIR/warmup-new.jsonl" 0 1
run_engine "$OUTPUT_DIR/warm-new.jsonl" "$WARM_NEW_RUNS" 0
curl --fail --silent --show-error --max-time 5 "${SEARCH_URL%/}/metrics" \
  > "$OUTPUT_DIR/metrics-after.json"

{
  summarize_phase baseline "$OUTPUT_DIR/baseline.jsonl"
  summarize_phase build "$OUTPUT_DIR/build.jsonl"
  summarize_phase transition "$OUTPUT_DIR/transition.jsonl"
  summarize_phase post_swap "$OUTPUT_DIR/post-swap.jsonl"
  summarize_phase warm_new "$OUTPUT_DIR/warm-new.jsonl"
} | jq -s '.' | tee "$OUTPUT_DIR/summary.json"

jq -e --argjson maximum "$MAX_ACCEPTABLE_MS" '
  all(.[] | select(.phase != "transition");
    .samples > 0 and .errors == 0 and .max_ms != null and .max_ms <= $maximum)
' "$OUTPUT_DIR/summary.json" >/dev/null || {
  printf 'publication isolation gate failed; required zero errors and max <= %sms\n' \
    "$MAX_ACCEPTABLE_MS" >&2
  exit 1
}

printf 'publication isolation gate passed: target=%s results=%s\n' \
  "$target_sequence" "$OUTPUT_DIR"
