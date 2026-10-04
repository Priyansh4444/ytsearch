#!/usr/bin/env bash
# Builds the reader with symbols for profiling and A/B comparison.
#
#   scripts/perf/reader-build.sh [--frame-pointers] <git-ref|.> <output-binary>
#
# `.` builds the working tree, uncommitted changes included. Any other ref is
# built from a temporary detached worktree, so comparing against `master` or
# an older commit never touches your checkout. --frame-pointers makes
# `perf record --call-graph fp` stacks reliable; DWARF unwinding stops at
# inlined leaf functions in this crate.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

rustflags="${RUSTFLAGS:-}"
target="${PERF_TARGET_DIR:-/tmp/ytsearch-perf-target}"
if [ "${1:-}" = "--frame-pointers" ]; then
  rustflags="$rustflags -C force-frame-pointers=yes"
  target="$target-fp"
  shift
fi
if [ $# -ne 2 ]; then
  sed -n '2,10p' "$0" >&2
  exit 2
fi
ref=$1 output=$2

source_dir=$REPO_ROOT
if [ "$ref" != "." ]; then
  source_dir="$(mktemp -d /tmp/ytsearch-perf-src.XXXXXX)"
  git -C "$REPO_ROOT" worktree add --quiet --detach "$source_dir" "$ref"
  trap 'git -C "$REPO_ROOT" worktree remove --force "$source_dir"; perf_cleanup' EXIT
fi

# Release codegen with line tables and no stripping, so perf annotate can map
# instructions back to source lines.
RUSTFLAGS="$rustflags" CARGO_PROFILE_RELEASE_STRIP=false CARGO_PROFILE_RELEASE_DEBUG=line-tables-only \
  cargo build --release --quiet -p youtube_raw_search \
  --manifest-path "$source_dir/Cargo.toml" --target-dir "$target"
cp "$target/release/youtube-raw-search" "$output"
echo "built $ref -> $output ($(git -C "$source_dir" describe --always --dirty))"
