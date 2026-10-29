#!/usr/bin/env bash
# Runs Qdrant, the semantic service, youtube-api and nginx in one machine.
# Any process exiting stops the machine so Fly restarts it whole.
set -euo pipefail

mkdir -p /data/qdrant/storage /data/qdrant/snapshots
chown -R semantic:semantic /data
as_app() { setpriv --reuid=semantic --regid=semantic --init-groups "$@"; }

QDRANT__STORAGE__STORAGE_PATH=/data/qdrant/storage \
  QDRANT__STORAGE__SNAPSHOTS_PATH=/data/qdrant/snapshots \
  QDRANT__SERVICE__HTTP_PORT=6333 QDRANT__SERVICE__GRPC_PORT=6334 \
  as_app /opt/semantic/qdrant/qdrant --config-path /opt/semantic/qdrant.yaml &
qdrant=$!
until curl -fsS http://127.0.0.1:6333/healthz >/dev/null 2>&1; do
  kill -0 "$qdrant" 2>/dev/null || { echo "qdrant exited during startup" >&2; exit 1; }
  sleep 0.5
done

# Serving needs an imported collection. Until one is recovered, retry quietly;
# the API answers lexically (through nginx) in the meantime.
(
  until as_app youtube-semantic serve; do
    echo "semantic service not ready (collection missing?); retrying in 30s" >&2
    sleep 30
  done
) &

# A Raw index of the same videos, when uploaded, answers lexical and hybrid
# search here. Its key is generated per boot and never leaves the machine.
# Without an index, Raw is a closed local port: lexical attempts fail fast and
# nginx retries them on production.
raw_url=http://127.0.0.1:9
raw_key="$(head -c 24 /dev/urandom | base64 | tr -d '/+=')"
if [[ -f /data/raw-index/manifest.json ]]; then
  RAW_SEARCH_BIND=127.0.0.1:8108 RAW_SEARCH_API_KEY="$raw_key" \
    as_app youtube-raw-search serve /data/raw-index &
  raw_url=http://127.0.0.1:8108
fi

YTSEARCH_API_ADDR=127.0.0.1:8787 \
  RAW_SEARCH_API_URL="$raw_url" \
  RAW_SEARCH_API_KEY="$raw_key" \
  YTSEARCH_PRESERVE_BACKEND_RELEVANCE_ORDER=true \
  SEMANTIC_SEARCH_URL=http://127.0.0.1:8790 \
  YTSEARCH_INDEX_QUEUE_PATH=/data/index-requests-unused.jsonl \
  YTSEARCH_ENGAGEMENT_SORTS=true \
  as_app youtube-api &

# Accept traffic only once semantic is ready, so the first request after an idle
# stop waits a few seconds instead of getting a silently lexical answer. Bounded,
# so a missing collection still serves lexically after 90 s.
for _ in $(seq 180); do
  curl -fsS http://127.0.0.1:8790/ready >/dev/null 2>&1 && break
  sleep 0.5
done
nginx -g 'daemon off;' &
wait -n
exit 1
