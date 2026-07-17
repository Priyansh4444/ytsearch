#!/usr/bin/env bash
set -euo pipefail

# Runs an isolated Qdrant container and removes it on exit. No staging data or
# services are touched. Override the VECTOR_* variables for a larger run.
QDRANT_IMAGE="${QDRANT_IMAGE:-qdrant/qdrant:v1.15.4}"
CONTAINER_NAME="ytsearch-qdrant-bench-$$"
PORT="${QDRANT_BENCH_PORT:-6334}"

cleanup() {
  docker rm -f "$CONTAINER_NAME" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker run --rm -d --name "$CONTAINER_NAME" -p "127.0.0.1:$PORT:6333" "$QDRANT_IMAGE" >/dev/null
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:$PORT/readyz" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
curl -fsS "http://127.0.0.1:$PORT/readyz" >/dev/null

QDRANT_URL="http://127.0.0.1:$PORT" \
  cargo run --release -p youtube_semantic --example qdrant_bench

docker stats --no-stream --format 'container={{.Name}} memory={{.MemUsage}} cpu={{.CPUPerc}}' "$CONTAINER_NAME"
