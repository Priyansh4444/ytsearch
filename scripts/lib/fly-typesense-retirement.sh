#!/usr/bin/env bash

# Exact-set and per-Machine safety checks shared by Typesense retirement tools.
# The caller must validate the production inventory before using these helpers.

_fly_typesense_retirement_error() {
  printf 'typesense-retirement: %s\n' "$*" >&2
}

fly_typesense_retirement_apps() {
  jq -r '[
    .retirement_sets.typesense.machines[].app,
    .retirement_sets.typesense.unattached_volumes[].app
  ] | unique[]' "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH"
}

fly_typesense_verify_exact_app_set() {
  local app="$1"
  local expected_machines expected_volumes actual_machines actual_volumes
  expected_machines="$(jq -r --arg app "$app" '[
    .retirement_sets.typesense.machines[] | select(.app == $app) | .machine_id
  ] | sort | join(" ")' "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")"
  expected_volumes="$(jq -r --arg app "$app" '[
    (.retirement_sets.typesense.machines[] | select(.app == $app) | .volume_id),
    (.retirement_sets.typesense.unattached_volumes[] | select(.app == $app) | .volume_id)
  ] | sort | join(" ")' "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")"
  actual_machines="$(flyctl machine list --app "$app" --json |
    jq -r '.[].id' | sort | tr '\n' ' ' | sed 's/ $//')"
  actual_volumes="$(flyctl volumes list --app "$app" --json |
    jq -r '.[].id' | sort | tr '\n' ' ' | sed 's/ $//')"
  if [[ "$actual_machines" != "$expected_machines" ]]; then
    _fly_typesense_retirement_error \
      "$app Machine set changed; expected '$expected_machines', got '$actual_machines'"
    return 1
  fi
  if [[ "$actual_volumes" != "$expected_volumes" ]]; then
    _fly_typesense_retirement_error \
      "$app volume set changed; expected '$expected_volumes', got '$actual_volumes'"
    return 1
  fi
}

fly_typesense_machine_json() {
  local app="$1"
  local machine="$2"
  flyctl machine list --app "$app" --json | jq -ce --arg id "$machine" '
    [.[] | select(.id == $id)] |
    if length == 1 then .[0] else error("exact retirement Machine not found") end
  '
}

fly_typesense_verify_machine_base() {
  local app="$1"
  local machine="$2"
  local volume="$3"
  local machine_json
  machine_json="$(fly_typesense_machine_json "$app" "$machine")"
  if ! jq -e --arg volume "$volume" '
    .state == "stopped" and
    ([.config.mounts[]?.volume] == [$volume])
  ' <<<"$machine_json" >/dev/null; then
    _fly_typesense_retirement_error \
      "$app/$machine is not the expected stopped Machine mounted to $volume"
    return 1
  fi
}

fly_typesense_verify_machine_deprecated() {
  local app="$1"
  local machine="$2"
  local volume="$3"
  local successor="$4"
  local reason="$5"
  local machine_json
  machine_json="$(fly_typesense_machine_json "$app" "$machine")"
  if ! jq -e --arg volume "$volume" --arg successor "$successor" --arg reason "$reason" '
    .state == "stopped" and
    .config.metadata.lifecycle == "deprecated" and
    .config.metadata.deprecated_reason == $reason and
    .config.metadata.successor == $successor and
    ([.config.mounts[]?.volume] == [$volume])
  ' <<<"$machine_json" >/dev/null; then
    _fly_typesense_retirement_error \
      "$app/$machine is not the expected stopped deprecated Machine mounted to $volume"
    return 1
  fi
}

fly_typesense_verify_exact_sets() {
  local app
  while IFS= read -r app; do
    fly_typesense_verify_exact_app_set "$app" || return 1
  done < <(fly_typesense_retirement_apps)
}

fly_typesense_verify_all_base() {
  local app machine volume
  while IFS=$'\t' read -r app machine volume; do
    fly_typesense_verify_machine_base "$app" "$machine" "$volume" || return 1
  done < <(jq -er '.retirement_sets.typesense.machines[] |
    [.app, .machine_id, .volume_id] | @tsv' \
    "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")
}

fly_typesense_verify_all_deprecated() {
  local reason app machine volume successor
  reason="$(jq -er '.retirement_sets.typesense.reason' \
    "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")"
  while IFS=$'\t' read -r app machine volume successor; do
    fly_typesense_verify_machine_deprecated \
      "$app" "$machine" "$volume" "$successor" "$reason" || return 1
  done < <(jq -er '.retirement_sets.typesense.machines[] |
    [.app, .machine_id, .volume_id, .successor_app] | @tsv' \
    "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")
}
