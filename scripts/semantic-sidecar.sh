#!/usr/bin/env bash
# Hybrid search beside the Raw reader: Qdrant on loopback and youtube-semantic on
# this Machine's private-network (6PN) address, where the production API reaches
# it with SEMANTIC_SEARCH_KEY. youtube-semantic checks that key itself.
#
# The reader comes first. Both processes take the highest OOM score and a lower
# CPU priority, so memory pressure or a slow embedding stops semantic search
# (the API answers with keyword results) and never the reader. Nothing starts
# until the reader is healthy, so the two never compete during a cold start.
# A crash restarts both after a pause; a missing collection or key leaves
# semantic search off and the reader untouched.
set -uo pipefail

ROOT="${SEMANTIC_ROOT:-/data/mutable/semantic}"
READER_PORT="${SEMANTIC_READER_PORT:-8108}"
PORT="${SEMANTIC_PORT:-8790}"
QDRANT_PORT="${SEMANTIC_QDRANT_PORT:-6333}"
RETRY_SECONDS="${SEMANTIC_RETRY_SECONDS:-60}"
WARM_SECONDS="${SEMANTIC_WARM_SECONDS:-60}"
PREFETCH_SECONDS="${SEMANTIC_PREFETCH_SECONDS:-600}"
log() { printf 'semantic-sidecar: %s\n' "$*" >&2; }

# Exit status 0 when http://HOST:PORT/PATH answers 200. The reader image has
# no curl; bash's /dev/tcp is enough for loopback and 6PN probes.
http_ok() {
  local host="$1" port="$2" path="$3" status
  # Braces scope the redirection: on a bare `exec`, 2>/dev/null would silence
  # this script's stderr, and every child's, for good.
  { exec 3<>"/dev/tcp/$host/$port"; } 2>/dev/null || return 1
  printf 'GET %s HTTP/1.0\r\nHost: localhost\r\n\r\n' "$path" >&3
  read -r -t 5 _ status _ <&3
  exec 3>&- 3<&-
  [[ "$status" == 200 ]]
}

# The reader's daily publication fills the page cache, and the kernel then
# swaps out idle semantic memory and drops Qdrant's vector pages: the next
# hybrid searches took 3 to 6 s on 2026-11-11, near the API's 6 s cap. One
# search a minute, straight to youtube-semantic (no analytics), keeps the
# model, graph and vectors recently used. It costs about 50 ms of CPU.
WARM_QUERIES=("how does this work" "best way to learn programming"
  "why is the database slow" "react server components" "rust memory safety"
  "how to start a startup" "what is machine learning" "deploying to production")
# Qdrant keeps vectors and its graph on disk, so a query it has not seen walks
# uncached pages: 220 to 410 ms in the service on 2026-11-17, and 90 to 140 ms
# once these files were cached. Read them (about 560 MB) at start and every
# PREFETCH_SECONDS so a publication's page-cache churn cannot leave them cold.
# Passage text (payload_storage, 1.5 GB) stays on disk; only 100 hits read it.
prefetch_index() {
  local bytes low=()
  command -v ionice >/dev/null 2>&1 && low=(ionice -c3)
  bytes="$(find "$ROOT/qdrant/storage/collections/$SEMANTIC_COLLECTION" -type f \
    \( -path '*/vector_storage/*' -o -path '*/vector_index/*' -o -path '*/payload_index/*' \
       -o -name '*id_tracker*' \) -print0 \
    | xargs -0 -r "${low[@]}" nice -n 19 cat 2>/dev/null | wc -c)"
  [[ "${1:-}" == quiet ]] || log "prefetched $((bytes / 1048576)) MB of Qdrant's vector index"
}
keep_warm() {
  local i=0 body since_prefetch=0
  while sleep "$WARM_SECONDS"; do
    since_prefetch=$((since_prefetch + WARM_SECONDS))
    if (( since_prefetch >= PREFETCH_SECONDS )); then
      prefetch_index quiet
      since_prefetch=0
    fi
    body="{\"q\":\"${WARM_QUERIES[i++ % ${#WARM_QUERIES[@]}]}\",\"limit\":20}"
    { exec 3<>"/dev/tcp/$address/$PORT"; } 2>/dev/null || continue
    printf 'POST /search HTTP/1.0\r\nHost: localhost\r\nContent-Type: application/json\r\nx-semantic-key: %s\r\nContent-Length: %s\r\n\r\n%s' \
      "$SEMANTIC_SEARCH_KEY" "${#body}" "$body" >&3
    timeout 30 cat <&3 >/dev/null
    exec 3>&- 3<&-
  done
}

if [[ ! -d "$ROOT/qdrant/storage/collections/$SEMANTIC_COLLECTION" ]]; then
  log "no collection under $ROOT; semantic search stays off"
  exit 0
fi
if [[ ! "${SEMANTIC_SEARCH_KEY:-}" =~ ^[A-Za-z0-9_-]{32,128}$ ]]; then
  log "SEMANTIC_SEARCH_KEY is missing or malformed; semantic search stays off"
  exit 0
fi
address="${SEMANTIC_ADDRESS:-$(getent ahostsv6 fly-local-6pn 2>/dev/null | awk 'NR == 1 { print $1 }')}"
if [[ -z "$address" ]]; then
  log "no fly-local-6pn address; semantic search stays off"
  exit 0
fi

# Inherited by every process started below; raising it needs no privilege.
echo 1000 >/proc/self/oom_score_adj
until http_ok 127.0.0.1 "$READER_PORT" /health; do sleep 5; done
log "reader healthy; starting semantic search on [$address]:$PORT"

# Only ever run in the background. exec states what bash already does for the
# last command of a background function: $! is the service itself (gosu and
# nice exec too), so a restart or TERM stops the process, not a wrapper shell.
as_semantic() { exec gosu rawsearch nice -n 10 "$@"; }
qdrant='' semantic='' warm=''
# shellcheck disable=SC2086 # unset PIDs must vanish from the list
trap 'kill $qdrant $semantic $warm 2>/dev/null; exit 0' TERM INT
while :; do
  as_semantic "${SEMANTIC_QDRANT_BIN:-/opt/semantic/qdrant/qdrant}" \
    --config-path "${SEMANTIC_QDRANT_CONFIG:-/opt/semantic/qdrant.yaml}" \
    >/dev/null &
  qdrant=$!
  for _ in $(seq 240); do
    http_ok 127.0.0.1 "$QDRANT_PORT" /healthz && break
    kill -0 "$qdrant" 2>/dev/null || break
    sleep 0.5
  done
  prefetch_index
  SEMANTIC_BIND="[$address]:$PORT" SEMANTIC_QDRANT_URL="http://127.0.0.1:$QDRANT_PORT" \
    as_semantic youtube-semantic serve &
  semantic=$!
  keep_warm &
  warm=$!
  wait -n "$qdrant" "$semantic"
  log "a semantic process exited; restarting both in ${RETRY_SECONDS}s"
  kill "$qdrant" "$semantic" "$warm" 2>/dev/null
  wait "$qdrant" "$semantic" "$warm" 2>/dev/null
  sleep "$RETRY_SECONDS"
done
