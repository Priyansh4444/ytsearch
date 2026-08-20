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

printf 'benchmark search tooling tests passed\n'
