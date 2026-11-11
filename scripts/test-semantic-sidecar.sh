#!/usr/bin/env bash
# The reader-hosted semantic sidecar: it starts nothing without a collection, a
# well-formed key and an address; it waits for the reader before starting; and
# it starts Qdrant and youtube-semantic with the 6PN bind, the key and the
# lowest OOM priority.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
work="$(mktemp -d)"
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
mkdir -p "$work/bin" "$work/root/qdrant/storage/collections/demo" "$work/web"
: >"$work/web/health"
: >"$work/web/healthz"

cat >"$work/bin/gosu" <<'SH'
#!/bin/sh
shift
exec "$@"
SH
cat >"$work/bin/fake-qdrant" <<'SH'
#!/bin/sh
echo started >"$SIDECAR_LOG/qdrant"
exec sleep 30
SH
cat >"$work/bin/youtube-semantic" <<'SH'
#!/bin/sh
printf 'args=%s\nbind=%s\nqdrant=%s\nkey=%s\noom=%s\n' "$*" "$SEMANTIC_BIND" \
  "$SEMANTIC_QDRANT_URL" "$SEMANTIC_SEARCH_KEY" "$(cat /proc/self/oom_score_adj)" \
  >"$SIDECAR_LOG/semantic"
exit 1
SH
chmod +x "$work/bin/"*

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
key=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
run() {
  PATH="$work/bin:$PATH" SIDECAR_LOG="$work" SEMANTIC_ROOT="$work/root" \
    SEMANTIC_COLLECTION=demo SEMANTIC_ADDRESS=::1 SEMANTIC_RETRY_SECONDS=30 \
    SEMANTIC_QDRANT_BIN="$work/bin/fake-qdrant" "$@"
}

# Each guard leaves semantic search off and exits cleanly.
for case in "SEMANTIC_SEARCH_KEY=short" "SEMANTIC_SEARCH_KEY=$key SEMANTIC_COLLECTION=absent" \
  "SEMANTIC_SEARCH_KEY="; do
  # shellcheck disable=SC2086
  run env $case timeout 5 "$SCRIPT_DIR/semantic-sidecar.sh" 2>/dev/null
  [[ ! -e "$work/qdrant" && ! -e "$work/semantic" ]]
done

reader_port="$(free_port)"
qdrant_port="$(free_port)"
run env SEMANTIC_SEARCH_KEY="$key" SEMANTIC_READER_PORT="$reader_port" \
  SEMANTIC_QDRANT_PORT="$qdrant_port" "$SCRIPT_DIR/semantic-sidecar.sh" 2>/dev/null &
pids+=("$!")
sleep 2
# Nothing starts while the reader is unhealthy.
[[ ! -e "$work/qdrant" ]]
python3 -m http.server --bind 127.0.0.1 --directory "$work/web" "$reader_port" >/dev/null 2>&1 &
pids+=("$!")
python3 -m http.server --bind 127.0.0.1 --directory "$work/web" "$qdrant_port" >/dev/null 2>&1 &
pids+=("$!")
for _ in $(seq 100); do [[ -s "$work/semantic" ]] && break; sleep 0.2; done
grep -Fx 'started' "$work/qdrant" >/dev/null
grep -Fx 'args=serve' "$work/semantic" >/dev/null
grep -Fx 'bind=[::1]:8790' "$work/semantic" >/dev/null
grep -Fx "qdrant=http://127.0.0.1:$qdrant_port" "$work/semantic" >/dev/null
grep -Fx "key=$key" "$work/semantic" >/dev/null
grep -Fx 'oom=1000' "$work/semantic" >/dev/null

printf 'semantic_sidecar=ok\n'
