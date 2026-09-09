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

remote_script='set -eu
mount_path="$1"
sampled_at_ms=$(date +%s%3N)
if [ -r /sys/fs/cgroup/memory.current ]; then
  memory_current=$(cat /sys/fs/cgroup/memory.current)
  memory_peak=$(cat /sys/fs/cgroup/memory.peak)
  swap_current=$(cat /sys/fs/cgroup/memory.swap.current)
  swap_peak=$(cat /sys/fs/cgroup/memory.swap.peak)
  oom=0
  oom_kill=0
  while read -r event count; do
    case "$event" in
      oom) oom="$count" ;;
      oom_kill) oom_kill="$count" ;;
    esac
  done < /sys/fs/cgroup/memory.events
  cgroup_version=2
  memory_peak_source=kernel
  swap_peak_source=kernel
elif [ -r /sys/fs/cgroup/memory/memory.usage_in_bytes ]; then
  memory_current=$(cat /sys/fs/cgroup/memory/memory.usage_in_bytes)
  reported_memory_peak=$(cat /sys/fs/cgroup/memory/memory.max_usage_in_bytes)
  if [ "$reported_memory_peak" -ge "$memory_current" ] && [ "$reported_memory_peak" -gt 0 ]; then
    memory_peak="$reported_memory_peak"
    memory_peak_source=kernel
  else
    memory_peak="$memory_current"
    memory_peak_source=sampled_current
  fi
  if [ ! -r /sys/fs/cgroup/memory/memory.memsw.usage_in_bytes ]; then
    printf "cgroup v1 memsw accounting is unavailable\n" >&2
    exit 1
  fi
  swap_current=""
  while read -r event count; do
    case "$event" in
      total_swap) swap_current="$count" ;;
    esac
  done < /sys/fs/cgroup/memory/memory.stat
  case "$swap_current" in
    ""|*[!0-9]*)
      printf "cgroup v1 total_swap accounting is unavailable or invalid\n" >&2
      exit 1
      ;;
  esac
  swap_peak="$swap_current"
  swap_peak_source=sampled_current
  oom=$(cat /sys/fs/cgroup/memory/memory.failcnt)
  oom_kill=0
  while read -r event count; do
    case "$event" in
      oom_kill) oom_kill="$count" ;;
    esac
  done < /sys/fs/cgroup/memory/memory.oom_control
  cgroup_version=1
else
  printf "supported memory cgroup counters are unavailable\n" >&2
  exit 1
fi
printf "%s\n" "$sampled_at_ms" "$memory_current" "$memory_peak" "$swap_current" "$swap_peak" \
  "$oom" "$oom_kill" "$cgroup_version" "$memory_peak_source" "$swap_peak_source"
df -P -B1 "$mount_path" | tail -n 1'
remote_output="$(timeout "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" \
  flyctl ssh console --app "$FLY_RESOURCE_APP" \
  --machine "$FLY_RESOURCE_MACHINE_ID" --command \
  "sh -c '$remote_script' sh '$FLY_RESOURCE_MOUNT_PATH'")"

sampled_at_ms="$(sed -n '1p' <<<"$remote_output")"
memory_current="$(sed -n '2p' <<<"$remote_output")"
memory_peak="$(sed -n '3p' <<<"$remote_output")"
swap_current="$(sed -n '4p' <<<"$remote_output")"
swap_peak="$(sed -n '5p' <<<"$remote_output")"
oom="$(sed -n '6p' <<<"$remote_output")"
oom_kill="$(sed -n '7p' <<<"$remote_output")"
cgroup_version="$(sed -n '8p' <<<"$remote_output")"
memory_peak_source="$(sed -n '9p' <<<"$remote_output")"
swap_peak_source="$(sed -n '10p' <<<"$remote_output")"
disk_line="$(sed -n '11p' <<<"$remote_output")"
disk_total="$(awk '{print $2}' <<<"$disk_line")"
disk_used="$(awk '{print $3}' <<<"$disk_line")"
disk_available="$(awk '{print $4}' <<<"$disk_line")"
disk_percent="$(awk '{gsub(/%/, "", $5); print $5}' <<<"$disk_line")"

for value in "$sampled_at_ms" "$memory_current" "$memory_peak" "$swap_current" "$swap_peak" "$oom" "$oom_kill" \
  "$disk_total" "$disk_used" "$disk_available" "$disk_percent"; do
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    printf 'could not parse numeric cgroup or volume metrics\n' >&2
    exit 1
  fi
done
if [[ ! "$cgroup_version" =~ ^[12]$ ]] \
  || [[ ! "$memory_peak_source" =~ ^(kernel|sampled_current)$ ]] \
  || [[ ! "$swap_peak_source" =~ ^(kernel|sampled_current)$ ]]; then
  printf 'could not parse cgroup accounting semantics\n' >&2
  exit 1
fi

jq -cn \
  --arg app "$FLY_RESOURCE_APP" \
  --arg machine_id "$FLY_RESOURCE_MACHINE_ID" \
  --arg mount_path "$FLY_RESOURCE_MOUNT_PATH" \
  --argjson sampled_at_ms "$sampled_at_ms" \
  --argjson memory_current "$memory_current" \
  --argjson memory_peak "$memory_peak" \
  --argjson swap_current "$swap_current" \
  --argjson swap_peak "$swap_peak" \
  --argjson oom "$oom" \
  --argjson oom_kill "$oom_kill" \
  --argjson cgroup_version "$cgroup_version" \
  --arg memory_peak_source "$memory_peak_source" \
  --arg swap_peak_source "$swap_peak_source" \
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
    cgroup_version:$cgroup_version,
    memory_peak_source:$memory_peak_source,
    swap_peak_source:$swap_peak_source,
    disk_total_bytes:$disk_total,
    disk_used_bytes:$disk_used,
    disk_available_bytes:$disk_available,
    disk_percent:$disk_percent
  }'
