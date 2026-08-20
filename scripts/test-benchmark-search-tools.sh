#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# shellcheck disable=SC1091
. "$SCRIPT_DIR/search-benchmark-cases.sh"
validate_search_benchmark_cases "$SCRIPT_DIR/benchmark-search-cases.tsv"

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

PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=timeout SEARCH_URL=https://fixture.invalid \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 OUTPUT="$tmp/raw-timeout.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-timeout.summary"
jq -se 'length > 0 and all(.[]; .ok == false)' "$tmp/raw-timeout.jsonl" >/dev/null
jq -e 'length > 0 and all(.[]; .errors == .runs)' "$tmp/raw-timeout.summary" >/dev/null

PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid SEARCH_URL=https://fixture.invalid \
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 OUTPUT="$tmp/raw-valid.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-valid.summary"
jq -se 'length > 0 and all(.[]; .ok == true)' "$tmp/raw-valid.jsonl" >/dev/null
jq -e 'length > 0 and all(.[]; .errors == 0)' "$tmp/raw-valid.summary" >/dev/null
initial_rows="$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')"
PATH="$tmp/bin:$PATH" MOCK_CURL_MODE=rawvalid SEARCH_URL=https://fixture.invalid \
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
  RUNS=1 WARMUP_RUNS=0 CAPTURE_RESOURCES=0 APPEND_OUTPUT=1 \
  STOP_FILE="$tmp/stop-load" OUTPUT="$tmp/raw-valid.jsonl" \
  "$SCRIPT_DIR/benchmark-search-engine.sh" > "$tmp/raw-stop.summary"
[ "$(wc -l < "$tmp/raw-valid.jsonl" | tr -d ' ')" -eq "$rows_before_stop" ] || {
  echo "stop file allowed additional benchmark requests" >&2
  exit 1
}

printf 'benchmark search tooling tests passed\n'
