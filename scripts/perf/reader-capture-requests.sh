#!/usr/bin/env bash
# Regenerates scripts/perf/reader-requests.txt, the reader requests that the
# benchmark cases produce through the real API.
#
#   scripts/perf/reader-capture-requests.sh
#
# Rerun it after changing the cases (scripts/benchmark-search-cases.tsv) or
# how youtube_search_client builds reader URLs. Builds the reader and API
# from the working tree.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

target="${PERF_TARGET_DIR:-/tmp/ytsearch-perf-target}"
cargo build --release --quiet -p youtube_raw_search -p youtube_api \
  --manifest-path "$REPO_ROOT/Cargo.toml" --target-dir "$target"
work="$(mktemp -d /tmp/ytsearch-perf-capture.XXXXXX)"
perf_start_reader "$target/release/youtube-raw-search" 18130 "$(perf_fast_cpus)" "$work/reader.log"
bun "$PERF_DIR/reader-capture.ts" "$REPO_ROOT/scripts/benchmark-search-cases.tsv" \
  "$target/release/youtube-api" 18130 "$PERF_DIR/reader-requests.txt"
rm -rf "$work"
