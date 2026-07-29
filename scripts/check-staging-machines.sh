#!/usr/bin/env sh
set -eu

# Summarize the Fly machines that make up staging without exposing secrets.

APPS="${APPS:-ytsearch-api-typesense-fly-staging ytsearch-typesense-staging ytsearch-indexer-staging}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_command flyctl
require_command jq

if [ "${DETAILS:-0}" = "1" ]; then
  printf 'app\tmachine\tstate\tregion\tsize\timage\tvolume\n'
fi

apps_with_machines=0
availability_ok=true
indexer_ok=true
pacific_hhmm="$(TZ=America/Los_Angeles date +%H%M)"
schedule_mode="${SEARCH_SCHEDULE_MODE:-observe}"
typesense_should_run=true
if [ "$schedule_mode" = "enforce" ] && [ "$pacific_hhmm" -lt 1145 ]; then
  typesense_should_run=false
elif [ "$pacific_hhmm" -ge 1145 ]; then
  typesense_should_run=true
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
    ytsearch-indexer-staging)
      machine_count="$(printf '%s' "$json" | jq 'length')"
      if [ "$machine_count" -ne 1 ]; then
        indexer_ok=false
      fi
      ;;
    ytsearch-typesense-staging)
      if { [ "$typesense_should_run" = true ] && [ "$started_for_app" -ne 1 ]; } ||
        { [ "$typesense_should_run" = false ] && [ "$started_for_app" -ne 0 ]; }; then
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

echo "apps=$apps_with_machines availability_ok=$availability_ok schedule_mode=$schedule_mode typesense_should_run=$typesense_should_run one_indexer=$indexer_ok ok=$ok"
