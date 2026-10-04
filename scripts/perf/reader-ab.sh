#!/usr/bin/env bash
# Compares two reader binaries on the real index, request by request.
#
#   scripts/perf/reader-ab.sh <binary-A> <binary-B> [runs]
#
# Both readers are pinned to the same fast cores and served the committed
# requests (reader-requests.txt) in interleaved order. Prints three lines:
#   wall:     per-request median latency ratios (B/A; < 1 means B is faster)
#   parity:   requests whose responses differ (must be 0 for a pure perf change)
#   counters: CPU time, cycles and instructions per reader, if perf is usable
# Exits non-zero on any parity mismatch.
#
# Wall time on a laptop drifts by up to 2-3x with power state; cycles and
# instructions do not. Run the same binary as both A and B first: the ratios
# it prints are the noise floor any real change has to clear.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

if [ $# -lt 2 ]; then
  sed -n '2,15p' "$0" >&2
  exit 2
fi
binary_a=$(realpath "$1") binary_b=$(realpath "$2") runs=${3:-5}
requests="${REQUESTS:-$PERF_DIR/reader-requests.txt}"
cpus="$(perf_fast_cpus)"
work="$(mktemp -d /tmp/ytsearch-perf-ab.XXXXXX)"

perf_start_reader "$binary_a" 18120 "$cpus" "$work/a.log"
pid_a=${PERF_PIDS[-1]}
perf_start_reader "$binary_b" 18122 "$cpus" "$work/b.log"
pid_b=${PERF_PIDS[-1]}

events=task-clock,cycles,instructions
if ! command -v perf >/dev/null || ! perf stat -e "$events" -x, true >/dev/null 2>&1; then
  perf_driver "$requests" "$runs" 18120 18122
  echo "counters: unavailable (install perf or lower kernel.perf_event_paranoid)"
  exit
fi
# Nested so both counters stop exactly when the driver exits; each perf
# counts only its own reader.
status=0
perf stat -x, -e "$events" -p "$pid_a" -o "$work/a.stat" -- \
  perf stat -x, -e "$events" -p "$pid_b" -o "$work/b.stat" -- \
  bash -c ". '$PERF_DIR/lib.sh'; trap - EXIT; perf_driver '$requests' '$runs' 18120 18122" || status=$?

# Hybrid CPUs report cycles/instructions per core type (cpu_core/..., cpu_atom/...);
# sum them per event.
sum_event() {
  awk -F, -v event="$1" '$1 ~ /^[0-9.]+$/ && $3 ~ event { total += $1 } END { printf "%.0f", total }' "$2"
}
ratio() { awk -v a="$1" -v b="$2" 'BEGIN { if (a > 0) printf "%.3f", b / a; else printf "n/a" }'; }
for event in task-clock cycles instructions; do
  a=$(sum_event "$event" "$work/a.stat") b=$(sum_event "$event" "$work/b.stat")
  printf '%s %s=%s/%s ratio=%s' "${sep:-counters:}" "$event" "$a" "$b" "$(ratio "$a" "$b")"
  sep=" |"
done
echo
multiplexed=$(awk -F, '$1 ~ /^[0-9.]+$/ && $5 != "" && $5 + 0 < 100 { print "yes"; exit }' "$work/a.stat" "$work/b.stat")
[ -z "$multiplexed" ] || echo "warning: counters were multiplexed; their ratios are estimates"
rm -rf "$work"
exit "$status"
