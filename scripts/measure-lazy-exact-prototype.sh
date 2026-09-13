#!/usr/bin/env bash
set -euo pipefail

usage() {
  printf 'usage: %s <output.jsonl> [--documents N] [--rounds N]\n' "$0" >&2
}

if (( $# == 0 )); then
  usage
  exit 2
fi

output=$1
shift
case "$output" in
  /tmp/*) ;;
  *)
    if [[ ${ALLOW_NON_TEMP_PROTOTYPE_OUTPUT:-0} != 1 ]]; then
      printf 'refusing non-temporary prototype output: %s\n' "$output" >&2
      printf 'set ALLOW_NON_TEMP_PROTOTYPE_OUTPUT=1 only for checked-in evidence\n' >&2
      exit 2
    fi
    ;;
esac

if [[ -e $output || -e $output.partial ]]; then
  printf 'refusing to overwrite prototype output: %s\n' "$output" >&2
  exit 2
fi

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
partial="$output.partial"
cd "$repo_root"

source_commit=$(git rev-parse HEAD)
if git diff --quiet --ignore-submodules -- && git diff --cached --quiet --ignore-submodules --; then
  source_dirty=false
else
  source_dirty=true
fi
rustc_version=$(rustc --version)
rustc_target=$(rustc -vV | sed -n 's/^host: //p')
environment="$(uname -srm):$(uname -m)"

# A failed run deliberately leaves *.partial behind instead of presenting an
# incomplete observation as a successful report.
YTSEARCH_PROTOTYPE_GIT_COMMIT="$source_commit" \
YTSEARCH_PROTOTYPE_GIT_DIRTY="$source_dirty" \
YTSEARCH_PROTOTYPE_RUSTC_VERSION="$rustc_version" \
YTSEARCH_PROTOTYPE_RUSTC_TARGET="$rustc_target" \
YTSEARCH_PROTOTYPE_BUILD_PROFILE=release \
YTSEARCH_PROTOTYPE_ENVIRONMENT="$environment" \
cargo run --quiet -p youtube_raw_search --release \
  --example lazy_exact_measure -- "$@" > "$partial"
mv -- "$partial" "$output"
printf 'prototype_measurement=%s\n' "$output"
