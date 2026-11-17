#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
trap 'exit 130' INT TERM
mkdir -p "$temporary_directory/bin"

cat > "$temporary_directory/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
[[ -z "${FAKE_FLY_CALL_LOG:-}" ]] || printf '%s %s\n' "$1" "$2" >> "$FAKE_FLY_CALL_LOG"
case "$1 $2" in
  "machine list")
    jq -n --arg state "${FAKE_MACHINE_STATE:-started}" --arg mount "${FLY_RESOURCE_MOUNT_PATH:-/data/mutable}" '[{
      id:"22222222222222",state:$state,
      config:{mounts:[{volume:"canary-volume",path:$mount}]}
    }]'
    ;;
  "ssh console")
    if [[ -n "${FAKE_CGROUP_ROOT:-}" ]]; then
      while [[ "$1" != --command ]]; do shift; done
      command="${2//\/sys\/fs\/cgroup/$FAKE_CGROUP_ROOT}"
      command="${command//\/data\/mutable/$FAKE_CGROUP_ROOT}"
      [[ "${FAKE_IGNORE_TERM:-0}" != 1 ]] || trap '' TERM
      sleep "${FAKE_SSH_DELAY:-0}"
      [[ "${FAKE_REMOTE_STALL:-0}" != 1 ]] || command="${command//sleep 1/sleep 7}"
      exec bash -c "$command"
    fi
    if [[ "${FAKE_CGROUP_VERSION:-2}" == 1 ]]; then
      cat <<'EOF'
1725000000123
345678901
345678901
0
0
0
0
1
sampled_current
sampled_current
/dev/vdb 20000000000 4000000000 16000000000 20% /data/mutable
EOF
    else
      cat <<'EOF'
1725000000456
123456789
234567890
0
0
2
1
2
kernel
kernel
/dev/vdb 20000000000 4000000000 16000000000 20% /data/mutable
EOF
    fi
    ;;
  *) printf 'unexpected flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$temporary_directory/bin/flyctl"

output="$(PATH="$temporary_directory/bin:$PATH" \
  FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh")"
jq -e '
  .app == "ytsearch-raw-production" and
  .machine_id == "22222222222222" and
  .state == "started" and
  .sampled_at_ms == 1725000000456 and
  .memory_current_bytes == 123456789 and
  .memory_peak_bytes == 234567890 and
  .swap_current_bytes == 0 and
  .swap_peak_bytes == 0 and
  .oom == 2 and .oom_kill == 1 and
  .cgroup_version == 2 and
  .memory_peak_source == "kernel" and .swap_peak_source == "kernel" and
  .disk_total_bytes == 20000000000 and
  .disk_percent == 20
' <<<"$output" >/dev/null

output="$(PATH="$temporary_directory/bin:$PATH" FAKE_CGROUP_VERSION=1 \
  FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh")"
jq -e '
  .cgroup_version == 1 and
  .sampled_at_ms == 1725000000123 and
  .memory_current_bytes == 345678901 and .memory_peak_bytes == 345678901 and
  .memory_peak_source == "sampled_current" and
  .swap_peak_source == "sampled_current"
' <<<"$output" >/dev/null

if PATH="$temporary_directory/bin:$PATH" FAKE_MACHINE_STATE=stopped \
  FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh" >/dev/null 2>&1; then
  printf 'stopped Machine must not produce a resource snapshot\n' >&2
  exit 1
fi

output="$(PATH="$temporary_directory/bin:$PATH" FAKE_MACHINE_STATE=stopped \
  FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable FLY_RESOURCE_ALLOW_STOPPED=1 \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh")"
jq -e '.state == "stopped" and (has("memory_current_bytes") | not)' <<<"$output" >/dev/null

