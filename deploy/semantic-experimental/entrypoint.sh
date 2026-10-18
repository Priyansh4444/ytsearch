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

# Lexical search is answered by production via nginx, so this API's Raw backend
# is a closed local port: any lexical attempt fails fast and nginx retries it.
YTSEARCH_API_ADDR=127.0.0.1:8787 \
  RAW_SEARCH_API_URL=http://127.0.0.1:9 \
  RAW_SEARCH_API_KEY=unused-lexical-answers-come-from-production \
  SEMANTIC_SEARCH_URL=http://127.0.0.1:8790 \
  YTSEARCH_INDEX_QUEUE_PATH=/data/index-requests-unused.jsonl \
  YTSEARCH_ENGAGEMENT_SORTS=true \
  as_app youtube-api &

nginx -g 'daemon off;' &
wait -n
exit 1
