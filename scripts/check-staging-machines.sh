#!/usr/bin/env sh
set -eu

# Summarize the Fly machines that make up staging without exposing secrets.

APPS="${APPS:-ytsearch-api-raw-production ytsearch-raw-production ytsearch-indexer-raw-production}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_command flyctl
require_command jq
require_command curl

if [ "${DETAILS:-0}" = "1" ]; then
  printf 'app\tmachine\tstate\tregion\tsize\timage\tvolume\n'
fi

apps_with_machines=0
availability_ok=true
indexer_ok=true
pacific_hhmm="$(TZ=America/Los_Angeles date +%H%M)"
availability_url="${SEARCH_AVAILABILITY_URL:-https://search.pronsh.dev/api/availability}"
availability_json="$(curl -fsS --max-time 5 "$availability_url" 2>/dev/null || true)"
if [ -n "$availability_json" ]; then
  live_schedule_mode="$(printf '%s' "$availability_json" | jq -r '
    if .schedule_policy == "always_on" then "always_on"
    elif .schedule_policy == "scheduled" or .enforced == true then "enforce"
    else "observe"
    end
  ')"
else
  live_schedule_mode="unknown"
  availability_ok=false
fi
schedule_mode="${SEARCH_SCHEDULE_MODE:-$live_schedule_mode}"
raw_should_run=true
if [ "$schedule_mode" = "enforce" ] && [ "$pacific_hhmm" -lt 1145 ]; then
  raw_should_run=false
elif [ "$pacific_hhmm" -ge 1145 ]; then
  raw_should_run=true
fi

for app in $APPS; do
  json="$(flyctl machine list --app "$app" --json 2>/dev/null || printf '[]')"
  if [ "$(printf '%s' "$json" | jq 'length')" -eq 0 ]; then
    if [ "${DETAILS:-0}" = "1" ]; then
      printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$app" "-" "no-machines" "-" "-" "-" "-"
    fi
    continue
  fi

  apps_with_machines=$((apps_with_machines + 1))

  started_for_app="$(printf '%s' "$json" | jq '[.[] | select(.state == "started")] | length')"
  case "$app" in
    ytsearch-indexer-raw-production)
      machine_count="$(printf '%s' "$json" | jq 'length')"
      if [ "$machine_count" -ne 1 ]; then
        indexer_ok=false
      fi
      ;;
    ytsearch-raw-production)
      if { [ "$raw_should_run" = true ] && [ "$started_for_app" -ne 1 ]; } ||
        { [ "$raw_should_run" = false ] && [ "$started_for_app" -ne 0 ]; }; then
        availability_ok=false
      fi
      ;;
    *)
      if [ "$started_for_app" -ne 1 ]; then
        availability_ok=false
      fi
      ;;
  esac

  if [ "${DETAILS:-0}" = "1" ]; then
    printf '%s' "$json" |
      jq -r --arg app "$app" '
      .[] |
      [
        $app,
        .id,
        .state,
        .region,
        (
          [
            (.config.guest.cpu_kind // ""),
            (.config.guest.cpus // "" | tostring),
            (.config.guest.memory_mb // "" | tostring)
          ] | join("/")
        ),
        (.image_ref.registry // .image_ref.repository // "-"),
        (
          [.config.mounts[]? | "\(.volume)/\(.path)"] |
          if length == 0 then "-" else join(",") end
        )
      ] | @tsv'
  fi
done

ok=false
if [ "$apps_with_machines" -eq 3 ] && [ "$availability_ok" = true ] && [ "$indexer_ok" = true ]; then
  ok=true
fi

echo "apps=$apps_with_machines availability_ok=$availability_ok schedule_mode=$schedule_mode raw_should_run=$raw_should_run one_indexer=$indexer_ok ok=$ok"