for transition in starting stopping; do
  if PATH="$temporary_directory/bin:$PATH" FAKE_MACHINE_STATE="$transition" \
    FLY_RESOURCE_APP=ytsearch-raw-production FLY_RESOURCE_MACHINE_ID=22222222222222 \
    FLY_RESOURCE_MOUNT_PATH=/data/mutable \
    "$SCRIPT_DIR/fly-machine-resource-snapshot.sh" >/dev/null 2>&1; then
    printf 'reader transitions must fail without explicit permission\n' >&2
    exit 1
  fi
  output="$(PATH="$temporary_directory/bin:$PATH" FAKE_MACHINE_STATE="$transition" \
    FLY_RESOURCE_APP=ytsearch-raw-production FLY_RESOURCE_MACHINE_ID=22222222222222 \
    FLY_RESOURCE_MOUNT_PATH=/data/mutable FLY_RESOURCE_ALLOW_TRANSITIONS=1 \
    "$SCRIPT_DIR/fly-machine-resource-snapshot.sh")"
  jq -e --arg state "$transition" '.state == $state and
    (has("memory_current_bytes") | not)' <<<"$output" >/dev/null
done

mkdir -p "$temporary_directory/wrapper-bin"
cat > "$temporary_directory/wrapper-bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "machine list")
    if [[ " $* " == *" --app ytsearch-raw-production "* ]]; then
      printf '[{"id":"22222222222222","state":"started","config":{"mounts":[{"volume":"vol_1111111111111111","path":"/data/mutable"}]}}]\n'
    else
      printf '[{"id":"33333333333333","state":"stopped","config":{"mounts":[{"volume":"vol_2222222222222222","path":"/var/lib/ytsearch"}]}}]\n'
    fi
    ;;
  "ssh console")
    printf '1725000000789\n123\n234\n0\n0\n0\n0\n2\nkernel\nkernel\n/dev/vdb 20000000000 4000000000 16000000000 20%% /data/mutable\n'
    ;;
  *) printf 'unexpected wrapper flyctl command: %s\n' "$*" >&2; exit 1 ;;
esac
SH
chmod +x "$temporary_directory/wrapper-bin/flyctl"
wrapper_env=(
  PATH="$temporary_directory/wrapper-bin:$PATH"
  FLY_PRODUCTION_INVENTORY_PATH="$SCRIPT_DIR/testdata/fly-production-resources.json"
)
canary_output="$(env "${wrapper_env[@]}" "$SCRIPT_DIR/fly-raw-canary-resource-snapshot.sh")"
jq -e '
  .machine_id == "22222222222222" and .mount_path == "/data/mutable" and .state == "started"
' <<<"$canary_output" >/dev/null
builder_output="$(env "${wrapper_env[@]}" "$SCRIPT_DIR/fly-raw-builder-resource-snapshot.sh")"
jq -e '
  .machine_id == "33333333333333" and .mount_path == "/var/lib/ytsearch" and .state == "stopped"
' <<<"$builder_output" >/dev/null

mkdir -p "$temporary_directory/cgroup/memory"
printf '100000\n' > "$temporary_directory/cgroup/memory/memory.usage_in_bytes"
printf '200000\n' > "$temporary_directory/cgroup/memory/memory.max_usage_in_bytes"
printf '186016\n' > "$temporary_directory/cgroup/memory/memory.memsw.usage_in_bytes"
printf '0\n' > "$temporary_directory/cgroup/memory/memory.failcnt"
printf 'oom_kill 0\n' > "$temporary_directory/cgroup/memory/memory.oom_control"
probe_fixture() {
  PATH="$temporary_directory/bin:$PATH" \
  FAKE_CGROUP_ROOT="$temporary_directory/cgroup" \
  FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable \
    "$SCRIPT_DIR/fly-machine-resource-snapshot.sh"
}
printf 'swap 0\ntotal_swap 0\n' > "$temporary_directory/cgroup/memory/memory.stat"
probe_fixture | jq -e '.swap_current_bytes == 0 and .swap_peak_bytes == 0' >/dev/null
printf 'swap 0\ntotal_swap 8192\n' > "$temporary_directory/cgroup/memory/memory.stat"
probe_fixture | jq -e '.swap_current_bytes == 8192 and .swap_peak_source == "sampled_current"' >/dev/null
for invalid in 'swap 0' 'total_swap invalid' 'total_swap -1'; do
  printf '%s\n' "$invalid" > "$temporary_directory/cgroup/memory/memory.stat"
  if probe_fixture >/dev/null 2>&1; then
    printf 'missing or invalid total_swap must fail closed\n' >&2
    exit 1
  fi
