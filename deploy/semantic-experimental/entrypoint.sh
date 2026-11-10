#!/usr/bin/env bash
# Runs Qdrant, the semantic service, youtube-api and nginx in one machine.
# Qdrant, the API or nginx exiting stops the machine so Fly restarts it whole.
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

# Keyword search comes from the production reader when this app holds its
# search-only key (RAW_SEARCH_UPSTREAM_URL and the RAW_SEARCH_API_KEY secret), so
# the toggle's two modes search the same catalog. A local Raw index, when uploaded,
# is the offline alternative with a per-boot key that never leaves the machine.
# With neither, Raw is a closed local port: lexical attempts fail fast and nginx
# retries them on production.
raw_url=http://127.0.0.1:9
raw_key="$(head -c 24 /dev/urandom | base64 | tr -d '/+=')"
if [[ -n "${RAW_SEARCH_UPSTREAM_URL:-}" && -n "${RAW_SEARCH_API_KEY:-}" ]]; then
  raw_url="$RAW_SEARCH_UPSTREAM_URL"
  raw_key="$RAW_SEARCH_API_KEY"
elif [[ -f /data/raw-index/manifest.json ]]; then
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
api_pid=$!

# Accept traffic only once semantic is ready, so the first request after an idle
# stop waits a few seconds instead of getting a silently lexical answer. Bounded,
# so a missing collection still serves lexically after 90 s.
for _ in $(seq 180); do
  curl -fsS http://127.0.0.1:8790/ready >/dev/null 2>&1 && break
  sleep 0.5
done
# The production API reaches semantic search through nginx with a shared key; the
# service itself only listens on loopback. Without a well-formed key the route does
# not exist. The key never reaches a log: nginx logs no request headers.
: >/run/semantic-service.conf
chmod 600 /run/semantic-service.conf
if [[ "${SEMANTIC_SEARCH_KEY:-}" =~ ^[A-Za-z0-9_-]{32,128}$ ]]; then
  printf '%s\n' \
    'location = /search {' \
    '    limit_except POST { deny all; }' \
    "    if (\$http_x_semantic_key != \"$SEMANTIC_SEARCH_KEY\") { return 403; }" \
    '    client_max_body_size 16k;' \
    '    proxy_pass http://127.0.0.1:8790;' \
    '}' >/run/semantic-service.conf
elif [[ -n "${SEMANTIC_SEARCH_KEY:-}" ]]; then
  echo "SEMANTIC_SEARCH_KEY is malformed; the keyed semantic route stays off" >&2
fi
nginx -g 'daemon off;' &
nginx_pid=$!

# Embedding runs on this box, beside serving, whenever a corpus is staged in
# /data/backfill. Fly's autostop must be off meanwhile (see the semantic playbook):
# traffic through its proxy does not reliably hold a machine up. When embedding ends
# the machine stops itself, so a large machine does not stay up unattended. A failed
# run leaves search serving; a restart resumes it.
if [[ -f /data/backfill/batches.json && ! -f /data/backfill/finished ]]; then
  (
    if as_app env SEMANTIC_PRUNE_SNAPSHOTS=1 \
      SEMANTIC_INDEX_THREADS="${SEMANTIC_INDEX_THREADS:-6}" \
      SEMANTIC_SNAPSHOT_ARGS="--mutations /data/backfill/mutations.sqlite3 --rename @t3dotgg=@theo" \
      bash /opt/semantic/run-semantic-backfill.sh /data/backfill /data/backfill/chunks.jsonl.gz \
      >>/data/backfill/backfill.log 2>&1; then
      touch /data/backfill/finished /tmp/backfill-just-finished
      kill "$nginx_pid"
    fi
  ) </dev/null &
fi
wait -n "$qdrant" "$api_pid" "$nginx_pid" || true
# Exit 0 is the one exit Fly's on-failure policy does not restart.
[[ -f /tmp/backfill-just-finished ]] && exit 0
exit 1
