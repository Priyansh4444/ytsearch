#!/usr/bin/env bash
# Profiles one reader binary on the real index.
#
#   scripts/perf/reader-profile.sh <binary> [symbol-substring]
#
# Prints the hottest functions. With a symbol, also prints that function's
# instructions holding at least 1% of its samples, which is usually enough to
# see whether it waits on a load (cache miss), a branch, or a call. Leaves
# perf.data in the printed directory for `perf report -i` / `perf annotate -i`.
# For call stacks, build with `reader-build.sh --frame-pointers` and run
#   perf report -i <dir>/perf.data --no-children -g caller --symbol-filter=<name>
set -euo pipefail
. "$(dirname "$0")/lib.sh"

if [ $# -lt 1 ]; then
  sed -n '2,12p' "$0" >&2
  exit 2
fi
binary=$(realpath "$1") symbol=${2:-}
requests="${REQUESTS:-$PERF_DIR/reader-requests.txt}"
work="$(mktemp -d /tmp/ytsearch-perf-profile.XXXXXX)"

perf_start_reader "$binary" 18120 "$(perf_fast_cpus)" "$work/reader.log"
perf_driver "$requests" 1 18120 >/dev/null
perf record -F 1999 -g -p "${PERF_PIDS[-1]}" -o "$work/perf.data" -- \
  bash -c ". '$PERF_DIR/lib.sh'; trap - EXIT; perf_driver '$requests' 2 18120" 2>&1 | grep -v '^\[ perf record'
# Report to a file once: under pipefail, `| head` would SIGPIPE the producer
# and silently end the script.
perf report -i "$work/perf.data" --no-children --sort symbol --stdio 2>/dev/null >"$work/report.txt"
awk '/^ +[0-9.]+%/ && ++shown <= 25' "$work/report.txt" | cut -c1-160

if [ -n "$symbol" ]; then
  full=$(grep -m1 -F "$symbol" "$work/report.txt" | sed -E 's/^ +[0-9.]+% +\[\.\] +//; s/  .*$//')
  echo
  echo "== $full: instructions with >= 1% of its samples"
  perf annotate -i "$work/perf.data" --stdio -s "$full" 2>/dev/null >"$work/annotate.txt"
  awk '/^ +[0-9]+\.[0-9]+ +:/ && $1 >= 1.0' "$work/annotate.txt" | cut -c1-150
fi
echo
echo "profile: $work/perf.data"