done
# Execute the real remote collector against independent cgroup files. A slow
# connection setup must not turn a one-second on-VM cadence into operator time.
printf 'total_swap 8192\n' > "$temporary_directory/cgroup/memory/memory.stat"
run_stream_fixture() {
  local output="$1" stop="$2" stall="$3"
  PATH="$temporary_directory/bin:$PATH" FAKE_CGROUP_ROOT="$temporary_directory/cgroup" \
    FAKE_FLY_CALL_LOG="$output.calls" FAKE_SSH_DELAY=2 FAKE_REMOTE_STALL="$stall" \
    FLY_RESOURCE_APP=ytsearch-raw-production FLY_RESOURCE_MACHINE_ID=22222222222222 \
    FLY_RESOURCE_MOUNT_PATH=/data/mutable FLY_RESOURCE_STREAM=1 FLY_RESOURCE_STOP_FILE="$stop" \
    "$SCRIPT_DIR/fly-machine-resource-snapshot.sh" > "$output" &
  local collector=$!
  local deadline=$(( SECONDS + 20 ))
  while [[ "$(wc -l < "$output")" -lt 2 ]]; do
    if ! kill -0 "$collector" 2>/dev/null || (( SECONDS > deadline )); then
      kill "$collector" 2>/dev/null || true
      wait "$collector" || true
      printf 'stream did not produce complete samples\n' >&2; exit 1
    fi
    sleep 0.1
  done
  touch "$stop"
  wait "$collector"
  [[ "$(grep -c '^ssh console$' "$output.calls")" == 1 ]]
  [[ "$(grep -c '^machine list$' "$output.calls")" == 1 ]]
}
run_stream_fixture "$temporary_directory/stream.jsonl" "$temporary_directory/stop" 0
jq -se 'length >= 2 and all(.[]; .swap_current_bytes == 8192) and
  (.[1].sampled_at_ms - .[0].sampled_at_ms) >= 900 and
  (.[1].sampled_at_ms - .[0].sampled_at_ms) < 6000' \
  "$temporary_directory/stream.jsonl" >/dev/null
# A genuine on-VM stall remains visible and will fail the unchanged 6s gate.
run_stream_fixture "$temporary_directory/stall.jsonl" "$temporary_directory/stop-stall" 1
jq -se '(.[1].sampled_at_ms - .[0].sampled_at_ms) > 6000' \
  "$temporary_directory/stall.jsonl" >/dev/null
if PATH="$temporary_directory/bin:$PATH" FLY_RESOURCE_APP=ytsearch-raw-production \
  FLY_RESOURCE_MACHINE_ID=22222222222222 FLY_RESOURCE_MOUNT_PATH=/data/mutable \
  FLY_RESOURCE_STREAM=1 FLY_RESOURCE_STOP_FILE="$temporary_directory/no-stop" \
  "$SCRIPT_DIR/fly-machine-resource-snapshot.sh" >/dev/null 2>&1; then
  printf 'malformed stream frames must fail closed\n' >&2; exit 1
fi
# A hung SSH setup (including a client ignoring TERM) must not inherit the
# six-hour active-stream allowance, and cleanup must reap its process group.
startup_started=$SECONDS
if PATH="$temporary_directory/bin:$PATH" FAKE_CGROUP_ROOT="$temporary_directory/cgroup" \
  FAKE_SSH_DELAY=30 FAKE_IGNORE_TERM=1 FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS=2 \
  FLY_RESOURCE_APP=ytsearch-raw-production FLY_RESOURCE_MACHINE_ID=22222222222222 \
  FLY_RESOURCE_MOUNT_PATH=/data/mutable FLY_RESOURCE_STREAM=1 \
  FLY_RESOURCE_STOP_FILE="$temporary_directory/no-startup-stop" \
  timeout --kill-after=1 10 "$SCRIPT_DIR/fly-machine-resource-snapshot.sh" \
  > "$temporary_directory/startup.jsonl" 2> "$temporary_directory/startup.err"; then
  printf 'hung stream setup must fail closed\n' >&2; exit 1
fi
(( SECONDS - startup_started < 9 ))
grep -q 'first complete sample' "$temporary_directory/startup.err"
[[ ! -s "$temporary_directory/startup.jsonl" ]]
printf 'fly_machine_resource_snapshot=ok\n'
