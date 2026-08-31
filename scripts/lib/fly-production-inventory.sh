#!/usr/bin/env bash

# Stable, non-secret Fly identities and retirement policy. Release commits,
# image digests, private addresses, and credentials deliberately do not belong
# in this inventory.

_fly_production_inventory_error() {
  printf 'fly-production-inventory: %s\n' "$*" >&2
}

fly_production_inventory_validate() {
  local library_directory repository_root inventory_path
  library_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
  repository_root="$(cd -- "$library_directory/../.." && pwd)"
  inventory_path="${FLY_PRODUCTION_INVENTORY_PATH:-$repository_root/config/fly-production-resources.json}"

  if ! command -v jq >/dev/null 2>&1; then
    _fly_production_inventory_error 'missing required command: jq'
    return 1
  fi
  if [[ ! -f "$inventory_path" ]]; then
    _fly_production_inventory_error "inventory does not exist: $inventory_path"
    return 1
  fi
  if ! jq -e '
    def app: type == "string" and test("^[a-z0-9][a-z0-9-]{0,62}$");
    def machine: type == "string" and test("^[0-9a-f]{14}$");
    def volume: type == "string" and test("^vol_[a-z0-9]{16}$");
    def resource:
      type == "object" and
      (keys | sort) == ["app", "machine_id", "mount_path", "volume_id"] and
      (.app | app) and (.machine_id | machine) and (.volume_id | volume);
    def retired_machine:
      type == "object" and
      (keys | sort) == ["app", "machine_id", "successor_app", "volume_id"] and
      (.app | app) and (.machine_id | machine) and (.volume_id | volume) and
      (.successor_app | app);
    def retired_volume:
      type == "object" and (keys | sort) == ["app", "volume_id"] and
      (.app | app) and (.volume_id | volume);
    type == "object" and
    (keys | sort) == ["environment", "resources", "retirement_sets", "schema_version"] and
    .schema_version == 1 and .environment == "production" and
    (.resources | keys | sort) == [
      "raw_api",
      "raw_publication_builder",
      "raw_queue_worker",
      "raw_reader",
      "raw_reader_canary"
    ] and
    (.resources.raw_api | resource) and
    (.resources.raw_reader | resource) and
    (.resources.raw_reader_canary | resource) and
    (.resources.raw_queue_worker | resource) and
    (.resources.raw_publication_builder | resource) and
    .resources.raw_api.mount_path == "/data" and
    .resources.raw_reader.mount_path == "/data/mutable" and
    .resources.raw_reader_canary.mount_path == "/data/mutable" and
    .resources.raw_queue_worker.mount_path == "/var/lib/ytsearch" and
    .resources.raw_publication_builder.mount_path == "/var/lib/ytsearch" and
    .resources.raw_reader.app == .resources.raw_reader_canary.app and
    .resources.raw_queue_worker.app == .resources.raw_publication_builder.app and
    ([.resources.raw_api.app, .resources.raw_reader.app, .resources.raw_queue_worker.app] |
      unique | length) == 3 and
    (.retirement_sets | keys) == ["typesense"] and
    (.retirement_sets.typesense | keys | sort) == ["machines", "reason", "unattached_volumes"] and
    .retirement_sets.typesense.reason == "raw-search-cutover" and
    (.retirement_sets.typesense.machines | length) == 4 and
    (.retirement_sets.typesense.machines | all(retired_machine)) and
    (
      [.resources.raw_api.app, .resources.raw_reader.app, .resources.raw_queue_worker.app] as $successor_apps |
      (.retirement_sets.typesense.machines |
        all(.successor_app as $successor | ($successor_apps | index($successor)) != null))
    ) and
    (.retirement_sets.typesense.unattached_volumes | length) == 3 and
    (.retirement_sets.typesense.unattached_volumes | all(retired_volume)) and
    ([.resources[] | .machine_id] + [.retirement_sets.typesense.machines[].machine_id] | length) ==
      ([.resources[] | .machine_id] + [.retirement_sets.typesense.machines[].machine_id] | unique | length) and
    ([.resources[] | .volume_id] + [.retirement_sets.typesense.machines[].volume_id] +
      [.retirement_sets.typesense.unattached_volumes[].volume_id] | length) ==
      ([.resources[] | .volume_id] + [.retirement_sets.typesense.machines[].volume_id] +
      [.retirement_sets.typesense.unattached_volumes[].volume_id] | unique | length)
  ' "$inventory_path" >/dev/null; then
    _fly_production_inventory_error "invalid inventory: $inventory_path"
    return 1
  fi
  FLY_PRODUCTION_INVENTORY_RESOLVED_PATH="$inventory_path"
}

