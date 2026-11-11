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
echo $$ >"$SIDECAR_LOG/qdrant.pid"
exec sleep 30
SH
cat >"$work/bin/youtube-semantic" <<'SH'
#!/bin/sh
echo "semantic stderr reaches the log" >&2
printf 'args=%s\nbind=%s\nqdrant=%s\nkey=%s\noom=%s\n' "$*" "$SEMANTIC_BIND" \
  "$SEMANTIC_QDRANT_URL" "$SEMANTIC_SEARCH_KEY" "$(cat /proc/self/oom_score_adj)" \
  >"$SIDECAR_LOG/semantic"
echo $$ >"$SIDECAR_LOG/semantic.pid"
exec python3 -I "$SIDECAR_LOG/fake-semantic.py"
SH
cat >"$work/fake-semantic.py" <<'PY'
import http.server, json, os, socket
log = os.path.join(os.environ["SIDECAR_LOG"], "warm")
class Handler(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with open(log, "a") as out:
            out.write(f"{self.path} {self.headers.get('x-semantic-key')} {body['q']}\n")
        self.send_response(200); self.end_headers(); self.wfile.write(b"{}")
    def log_message(self, *args): pass
class Server(http.server.HTTPServer):
    address_family = socket.AF_INET6
host, port = os.environ["SEMANTIC_BIND"].rsplit(":", 1)
Server((host.strip("[]"), int(port)), Handler).serve_forever()
PY
chmod +x "$work/bin/"*

free_port() { python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])'; }
semantic_port="$(python3 -c 'import socket; s=socket.socket(socket.AF_INET6); s.bind(("::1",0)); print(s.getsockname()[1])')"
key=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
run() {
  PATH="$work/bin:$PATH" SIDECAR_LOG="$work" SEMANTIC_ROOT="$work/root" \
    SEMANTIC_COLLECTION=demo SEMANTIC_ADDRESS=::1 SEMANTIC_RETRY_SECONDS=30 \
    SEMANTIC_WARM_SECONDS=1 SEMANTIC_PORT="$semantic_port" \
    SEMANTIC_QDRANT_BIN="$work/bin/fake-qdrant" "$@"
}
# In the background, exec so that $! is the sidecar itself and cleanup's TERM
# reaches its trap, which stops Qdrant, the service and the warm-up loop.
run_exec() { run exec "$@"; }

# Each guard leaves semantic search off and exits cleanly.
for case in "SEMANTIC_SEARCH_KEY=short" "SEMANTIC_SEARCH_KEY=$key SEMANTIC_COLLECTION=absent" \
  "SEMANTIC_SEARCH_KEY="; do
  # shellcheck disable=SC2086
  run env $case timeout 5 "$SCRIPT_DIR/semantic-sidecar.sh" 2>/dev/null
  [[ ! -e "$work/qdrant" && ! -e "$work/semantic" ]]
done

reader_port="$(free_port)"
qdrant_port="$(free_port)"
run_exec env SEMANTIC_SEARCH_KEY="$key" SEMANTIC_READER_PORT="$reader_port" \
  SEMANTIC_QDRANT_PORT="$qdrant_port" "$SCRIPT_DIR/semantic-sidecar.sh" 2>"$work/stderr" &
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
grep -Fx "bind=[::1]:$semantic_port" "$work/semantic" >/dev/null
grep -Fx "qdrant=http://127.0.0.1:$qdrant_port" "$work/semantic" >/dev/null
grep -Fx "key=$key" "$work/semantic" >/dev/null
grep -Fx 'oom=1000' "$work/semantic" >/dev/null
# Warm-up searches reach the service with the key, rotating queries.
for _ in $(seq 50); do [[ "$(wc -l 2>/dev/null <"$work/warm" || echo 0)" -ge 2 ]] && break; sleep 0.2; done
[[ "$(grep -c "^/search $key " "$work/warm")" -ge 2 ]]
[[ "$(cut -d' ' -f3- "$work/warm" | sort -u | wc -l)" -ge 2 ]]
# Probing must not swallow the sidecar's or its children's diagnostics.
grep -F 'reader healthy; starting semantic search' "$work/stderr" >/dev/null
grep -Fx 'semantic stderr reaches the log' "$work/stderr" >/dev/null
# (`! cmd` never trips set -e, so negative checks fail explicitly.)
if grep -F 'Connection refused' "$work/stderr" >/dev/null; then
  printf 'probe errors leaked into the log\n' >&2; exit 1
fi

# TERM stops everything the sidecar started, so a restart never leaves an old
# Qdrant holding its port.
children=("$(cat "$work/qdrant.pid")" "$(cat "$work/semantic.pid")")
kill "${pids[0]}"
for _ in $(seq 50); do
  kill -0 "${children[@]}" 2>/dev/null || break
  sleep 0.1
done
for child in "${children[@]}"; do
  if kill -0 "$child" 2>/dev/null; then
    printf 'TERM left child %s running\n' "$child" >&2; exit 1
  fi
done

printf 'semantic_sidecar=ok\n'
