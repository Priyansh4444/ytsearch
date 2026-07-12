#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
OUT_DIR="${OUT_DIR:-$ROOT/autoresearch/runs/$(date -u +%Y%m%dT%H%M%SZ)}"
RUNS="${RUNS:-10}"
API_BASE_URL="${API_BASE_URL:-https://ytsearch-api-typesense-fly-staging.fly.dev}"

mkdir -p "$OUT_DIR"
git -C "$ROOT" rev-parse --short HEAD > "$OUT_DIR/commit.txt"
{
  printf 'timestamp=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'uname=%s\n' "$(uname -srm)"
  printf 'rustc=%s\n' "$(rustc --version)"
  printf 'bun=%s\n' "$(bun --version)"
  printf 'api=%s\n' "$API_BASE_URL"
} > "$OUT_DIR/environment.txt"

echo "phase=correctness"
(cd "$ROOT" && timeout "${TEST_TIMEOUT_SECONDS:-900}" bun run test) > "$OUT_DIR/tests.log" 2>&1

echo "phase=serialization"
(cd "$ROOT" && cargo run --release -q -p youtube_serialization_bench -- \
  "${SERIALIZATION_CHUNKS:-10000}" "${SERIALIZATION_ROUNDS:-10}") \
  > "$OUT_DIR/serialization.log" 2>&1

echo "phase=search"
API_BASE_URL="$API_BASE_URL" RUNS="$RUNS" \
  RESULTS_FILE="$OUT_DIR/search.log" \
  "$SCRIPT_DIR/benchmark-search-cases.sh" > "$OUT_DIR/search-summary.log"

printf 'out_dir=%s\n' "$OUT_DIR"
grep -E '^(json_|protobuf_)' "$OUT_DIR/serialization.log" || true
sed -n '/^case\/backend/,$p' "$OUT_DIR/search-summary.log"
