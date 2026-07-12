#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

cat > "$tmp/ok.json" <<'JSON'
{"frontend_http":200,"engine_http":200,"search":{"http":200},"indexing":{"status":"running","handle":"@test","indexed":2,"public":3},"worker":{"state":"started"}}
JSON
SNAPSHOT_FILE="$tmp/ok.json" "$SCRIPT_DIR/doctor.sh" | grep -F 'OK serving; indexing @test 2/3' >/dev/null

cat > "$tmp/engine.json" <<'JSON'
{"frontend_http":200,"engine_http":503,"search":{"http":200},"indexing":{"status":"idle"},"worker":{"state":"started"}}
JSON
if SNAPSHOT_FILE="$tmp/engine.json" "$SCRIPT_DIR/doctor.sh" > "$tmp/output"; then
  echo "doctor should fail for an unhealthy engine" >&2
  exit 1
fi
grep -F 'FAIL Typesense health HTTP 503' "$tmp/output" >/dev/null
grep -F 'NEXT run:' "$tmp/output" >/dev/null

cat > "$tmp/worker.json" <<'JSON'
{"frontend_http":200,"engine_http":200,"search":{"http":200},"indexing":{"status":"queued","handle":"@test"},"worker":null}
JSON
if SNAPSHOT_FILE="$tmp/worker.json" "$SCRIPT_DIR/doctor.sh" > "$tmp/output"; then
  echo "doctor should fail when the queue worker is missing" >&2
  exit 1
fi
grep -F 'FAIL queue worker state missing' "$tmp/output" >/dev/null

echo "doctor_tests=ok"
