#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM
mkdir -p "$temporary_directory/bin"

cat > "$temporary_directory/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "machine list")
    jq -n --arg state "${FAKE_MACHINE_STATE:-started}" '[{
      id:"22222222222222",state:$state,
      config:{mounts:[{volume:"canary-volume",path:"/data/mutable"}]}
    }]'
    ;;
  "ssh console")
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

printf 'fly_machine_resource_snapshot=ok\n'
