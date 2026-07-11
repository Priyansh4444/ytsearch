#!/usr/bin/env sh
set -eu

# Summarize the Fly machines that make up staging without exposing secrets.

APPS="${APPS:-ytsearch-api-typesense-fly-staging ytsearch-index-queue-staging ytsearch-typesense-staging ytsearch-indexer-staging}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

require_command flyctl
require_command jq

printf 'app\tmachine\tstate\tregion\tsize\timage\tvolume\n'

started=0
apps_with_machines=0

for app in $APPS; do
  json="$(flyctl machine list --app "$app" --json 2>/dev/null || printf '[]')"
  if [ "$(printf '%s' "$json" | jq 'length')" -eq 0 ]; then
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$app" "-" "no-machines" "-" "-" "-" "-"
    continue
  fi

  apps_with_machines=$((apps_with_machines + 1))

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

  started=$((started + $(printf '%s' "$json" | jq '[.[] | select(.state == "started")] | length')))
done

echo
echo "summary started_machines=$started apps_with_machines=$apps_with_machines expected_staging_machines=4"
echo "see docs/staging-architecture.md for the active Typesense topology"
