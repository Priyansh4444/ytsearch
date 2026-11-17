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
if ! jq -e --arg mount "$FLY_RESOURCE_MOUNT_PATH"   'any(.config.mounts[]?; .path == $mount)' <<<"$machine" >/dev/null; then
  printf 'resource Machine lacks the expected volume mount\n' >&2
  exit 1
fi
if [[ "$machine_state" == stopped && "${FLY_RESOURCE_ALLOW_STOPPED:-0}" == 1 ]]   || [[ "$machine_state" =~ ^(starting|stopping)$ && "${FLY_RESOURCE_ALLOW_TRANSITIONS:-0}" == 1 ]]; then
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
emit_snapshot() {
  local remote_output="$1"

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
}

if [[ "${FLY_RESOURCE_STREAM:-0}" != 1 ]]; then
  remote_output="$(timeout "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" \
    flyctl ssh console --app "$FLY_RESOURCE_APP" \
    --machine "$FLY_RESOURCE_MACHINE_ID" --command \
    "sh -c '$remote_script' sh '$FLY_RESOURCE_MOUNT_PATH'")"
  emit_snapshot "$remote_output"
  exit 0
fi

: "${FLY_RESOURCE_STOP_FILE:?stream requires a caller-owned stop file}"
# Bound orphaned collectors even if the controlling process disappears. The
# caller normally stops this much sooner via its stop file.
stream_script="sample() {
$remote_script
}
count=0
while [ \$count -lt 21600 ]; do
  record=\$(sample \"\$1\") || exit 1
  printf \"%s\\n\" \"\$record\" | tr \"\\n\" \"\\t\"
  printf \"\\n\"
  count=\$((count + 1))
  sleep 1
done"
stream_directory="$(mktemp -d)"
chmod 700 "$stream_directory"
mkfifo "$stream_directory/samples"
ssh_pid=""
watcher_pid=""
terminate_stream_process() {
  local pid="$1"
  if kill -0 "$pid" 2>/dev/null; then
    # timeout owns a separate process group; terminate the SSH client too,
    # even if it ignores TERM and keeps the FIFO open.
    kill -TERM -- "-$pid" 2>/dev/null || kill -TERM "$pid" 2>/dev/null || true
    sleep 2
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
}
cleanup_stream() {
  trap - EXIT INT TERM
  if [[ -n "$ssh_pid" ]]; then
    terminate_stream_process "$ssh_pid"
    wait "$ssh_pid" 2>/dev/null || true
  fi
  if [[ -n "$watcher_pid" ]]; then
    kill "$watcher_pid" 2>/dev/null || true
    wait "$watcher_pid" 2>/dev/null || true
  fi
  rm -rf "$stream_directory"
}
trap cleanup_stream EXIT
trap 'exit 130' INT TERM
timeout --kill-after=2 21630 flyctl ssh console --app "$FLY_RESOURCE_APP" \
  --machine "$FLY_RESOURCE_MACHINE_ID" --command \
  "sh -c '$stream_script' sh '$FLY_RESOURCE_MOUNT_PATH'" > "$stream_directory/samples" &
ssh_pid=$!
(
  startup_deadline=$(( SECONDS + FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS ))
  while kill -0 "$ssh_pid" 2>/dev/null; do
    if [[ -e "$FLY_RESOURCE_STOP_FILE" ]]; then
      terminate_stream_process "$ssh_pid"
      exit 0
    fi
    if [[ ! -e "$stream_directory/ready" ]] && (( SECONDS >= startup_deadline )); then
      touch "$stream_directory/startup-timeout"
      terminate_stream_process "$ssh_pid"
      exit 0
    fi
    sleep 0.2
  done
) &
watcher_pid=$!
samples=0
while IFS= read -r record; do
  # Every remote sample is one complete frame. Do not synthesize timestamps or
  # accept missing counters when a connection drops between samples.
  remote_output="$(printf '%s' "$record" | tr '\t' '\n')"
  [[ "$(printf '%s\n' "$remote_output" | wc -l)" == 11 ]] || {
    printf 'incomplete remote resource sample\n' >&2; exit 1;
  }
  emit_snapshot "$remote_output"
  touch "$stream_directory/ready"
  ((samples += 1))
done < "$stream_directory/samples"
status=0
wait "$ssh_pid" || status=$?
ssh_pid=""
if [[ -e "$FLY_RESOURCE_STOP_FILE" ]]; then exit 0; fi
if [[ -e "$stream_directory/startup-timeout" ]]; then
  printf 'resource stream did not produce its first complete sample before the startup deadline\n' >&2
  exit 1
fi
if (( status != 0 )); then
  # An on-demand builder normally stops after its final independently verified
  # snapshot. Only an authoritative lifecycle read can explain a closed stream.
  state="$(timeout "$FLY_RESOURCE_COMMAND_TIMEOUT_SECONDS" \
    flyctl machine list --app "$FLY_RESOURCE_APP" --json | \
    jq -er --arg id "$FLY_RESOURCE_MACHINE_ID" '.[]|select(.id==$id)|.state')"
  if [[ "${FLY_RESOURCE_ALLOW_STOPPED:-0}" == 1 && "$state" =~ ^(stopping|stopped)$ ]] && (( samples > 0 )); then exit 0; fi
  printf 'resource stream ended unexpectedly\n' >&2
  exit 1
fi
printf 'resource stream exhausted its bounded lifetime\n' >&2
exit 1