fly_production_inventory_load_raw_release() {
  local values index
  local -a variable_names
  fly_production_inventory_validate || return 1
  mapfile -t values < <(jq -er '
    .resources.raw_reader_canary.app,
    .resources.raw_reader_canary.machine_id,
    .resources.raw_reader_canary.volume_id,
    .resources.raw_reader_canary.mount_path,
    .resources.raw_publication_builder.app,
    .resources.raw_publication_builder.machine_id,
    .resources.raw_publication_builder.volume_id,
    .resources.raw_publication_builder.mount_path
  ' "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")
  if (( ${#values[@]} != 8 )); then
    _fly_production_inventory_error 'Raw release inventory returned an unexpected value count'
    return 1
  fi

  values+=("${values[0]}" "${values[1]}")
  variable_names=(
    RAW_APP RAW_CANARY_MACHINE_ID RAW_CANARY_VOLUME_ID RAW_CANARY_MOUNT_PATH
    RAW_BUILDER_APP RAW_BUILDER_MACHINE_ID RAW_BUILDER_VOLUME_ID RAW_BUILDER_MOUNT_PATH
    RAW_TARGET_APP RAW_PUBLICATION_TARGET_MACHINE_ID
  )
  for index in "${!variable_names[@]}"; do
    if [[ -v "${variable_names[index]}" \
      && "${!variable_names[index]}" != "${values[index]}" ]]; then
      _fly_production_inventory_error \
        "${variable_names[index]} conflicts with the selected inventory; replace the whole inventory instead"
      return 1
    fi
  done
  for index in "${!variable_names[@]}"; do
    printf -v "${variable_names[index]}" '%s' "${values[index]}"
  done
}

fly_production_inventory_load_raw_runtime() {
  local values index
  local -a variable_names
  fly_production_inventory_validate || return 1
  mapfile -t values < <(jq -er '
    .resources.raw_api.app,
    .resources.raw_api.machine_id,
    .resources.raw_api.volume_id,
    .resources.raw_api.mount_path,
    .resources.raw_reader.app,
    .resources.raw_reader.machine_id,
    .resources.raw_reader.volume_id,
    .resources.raw_reader.mount_path,
    .resources.raw_queue_worker.app,
    .resources.raw_queue_worker.machine_id,
    .resources.raw_queue_worker.volume_id,
    .resources.raw_queue_worker.mount_path
  ' "$FLY_PRODUCTION_INVENTORY_RESOLVED_PATH")
  if (( ${#values[@]} != 12 )); then
    _fly_production_inventory_error 'Raw runtime inventory returned an unexpected value count'
    return 1
  fi

  variable_names=(
    RAW_API_APP RAW_API_MACHINE_ID RAW_API_VOLUME_ID RAW_API_MOUNT_PATH
    RAW_READER_APP RAW_READER_MACHINE_ID RAW_READER_VOLUME_ID RAW_READER_MOUNT_PATH
    RAW_QUEUE_WORKER_APP RAW_QUEUE_WORKER_MACHINE_ID RAW_QUEUE_WORKER_VOLUME_ID
    RAW_QUEUE_WORKER_MOUNT_PATH
  )
  for index in "${!variable_names[@]}"; do
    if [[ -v "${variable_names[index]}" \
      && "${!variable_names[index]}" != "${values[index]}" ]]; then
      _fly_production_inventory_error \
        "${variable_names[index]} conflicts with the selected inventory; replace the whole inventory instead"
      return 1
    fi
  done
  for index in "${!variable_names[@]}"; do
    printf -v "${variable_names[index]}" '%s' "${values[index]}"
  done
}
