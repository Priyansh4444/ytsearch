#!/usr/bin/env bash
# Start a local lexical + semantic stack on loopback for evaluation:
#   Qdrant :16401, youtube-raw-search :18108, youtube-semantic :18790, youtube-api :18787
# All of mode=lexical|semantic|hybrid are then answered by http://127.0.0.1:18787.
#
# usage: scripts/run-semantic-local-stack.sh STATE_DIR RAW_INDEX_DIR
#        scripts/run-semantic-local-stack.sh stop STATE_DIR
#   STATE_DIR      holds qdrant/storage (with the imported collection), logs and a
#                  generated Raw search key
#   RAW_INDEX_DIR  a youtube-raw-search build of the same chunks
# Needs release builds and data/semantic assets (scripts/prepare-semantic-assets.py).
# Processes are detached with setsid so they outlive the shell; `stop` kills the
# recorded PIDs. Avoid `pkill -f` with these names: the pattern also matches the
# command line of the shell running it, which then kills itself (exit 144).
set -euo pipefail
if [[ "${1:-}" == stop && $# -eq 2 ]]; then
  pids=()
  for pid_file in "$2"/local-stack/*.pid; do
    [[ -f "$pid_file" ]] || continue
    pids+=("$(cat "$pid_file")")
    kill "${pids[-1]}" 2>/dev/null || true
    rm -f "$pid_file"
  done
  # Qdrant holds its WAL lock until it exits; a restart before then panics.
  for _ in $(seq 60); do
    alive=0
    for pid in "${pids[@]}"; do kill -0 "$pid" 2>/dev/null && alive=1; done
    ((alive)) || exit 0
    sleep 0.5
  done
  echo "processes still running after 30 s: ${pids[*]}" >&2
  exit 1
fi
if [[ $# -ne 2 ]]; then
  echo "usage: $0 STATE_DIR RAW_INDEX_DIR | $0 stop STATE_DIR" >&2
  exit 2
fi
root="$(cd "$(dirname "$0")/.." && pwd)"
state="$(cd "$1" && pwd)"
index="$(cd "$2" && pwd)"
bin="$root/target/release"
assets="$root/data/semantic"
logs="$state/local-stack"
mkdir -p "$logs"
# Qdrant writes .qdrant-initialized into its working directory.
cd "$state"
for binary in youtube-api youtube-semantic youtube-raw-search; do
  [[ -x "$bin/$binary" ]] || { echo "missing $bin/$binary: cargo build --release" >&2; exit 1; }
done
[[ -s "$logs/raw-key" ]] || (umask 077 && head -c 24 /dev/urandom | base64 | tr -d '/+=' > "$logs/raw-key")

wait_for() {
  for _ in $(seq 120); do
    curl -fsS "$1" >/dev/null 2>&1 && return 0
    sleep 0.5
  done
  echo "timed out waiting for $1 (see $logs)" >&2
  exit 1
}
detach() {
  local name="$1"
  shift
  # setsid may fork, so $! can be a parent that exits at once; the process
  # records its own PID before exec instead.
  setsid bash -c 'echo "$$" >"$1"; shift; exec "$@"' _ "$logs/$name.pid" "$@" \
    >"$logs/$name.log" 2>&1 < /dev/null &
}

QDRANT__SERVICE__HTTP_PORT=16401 QDRANT__SERVICE__GRPC_PORT=16402 \
  QDRANT__STORAGE__STORAGE_PATH="$state/qdrant/storage" \
  QDRANT__STORAGE__SNAPSHOTS_PATH="$state/qdrant/snapshots" QDRANT__TELEMETRY_DISABLED=true \
  detach qdrant "$assets/qdrant/qdrant" --config-path "$root/config/qdrant-semantic.local.yaml"
RAW_SEARCH_BIND=127.0.0.1:18108 RAW_SEARCH_API_KEY="$(cat "$logs/raw-key")" \
  detach raw "$bin/youtube-raw-search" serve "$index"
wait_for http://127.0.0.1:16401/healthz

SEMANTIC_MODEL_DIR="$assets/model" \
  ORT_DYLIB_PATH="$assets/runtime/onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0" \
  SEMANTIC_QDRANT_URL=http://127.0.0.1:16401 \
  SEMANTIC_COLLECTION="${SEMANTIC_COLLECTION:-ytsearch_semantic_pilot_experimental_128}" \
  SEMANTIC_DIMENSIONS=128 SEMANTIC_BIND=127.0.0.1:18790 \
  detach semantic "$bin/youtube-semantic" serve
RAW_SEARCH_API_URL=http://127.0.0.1:18108 RAW_SEARCH_API_KEY="$(cat "$logs/raw-key")" \
  SEMANTIC_SEARCH_URL=http://127.0.0.1:18790 YTSEARCH_API_ADDR=127.0.0.1:18787 \
  YTSEARCH_INDEX_QUEUE_PATH="$logs/queue.jsonl" YTSEARCH_ENGAGEMENT_SORTS=true \
  YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER=true YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE=100000 \
  detach api "$bin/youtube-api"
wait_for http://127.0.0.1:18790/ready
wait_for http://127.0.0.1:18787/api/health
echo "local stack ready: http://127.0.0.1:18787/api/search (logs in $logs)"
