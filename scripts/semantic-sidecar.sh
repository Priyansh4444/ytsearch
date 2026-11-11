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

as_semantic() { gosu rawsearch nice -n 10 "$@"; }
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
  SEMANTIC_BIND="[$address]:$PORT" SEMANTIC_QDRANT_URL="http://127.0.0.1:$QDRANT_PORT" \
    as_semantic youtube-semantic serve &
  semantic=$!
  wait -n "$qdrant" "$semantic"
  log "a semantic process exited; restarting both in ${RETRY_SECONDS}s"
  kill "$qdrant" "$semantic" 2>/dev/null
  wait "$qdrant" "$semantic" 2>/dev/null
  sleep "$RETRY_SECONDS"
done
