#!/usr/bin/env bash

# The caller owns the exact Machine lease and stops it on every exit path.
fly_publication_job_command() {
  local job_directory="$1" duration="$2" kill_after="$3" handshake="${4:-0}" program
  [[ "$job_directory" =~ ^/tmp/ytsearch-publication-[a-zA-Z0-9]+$ ]] || return 2
  [[ "$duration" =~ ^[1-9][0-9]*$ && "$kill_after" =~ ^[1-9][0-9]*$ ]] || return 2
  [[ "$handshake" == 0 || "$handshake" == 1 ]] || return 2
  program=$(cat <<'SH'
set -eu
# ytsearch bounded publication job
job=$1
duration=$2
kill_after=$3
handshake=$4
for command in nohup timeout sh mv tail date stat touch cat; do
  command -v "$command" >/dev/null
done
if mkdir -m 700 "$job" 2>/dev/null; then
  nohup sh -c '
    printf "started\n" > "$3/started"
    if [ "$4" = 1 ]; then
      export RAW_PUBLICATION_ACTIVATION_READY_FILE="$3/activation-ready"
      export RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE="$3/activation-continue"
      export RAW_PUBLICATION_ACTIVATION_STARTED_FILE="$3/activation-started"
      export RAW_PUBLICATION_ACTIVATION_FINISHED_FILE="$3/activation-finished"
    fi
    timeout --signal=TERM --kill-after="$2" "$1" ytsearch publish --backend raw
    result=$?
    printf "%s\n" "$result" > "$3/result.next"
    mv "$3/result.next" "$3/result"
  ' sh "$duration" "$kill_after" "$job" "$handshake" </dev/null >"$job/log" 2>&1 &
elif [ ! -d "$job" ]; then
  echo "cannot create publication job directory" >&2
  exit 1
fi
if [ -f "$job/result" ]; then
  result=$(cat "$job/result")
  case "$result" in
    0)
      if [ "$handshake" = 1 ]; then
        printf 'complete:%s:%s\n' "$(cat "$job/activation-started")" \
          "$(cat "$job/activation-finished")"
      else
        printf 'complete\n'
      fi
      ;;
    ''|*[!0-9]*) echo "invalid publication result" >&2; exit 1 ;;
    *) printf 'failed:%s\n' "$result"; tail -n 20 "$job/log" >&2 ;;
  esac
elif [ -f "$job/activation-ready" ] && [ ! -f "$job/activation-continue" ]; then
  printf 'activation-ready\n'
elif [ -f "$job/activation-finished" ]; then
  printf 'activation-finished:%s:%s\n' "$(cat "$job/activation-started")" \
    "$(cat "$job/activation-finished")"
elif [ -f "$job/activation-started" ]; then
  printf 'activation-started:%s\n' "$(cat "$job/activation-started")"
elif [ ! -f "$job/started" ] && [ "$(($(date +%s) - $(stat -c %Y "$job")))" -ge 30 ]; then
  printf 'failed:launch-unconfirmed\n'
  echo "job claimed but publisher did not acknowledge startup within 30s; refusing duplicate launch" >&2
else
  printf 'pending\n'
fi
SH
)
  program="${program//\'/\'\\\'\'}"
  printf "sh -c '%s' sh %s %s %s %s" "$program" "$job_directory" "$duration" "$kill_after" "$handshake"
}

fly_publication_job_wait() {
  local job_directory="$1" diagnostics="$2" command status remaining request_budget
  local started finished
  local deadline=$(( SECONDS + RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS ))
  local handshake=0 activation_ready_file="${RAW_PUBLICATION_ACTIVATION_READY_FILE:-}"
  local activation_continue_file="${RAW_PUBLICATION_ACTIVATION_CONTINUE_FILE:-}"
  local activation_started_file="${RAW_PUBLICATION_ACTIVATION_STARTED_FILE:-}"
  local activation_finished_file="${RAW_PUBLICATION_ACTIVATION_FINISHED_FILE:-}"
  local connection_failed=0
  if [[ -n "$activation_ready_file" || -n "$activation_continue_file" \
    || -n "$activation_started_file" || -n "$activation_finished_file" ]]; then
    [[ -n "$activation_ready_file" && -n "$activation_continue_file" \
      && -n "$activation_started_file" && -n "$activation_finished_file" \
      && "$activation_ready_file" == /* && "$activation_continue_file" == /* \
      && "$activation_started_file" == /* && "$activation_finished_file" == /* ]] || {
      printf 'local publication activation handshake paths must be absolute and set together\n' >&2
      return 2
    }
    handshake=1
  fi
  command=$(fly_publication_job_command "$job_directory" \
    "$RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS" \
    "$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS" "$handshake") || return
  printf 'Publication deadline: %ss; remote log: %s/log\n' \
    "$RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS" "$job_directory" >&2
  while (( SECONDS < deadline )); do
    remaining=$(( deadline - SECONDS ))
    request_budget="$RAW_BUILDER_COMMAND_TIMEOUT_SECONDS"
    (( request_budget <= remaining )) || request_budget="$remaining"
    if status=$(bounded_flyctl "$request_budget" ssh console --app "$RAW_BUILDER_APP" \
      --machine "$RAW_BUILDER_MACHINE_ID" --command "$command" 2>"$diagnostics"); then
      case "$status" in
        complete) return 0 ;;
        complete:[0-9]*:[0-9]*|activation-finished:[0-9]*:[0-9]*)
          started="${status#*:}"
          finished="${started#*:}"
          started="${started%%:*}"
          printf '%s\n' "$started" > "$activation_started_file"
          printf '%s\n' "$finished" > "$activation_finished_file"
          [[ "$status" == complete:* ]] && return 0
          ;;
        activation-started:[0-9]*)
          printf '%s\n' "${status#*:}" > "$activation_started_file"
          ;;
        failed:*)
          printf 'Remote publication %s\n' "$status" >&2
          tail -n 20 "$diagnostics" >&2
          return 1
          ;;
        activation-ready)
          touch "$activation_ready_file"
          if [[ -e "$activation_continue_file" ]]; then
            bounded_flyctl "$request_budget" ssh console --app "$RAW_BUILDER_APP" \
              --machine "$RAW_BUILDER_MACHINE_ID" \
              --command "touch $job_directory/activation-continue" >/dev/null
          fi
          ;;
        pending) ;;
        *) printf 'Unrecognized publication job status; refusing success\n' >&2; return 1 ;;
      esac
    else
      cp "$diagnostics" "$diagnostics.failure"
      if (( connection_failed == 0 )); then
        printf 'Publication SSH unavailable; checking the same job until its deadline\n' >&2
        connection_failed=1
      fi
    fi
    remaining=$(( deadline - SECONDS ))
    (( remaining > 0 )) || break
    (( remaining <= 5 )) || remaining=5
    sleep "$remaining"
  done
  printf 'Publication deadline exceeded; completion unconfirmed, stopping leased builder\n' >&2
  if [[ -f "$diagnostics.failure" ]]; then
    tail -n 20 "$diagnostics.failure" >&2
  fi
  return 124
}
