#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

cat > "$tmp/ok.json" <<'JSON'
{"frontend_http":200,"engine_http":200,"search":{"http":200},"indexing":{"status":"running","handle":"@test","indexed":2,"public":3},"worker":{"state":"started"}}
JSON
SNAPSHOT_FILE="$tmp/ok.json" DOCTOR_STATE_FILE="$tmp/state.json" "$SCRIPT_DIR/doctor.sh" | grep -F 'OK serving; indexing @test 2/3' >/dev/null

cat > "$tmp/idle.json" <<'JSON'
{"frontend_http":200,"engine_http":200,"search":{"http":200},"indexing":{"status":"idle"},"worker":{"state":"stopped"}}
JSON
SNAPSHOT_FILE="$tmp/idle.json" "$SCRIPT_DIR/doctor.sh" | grep -F 'OK serving; indexer idle' >/dev/null

jq -nc --argjson last_progress "$(( $(date +%s) - 3600 ))" \
  '{handle:"@test",indexed:"2",last_progress:$last_progress}' > "$tmp/stalled-state.json"
if SNAPSHOT_FILE="$tmp/ok.json" DOCTOR_STATE_FILE="$tmp/stalled-state.json" STALL_MINUTES=30 \
  "$SCRIPT_DIR/doctor.sh" > "$tmp/output"; then
  echo "doctor should fail when indexing has not advanced" >&2
  exit 1
fi
grep -F 'FAIL indexing stalled @test at 2/3' "$tmp/output" >/dev/null
grep -F 'summarize-indexer-logs.sh' "$tmp/output" >/dev/null

cat > "$tmp/engine.json" <<'JSON'
{"frontend_http":200,"engine_http":503,"search":{"http":200},"indexing":{"status":"idle"},"worker":{"state":"started"}}
JSON
if SNAPSHOT_FILE="$tmp/engine.json" "$SCRIPT_DIR/doctor.sh" > "$tmp/output"; then
  echo "doctor should fail for an unhealthy engine" >&2
  exit 1
fi
grep -F 'FAIL Raw search health HTTP 503' "$tmp/output" >/dev/null
grep -F 'NEXT run:' "$tmp/output" >/dev/null

cat > "$tmp/worker.json" <<'JSON'
{"frontend_http":200,"engine_http":200,"search":{"http":200},"indexing":{"status":"queued","handle":"@test"},"worker":null}
JSON
if SNAPSHOT_FILE="$tmp/worker.json" "$SCRIPT_DIR/doctor.sh" > "$tmp/output"; then
  echo "doctor should fail when the queue worker is missing" >&2
  exit 1
fi
grep -F 'FAIL queue worker state missing' "$tmp/output" >/dev/null

mkdir "$tmp/scripts"
cp "$SCRIPT_DIR/doctor.sh" "$tmp/scripts/doctor.sh"
printf '#!/bin/sh\necho queue_configuration_checked\nexit "${FIXTURE_CHECK_EXIT:-0}"\n' > "$tmp/scripts/check-queue-indexers.sh"
printf '#!/bin/sh\necho recent_indexer_logs_checked\n' > "$tmp/scripts/summarize-indexer-logs.sh"
chmod +x "$tmp/scripts/"*.sh
"$tmp/scripts/doctor.sh" --indexer > "$tmp/output"
grep -F 'queue_configuration_checked' "$tmp/output" >/dev/null
grep -F 'recent_indexer_logs_checked' "$tmp/output" >/dev/null
if FIXTURE_CHECK_EXIT=1 "$tmp/scripts/doctor.sh" --indexer > "$tmp/output"; then
  echo "indexer doctor must preserve failed configuration checks" >&2
  exit 1
fi
grep -F 'recent_indexer_logs_checked' "$tmp/output" >/dev/null

echo "doctor_tests=ok"
