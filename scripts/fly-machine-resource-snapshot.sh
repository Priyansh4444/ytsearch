#!/usr/bin/env bash
set -euo pipefail

: "${FLY_RESOURCE_APP:?set FLY_RESOURCE_APP}"
: "${FLY_RESOURCE_MACHINE_ID:?set FLY_RESOURCE_MACHINE_ID to one exact Machine}"
: "${FLY_RESOURCE_MOUNT_PATH:?set FLY_RESOURCE_MOUNT_PATH}"
: "${FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS:=15}"

if [[ ! "$FLY_RESOURCE_APP" =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]]; then
  printf 'FLY_RESOURCE_APP is invalid\n' >&2
  exit 1
fi
if [[ ! "$FLY_RESOURCE_MACHINE_ID" =~ ^[0-9a-f]{14}$ ]]; then
  printf 'FLY_RESOURCE_MACHINE_ID must be one exact 14-character Machine ID\n' >&2
  exit 1
fi
case "$FLY_RESOURCE_MOUNT_PATH" in
  /data/mutable|/var/lib/ytsearch) ;;
  *) printf 'FLY_RESOURCE_MOUNT_PATH is not an approved Raw volume path\n' >&2; exit 1 ;;
esac
if [[ ! "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]; then
  printf 'FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS must be a positive integer\n' >&2
  exit 1
fi

for command in flyctl jq timeout; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

machine="$(timeout "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" \
  flyctl machine list --app "$FLY_RESOURCE_APP" --json |
  jq -ce --arg id "$FLY_RESOURCE_MACHINE_ID" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact resource Machine not found") end
  ')"
machine_state="$(jq -r '.state' <<<"$machine")"
if [[ "$machine_state" == stopped && "${FLY_RESOURCE_ALLOW_STOPPED:-0}" == 1 ]]; then
  jq -cn --arg app "$FLY_RESOURCE_APP" --arg machine_id "$FLY_RESOURCE_MACHINE_ID" \
    --arg mount_path "$FLY_RESOURCE_MOUNT_PATH" --arg state "$machine_state" \
    --argjson sampled_at_ms "$(date +%s%3N)" '{
      app:$app,machine_id:$machine_id,mount_path:$mount_path,
      sampled_at_ms:$sampled_at_ms,state:$state
    }'
  exit 0
fi
if ! jq -e --arg mount "$FLY_RESOURCE_MOUNT_PATH" '
  .state == "started" and any(.config.mounts[]?; .path == $mount)
' <<<"$machine" >/dev/null; then
  printf 'resource Machine is not started with the expected volume mount\n' >&2
  exit 1
fi

remote_output="$(timeout "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" \
  flyctl ssh console --app "$FLY_RESOURCE_APP" \
  --machine "$FLY_RESOURCE_MACHINE_ID" --command \
  "sh -c 'cat /sys/fs/cgroup/memory.current; cat /sys/fs/cgroup/memory.peak; cat /sys/fs/cgroup/memory.swap.current; cat /sys/fs/cgroup/memory.swap.peak; cat /sys/fs/cgroup/memory.events; df -P -B1 $FLY_RESOURCE_MOUNT_PATH | tail -n 1'")"

memory_current="$(sed -n '1p' <<<"$remote_output")"
memory_peak="$(sed -n '2p' <<<"$remote_output")"
swap_current="$(sed -n '3p' <<<"$remote_output")"
swap_peak="$(sed -n '4p' <<<"$remote_output")"
oom="$(awk '$1 == "oom" { print $2 }' <<<"$remote_output")"
oom_kill="$(awk '$1 == "oom_kill" { print $2 }' <<<"$remote_output")"
disk_line="$(tail -n 1 <<<"$remote_output")"
disk_total="$(awk '{print $2}' <<<"$disk_line")"
disk_used="$(awk '{print $3}' <<<"$disk_line")"
disk_available="$(awk '{print $4}' <<<"$disk_line")"
disk_percent="$(awk '{gsub(/%/, "", $5); print $5}' <<<"$disk_line")"

for value in "$memory_current" "$memory_peak" "$swap_current" "$swap_peak" "$oom" "$oom_kill" \
  "$disk_total" "$disk_used" "$disk_available" "$disk_percent"; do
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    printf 'could not parse numeric cgroup or volume metrics\n' >&2
    exit 1
  fi
done

jq -cn \
  --arg app "$FLY_RESOURCE_APP" \
  --arg machine_id "$FLY_RESOURCE_MACHINE_ID" \
  --arg mount_path "$FLY_RESOURCE_MOUNT_PATH" \
  --argjson sampled_at_ms "$(date +%s%3N)" \
  --argjson memory_current "$memory_current" \
  --argjson memory_peak "$memory_peak" \
  --argjson swap_current "$swap_current" \
  --argjson swap_peak "$swap_peak" \
  --argjson oom "$oom" \
  --argjson oom_kill "$oom_kill" \
  --argjson disk_total "$disk_total" \
  --argjson disk_used "$disk_used" \
  --argjson disk_available "$disk_available" \
  --argjson disk_percent "$disk_percent" '{
    app:$app,
    machine_id:$machine_id,
    mount_path:$mount_path,
    sampled_at_ms:$sampled_at_ms,
    state:"started",
    memory_current_bytes:$memory_current,
    memory_peak_bytes:$memory_peak,
    swap_current_bytes:$swap_current,
    swap_peak_bytes:$swap_peak,
    oom:$oom,
    oom_kill:$oom_kill,
    disk_total_bytes:$disk_total,
    disk_used_bytes:$disk_used,
    disk_available_bytes:$disk_available,
    disk_percent:$disk_percent
  }'
