#!/usr/bin/env bash

# The caller owns the exact Machine lease and stops it on every exit path.
fly_publication_job_command() {
  local job_directory="$1" duration="$2" kill_after="$3" program
  [[ "$job_directory" =~ ^/tmp/ytsearch-publication-[a-zA-Z0-9]+$ ]] || return 2
  [[ "$duration" =~ ^[1-9][0-9]*$ && "$kill_after" =~ ^[1-9][0-9]*$ ]] || return 2
  program=$(cat <<'SH'
set -eu
# ytsearch bounded publication job
job=$1
duration=$2
kill_after=$3
for command in nohup timeout sh mv tail date stat; do
  command -v "$command" >/dev/null
done
if mkdir -m 700 "$job" 2>/dev/null; then
  nohup sh -c '
    printf "started\n" > "$3/started"
    timeout --signal=TERM --kill-after="$2" "$1" ytsearch publish --backend raw
    result=$?
    printf "%s\n" "$result" > "$3/result.next"
    mv "$3/result.next" "$3/result"
  ' sh "$duration" "$kill_after" "$job" </dev/null >"$job/log" 2>&1 &
elif [ ! -d "$job" ]; then
  echo "cannot create publication job directory" >&2
  exit 1
fi
if [ -f "$job/result" ]; then
  result=$(cat "$job/result")
  case "$result" in
    0) printf 'complete\n' ;;
    ''|*[!0-9]*) echo "invalid publication result" >&2; exit 1 ;;
    *) printf 'failed:%s\n' "$result"; tail -n 20 "$job/log" >&2 ;;
  esac
elif [ ! -f "$job/started" ] && [ "$(($(date +%s) - $(stat -c %Y "$job")))" -ge 30 ]; then
  printf 'failed:launch-unconfirmed\n'
  echo "job claimed but publisher did not acknowledge startup within 30s; refusing duplicate launch" >&2
else
  printf 'pending\n'
fi
SH
)
  program="${program//\'/\'\\\'\'}"
  printf "sh -c '%s' sh %s %s %s" "$program" "$job_directory" "$duration" "$kill_after"
}

fly_publication_job_wait() {
  local job_directory="$1" diagnostics="$2" command status remaining request_budget
  local deadline=$(( SECONDS + RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS ))
  local connection_failed=0
  command=$(fly_publication_job_command "$job_directory" \
    "$RAW_BUILDER_PUBLICATION_COMMAND_TIMEOUT_SECONDS" "$RAW_BUILDER_COMMAND_KILL_AFTER_SECONDS") || return
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
        failed:*)
          printf 'Remote publication %s\n' "$status" >&2
          tail -n 20 "$diagnostics" >&2
          return 1
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
