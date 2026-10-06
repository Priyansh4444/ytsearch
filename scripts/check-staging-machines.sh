#!/usr/bin/env bash
set -euo pipefail

# Verify the exact production Raw topology without exposing secrets. Runtime
# identities come from the same inventory used by release and Worker tooling.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/fly-production-inventory.sh
source "$SCRIPT_DIR/lib/fly-production-inventory.sh"
fly_production_inventory_load_raw_runtime
fly_production_inventory_load_raw_release

for command in curl flyctl jq; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
trap 'exit 130' INT TERM
api_machines="$temporary_directory/api.json"
reader_machines="$temporary_directory/readers.json"
indexer_machines="$temporary_directory/indexers.json"
api_volumes="$temporary_directory/api-volumes.json"
reader_volumes="$temporary_directory/reader-volumes.json"
indexer_volumes="$temporary_directory/indexer-volumes.json"

flyctl machine list --app "$RAW_API_APP" --json > "$api_machines"
flyctl machine list --app "$RAW_READER_APP" --json > "$reader_machines"
flyctl machine list --app "$RAW_QUEUE_WORKER_APP" --json > "$indexer_machines"
flyctl volumes list --app "$RAW_API_APP" --json > "$api_volumes"
flyctl volumes list --app "$RAW_READER_APP" --json > "$reader_volumes"
flyctl volumes list --app "$RAW_QUEUE_WORKER_APP" --json > "$indexer_volumes"

pacific_hhmm="$(TZ=America/Los_Angeles date +%H%M)"
availability_url="${SEARCH_AVAILABILITY_URL:-https://search.pronsh.dev/api/availability}"
availability_json="$(curl --fail --silent --show-error --max-time 5 \
  "$availability_url" 2>/dev/null || true)"
availability_ok=true
if [[ -n "$availability_json" ]]; then
  schedule_mode="$(jq -r '
    if .schedule_policy == "always_on" then "always_on"
    elif .schedule_policy == "scheduled" or .enforced == true then "enforce"
    else "observe"
    end
  ' <<<"$availability_json")"
else
  schedule_mode=unknown
  availability_ok=false
fi
raw_expected_state=started
if [[ "$schedule_mode" == enforce && "$pacific_hhmm" < 1145 ]]; then
  raw_expected_state=stopped
fi

exact_machine_set() {
  local machine_file="$1"
  local expected
  shift
  expected="$(printf '%s\n' "$@" | jq -Rsc 'split("\n") | map(select(length > 0)) | sort')"
  jq -e --argjson expected "$expected" '([.[].id] | sort) == $expected' \
    "$machine_file" >/dev/null
}

exact_volume_set() {
  local volume_file="$1"
  local expected
  shift
  expected="$(printf '%s\n' "$@" | jq -Rsc 'split("\n") | map(select(length > 0)) | sort')"
  jq -e --argjson expected "$expected" '([.[].id] | sort) == $expected' \
    "$volume_file" >/dev/null
}

resource_matches() {
  local machine_file="$1"
  local machine_id="$2"
  local volume_id="$3"
  local mount_path="$4"
  local allowed_states="$5"
  jq -e --arg id "$machine_id" --arg volume "$volume_id" \
    --arg mount "$mount_path" --arg states "$allowed_states" '
    [.[] | select(.id == $id)] |
    length == 1 and
    (.[0].state as $state | ($states | split(",") | index($state)) != null) and
    ((.[0].config.mounts // []) | map({volume, path})) == [{volume:$volume,path:$mount}]
  ' "$machine_file" >/dev/null
}

resources_ok=true
exact_machine_set "$api_machines" "$RAW_API_MACHINE_ID" || resources_ok=false
exact_machine_set "$reader_machines" \
  "$RAW_READER_MACHINE_ID" "$RAW_CANARY_MACHINE_ID" || resources_ok=false
exact_machine_set "$indexer_machines" \
  "$RAW_QUEUE_WORKER_MACHINE_ID" "$RAW_BUILDER_MACHINE_ID" || resources_ok=false
exact_volume_set "$api_volumes" "$RAW_API_VOLUME_ID" || resources_ok=false
exact_volume_set "$reader_volumes" \
  "$RAW_READER_VOLUME_ID" "$RAW_CANARY_VOLUME_ID" || resources_ok=false
exact_volume_set "$indexer_volumes" \
  "$RAW_QUEUE_WORKER_VOLUME_ID" "$RAW_BUILDER_VOLUME_ID" || resources_ok=false
resource_matches "$api_machines" "$RAW_API_MACHINE_ID" "$RAW_API_VOLUME_ID" \
  "$RAW_API_MOUNT_PATH" started || resources_ok=false
resource_matches "$reader_machines" "$RAW_READER_MACHINE_ID" "$RAW_READER_VOLUME_ID" \
  "$RAW_READER_MOUNT_PATH" "$raw_expected_state" || resources_ok=false
resource_matches "$reader_machines" "$RAW_CANARY_MACHINE_ID" "$RAW_CANARY_VOLUME_ID" \
  "$RAW_CANARY_MOUNT_PATH" stopped || resources_ok=false
resource_matches "$indexer_machines" "$RAW_QUEUE_WORKER_MACHINE_ID" \
  "$RAW_QUEUE_WORKER_VOLUME_ID" "$RAW_QUEUE_WORKER_MOUNT_PATH" \
  created,starting,started,stopped || resources_ok=false
resource_matches "$indexer_machines" "$RAW_BUILDER_MACHINE_ID" "$RAW_BUILDER_VOLUME_ID" \
  "$RAW_BUILDER_MOUNT_PATH" stopped || resources_ok=false

if [[ "${DETAILS:-0}" == 1 ]]; then
  printf 'app\tmachine\tstate\tregion\tsize\timage\tvolume\n'
  for pair in \
    "$RAW_API_APP:$api_machines" \
    "$RAW_READER_APP:$reader_machines" \
    "$RAW_QUEUE_WORKER_APP:$indexer_machines"; do
    app="${pair%%:*}"
    machine_file="${pair#*:}"
    jq -r --arg app "$app" '
      .[] | [
        $app,.id,.state,(.region // "-"),
        ([.config.guest.cpu_kind // "", (.config.guest.cpus // "" | tostring),
          (.config.guest.memory_mb // "" | tostring)] | join("/")),
        (.image_ref.registry // .image_ref.repository // "-"),
        ([.config.mounts[]? | "\(.volume)/\(.path)"] |
          if length == 0 then "-" else join(",") end)
      ] | @tsv
    ' "$machine_file"
  done
fi

ok=false
if [[ "$availability_ok" == true && "$resources_ok" == true ]]; then
  ok=true
fi
printf 'apps=3 availability_ok=%s schedule_mode=%s raw_expected_state=%s exact_resources_ok=%s ok=%s\n' \
  "$availability_ok" "$schedule_mode" "$raw_expected_state" "$resources_ok" "$ok"
[[ "$ok" == true ]]
