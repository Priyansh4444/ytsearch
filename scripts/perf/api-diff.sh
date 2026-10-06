#!/usr/bin/env bash
# Compares the API built from two git refs, response by response.
#
#   scripts/perf/api-diff.sh <base-ref> [head-ref|.]
#
# Builds youtube_api at both refs (head defaults to `.`, the working tree,
# uncommitted changes included) and runs both against one reader serving the
# full-size index. Replays every benchmark case under each sort and page (the
# set reader-capture.ts uses) plus a few other endpoints and validation errors,
# and compares status, caching headers and full JSON bodies; only timing
# fields may differ. Prints each mismatching request with its first
# difference, then `requests=N mismatches=M`. Exits non-zero on any mismatch.
#
# The reader is built from the head ref, which must read the local index (an
# old base ref may not); READER=<binary> uses that binary instead.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  sed -n '2,16p' "$0" >&2
  exit 2
fi
base_ref=$1 head_ref=${2:-.}
target="${PERF_TARGET_DIR:-/tmp/ytsearch-perf-target}"
work="$(mktemp -d /tmp/ytsearch-api-diff.XXXXXX)"
trap 'perf_cleanup; rm -rf "$work"' EXIT

# build_api <ref> <name> [-p <package>...]: builds youtube_api (and any extra
# packages) at <ref> and copies the API to $work/api-<name>.
build_api() {
  local ref=$1 name=$2
  shift 2
  perf_source "$ref" "$work/src-$name"
  cargo build --release --quiet -p youtube_api "$@" \
    --manifest-path "$PERF_SOURCE/Cargo.toml" --target-dir "$target"
  cp "$target/release/youtube-api" "$work/api-$name"
  echo "built $ref -> api-$name ($(git -C "$PERF_SOURCE" describe --always --dirty))"
}

build_api "$base_ref" base
if [ -n "${READER:-}" ]; then
  reader=$(realpath "$READER")
  build_api "$head_ref" head
else
  build_api "$head_ref" head -p youtube_raw_search
  reader=$work/reader
  cp "$target/release/youtube-raw-search" "$reader"
fi

perf_start_reader "$reader" 18140 "$(perf_fast_cpus)" "$work/reader.log"
perf_on_slow_cpus bun "$PERF_DIR/api-diff.ts" "$REPO_ROOT/scripts/benchmark-search-cases.tsv" \
  "$work/api-base" "$work/api-head" 18140 "$work"
