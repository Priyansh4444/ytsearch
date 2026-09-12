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

# A failed run deliberately leaves *.partial behind instead of presenting an
# incomplete observation as a successful report.
cargo run --quiet -p youtube_raw_search --release \
  --example lazy_exact_measure -- "$@" > "$partial"
mv -- "$partial" "$output"
printf 'prototype_measurement=%s\n' "$output"
