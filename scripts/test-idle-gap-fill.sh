#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
test_dir="$(mktemp -d)"
trap 'rm -rf "$test_dir"' EXIT
mkdir -p "$test_dir/api" "$test_dir/data"

CLOUD_INDEX_QUEUE_LIBRARY_ONLY=1
INDEX_QUEUE_API_BASE_URL="file://$test_dir"
IDLE_GAP_FILL_STATE_FILE="$test_dir/data/cursor"
TRANSCRIPT_CACHE_DIR="$test_dir/data/transcript-cache"
export CLOUD_INDEX_QUEUE_LIBRARY_ONLY INDEX_QUEUE_API_BASE_URL IDLE_GAP_FILL_STATE_FILE TRANSCRIPT_CACHE_DIR
. "$SCRIPT_DIR/cloud-index-queue-entrypoint.sh"

cat > "$test_dir/api/youtubers" <<'JSON'
{"channels":[
  {"handle":"@complete","public_video_count":10,"indexed_video_count":10},
  {"handle":"@bravo","public_video_count":12,"indexed_video_count":10},
  {"handle":"@alpha","public_video_count":9,"indexed_video_count":3}
]}
JSON

first="$(pull_idle_gap_handle)"
[ "$first" = "@alpha" ] || {
  echo "expected @alpha first, got: $first" >&2
  exit 1
}

remember_idle_gap_handle "$first"
second="$(pull_idle_gap_handle)"
[ "$second" = "@bravo" ] || {
  echo "expected @bravo second, got: $second" >&2
  exit 1
}

remember_idle_gap_handle "$second"
wrapped="$(pull_idle_gap_handle)"
[ "$wrapped" = "@alpha" ] || {
  echo "expected rotation to wrap to @alpha, got: $wrapped" >&2
  exit 1
}

echo "idle gap fill selection passed"
