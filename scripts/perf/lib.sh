# Shared helpers for scripts/perf/*.sh. Source it; do not run it.
#
# Readers started here run in the background of the calling script and are
# killed by its EXIT trap, so an interrupted benchmark leaves nothing behind.

PERF_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$PERF_DIR/../.." && pwd)"
PERF_KEY=perf-local
PERF_PIDS=()

# The full-size schema-9 index. It lives in the main checkout, which linked
# worktrees reach through git's common directory.
perf_index() {
  if [ -n "${RAW_INDEX:-}" ]; then
    printf '%s\n' "$RAW_INDEX"
    return
  fi
  local common
  common="$(git -C "$REPO_ROOT" rev-parse --path-format=absolute --git-common-dir)"
  printf '%s\n' "$(dirname "$common")/.search-experiment/raw-index-schema9-accel-test"
}

# CPUs with the highest maximum frequency (the P-cores on a hybrid CPU).
# Pinning both sides of a comparison to these removes P/E-core migration,
# the largest source of noise on a laptop.
perf_fast_cpus() {
  local cpus
  cpus=$(lscpu -e=CPU,MAXMHZ 2>/dev/null | awk '
    NR > 1 && $2 ~ /^[0-9.]+$/ { mhz[$1] = $2 + 0; if ($2 + 0 > top) top = $2 + 0 }
    END { sep = ""; for (c in mhz) if (mhz[c] == top) { printf "%s%s", sep, c; sep = "," } print "" }')
  # No frequency column (common in VMs): use every CPU.
  printf '%s\n' "${cpus:-0-$(($(nproc) - 1))}"
}

# CPUs with the lowest maximum frequency, for the load driver, so it does
# not compete with the readers. Empty on a CPU with uniform cores.
perf_slow_cpus() {
  lscpu -e=CPU,MAXMHZ 2>/dev/null | awk '
    NR > 1 && $2 ~ /^[0-9.]+$/ {
      mhz[$1] = $2 + 0; if (low == "" || $2 + 0 < low) low = $2 + 0; if ($2 + 0 > top) top = $2 + 0
    }
    END { if (low == "" || low == top) exit; sep = ""; for (c in mhz) if (mhz[c] == low) { printf "%s%s", sep, c; sep = "," } print "" }'
}

perf_cleanup() {
  local pid
  for pid in "${PERF_PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  wait 2>/dev/null || true
}
trap perf_cleanup EXIT

# perf_start_reader <binary> <port> <cpus> <log>: serves the index on
# [::1]:<port> and returns once the generation is warm.
perf_start_reader() {
  local binary=$1 port=$2 cpus=$3 log=$4 index
  index="$(perf_index)"
  if [ ! -f "$index/manifest.json" ]; then
    echo "No index at $index; set RAW_INDEX to a schema-9 generation." >&2
    exit 1
  fi
  : >"$log"
  RAW_SEARCH_BIND="[::1]:$port" RAW_SEARCH_API_KEY="$PERF_KEY" \
    taskset -c "$cpus" "$binary" serve "$index" >"$log" 2>&1 &
  PERF_PIDS+=("$!")
  local started=$SECONDS
  until grep -q 'warmed raw generation' "$log"; do
    if ! kill -0 "${PERF_PIDS[-1]}" 2>/dev/null || [ $((SECONDS - started)) -gt 300 ]; then
      echo "Reader $binary failed to warm; see $log" >&2
      tail -5 "$log" >&2
      exit 1
    fi
    sleep 0.3
  done
}

# perf_driver <args...>: runs the Bun load driver on the slow CPUs when the
# CPU is hybrid, otherwise unpinned.
perf_driver() {
  local cpus
  cpus="$(perf_slow_cpus)"
  if [ -n "$cpus" ]; then
    taskset -c "$cpus" bun "$PERF_DIR/reader-drive.ts" "$@"
  else
    bun "$PERF_DIR/reader-drive.ts" "$@"
  fi
}
