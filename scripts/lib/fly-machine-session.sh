#!/usr/bin/env bash

# A caller-owned Fly Machine lease session.
#
# Public functions take the session directory as their first argument. Callers
# own cleanup and traps. A failed stop deliberately leaves the nonce in place,
# and release refuses to run until a leased GET confirms state=stopped.

_fly_machine_session_error() {
  printf 'fly-machine-session: %s\n' "$*" >&2
}

_fly_machine_session_api_origin_is_valid() {
  local origin="$1"
  local authority host port label
  local -a labels

  [[ "$origin" =~ ^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?$ ]] || return 1
  authority="${origin#https://}"
  host="$authority"
  port=""
  if [[ "$authority" == *:* ]]; then
    host="${authority%:*}"
    port="${authority##*:}"
  fi
  if [[ -n "$port" ]] && (( 10#$port < 1 || 10#$port > 65535 )); then
    return 1
  fi
  if (( ${#host} > 253 )) || [[ "$host" == .* || "$host" == *. || "$host" == *..* ]]; then
    return 1
  fi
  IFS=. read -r -a labels <<< "$host"
  for label in "${labels[@]}"; do
    [[ "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9-]{0,61}[A-Za-z0-9])?$ ]] || return 1
  done
}

fly_machine_session_default_recovery_root() {
  local state_root="${XDG_STATE_HOME:-${TMPDIR:-/tmp}}"
  printf '%s/ytsearch/fly-machine-recovery\n' "${state_root%/}"
}

_fly_machine_session_read_line() {
  local path="$1"
  local value
  IFS= read -r value < "$path" || {
    _fly_machine_session_error "could not read $path"
    return 1
  }
  if [[ -z "$value" || "$value" == *$'\n'* || "$value" == *$'\r'* ]]; then
    _fly_machine_session_error "invalid single-line value in $path"
    return 1
  fi
  printf '%s' "$value"
}

_fly_machine_session_require() {
  local directory="$1"
  local mode
  if [[ ! -d "$directory" || -L "$directory" ]]; then
    _fly_machine_session_error "session directory is missing or is a symlink: $directory"
    return 1
  fi
  mode="$(stat -c '%a' "$directory")" || return 1
  if [[ "$mode" != 700 ]]; then
    _fly_machine_session_error "session directory must have mode 700: $directory"
    return 1
  fi
  local required
  for required in app machine-id api-host auth.header; do
    if [[ ! -f "$directory/$required" || -L "$directory/$required" ]]; then
      _fly_machine_session_error "session file is missing or is a symlink: $directory/$required"
      return 1
    fi
  done
  if [[ "$(stat -c '%a' "$directory/auth.header")" != 600 ]]; then
    _fly_machine_session_error "authorization header must have mode 600"
    return 1
  fi
}

_fly_machine_session_require_lease() {
  local directory="$1"
  _fly_machine_session_require "$directory" || return 1
  if [[ ! -f "$directory/lease.header" || -L "$directory/lease.header" ]]; then
    _fly_machine_session_error "session does not hold a usable lease"
    return 1
  fi
  if [[ "$(stat -c '%a' "$directory/lease.header")" != 600 ]]; then
    _fly_machine_session_error "lease header must have mode 600"
    return 1
  fi
}

_fly_machine_session_url() {
  local directory="$1"
  local app machine_id api_host
  app="$(_fly_machine_session_read_line "$directory/app")" || return 1
  machine_id="$(_fly_machine_session_read_line "$directory/machine-id")" || return 1
  api_host="$(_fly_machine_session_read_line "$directory/api-host")" || return 1
  printf '%s/v1/apps/%s/machines/%s' "$api_host" "$app" "$machine_id"
}

_fly_machine_session_refresh_auth() {
  local directory="$1"
  local token temporary_header

  # Ignore a process-scoped token here: it is the credential the API just
  # rejected. Ask flyctl for the current persisted login instead.
  token="$(env -u FLY_API_TOKEN flyctl auth token)" || {
    _fly_machine_session_error "could not refresh the Fly API token"
    return 1
  }
  if [[ -z "$token" || "$(printf '%s' "$token" | tr -d '\r\n')" != "$token" ]]; then
    _fly_machine_session_error "refreshed Fly API token must be one nonempty line"
    return 1
  fi

  temporary_header="$(mktemp "$directory/auth.header.refresh.XXXXXX")" || return 1
  chmod 600 "$temporary_header"
  printf 'Authorization: Bearer %s\n' "$token" > "$temporary_header"
  mv -f -- "$temporary_header" "$directory/auth.header"
}

_fly_machine_session_request() {
  local directory="$1"
  local method="$2"
  local suffix="$3"
  local output="$4"
  local request_file="${5:-}"
  local use_lease="${6:-1}"
  local url status attempt
  local connect_timeout="${FLY_MACHINE_SESSION_CONNECT_TIMEOUT_SECONDS:-5}"
  local request_timeout="${FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS:-30}"
  local curl_bin="${FLY_MACHINE_SESSION_CURL_BIN:-curl}"
  local -a arguments

  if [[ "$use_lease" == 1 ]]; then
    _fly_machine_session_require_lease "$directory" || return 1
  else
    _fly_machine_session_require "$directory" || return 1
  fi
  if [[ ! "$connect_timeout" =~ ^[1-9][0-9]*$ \
    || ! "$request_timeout" =~ ^[1-9][0-9]*$ ]]; then
    _fly_machine_session_error 'curl timeout settings must be positive integer seconds'
    return 2
  fi
  if [[ "$curl_bin" == */* ]]; then
    if [[ ! -x "$curl_bin" ]]; then
      _fly_machine_session_error "curl executable is unavailable: $curl_bin"
      return 2
    fi
  elif ! command -v "$curl_bin" >/dev/null 2>&1; then
    _fly_machine_session_error "curl executable is unavailable: $curl_bin"
    return 2
  fi
  url="$(_fly_machine_session_url "$directory")$suffix" || return 1
  : > "$output"
  chmod 600 "$output"
  arguments=(
    --silent
    --show-error
    --connect-timeout "$connect_timeout"
    --max-time "$request_timeout"
    --output "$output"
    --write-out '%{http_code}'
    --request "$method"
    --header "@$directory/auth.header"
  )
  if [[ "$use_lease" == 1 ]]; then
    arguments+=(--header "@$directory/lease.header")
  fi
  if [[ -n "$request_file" ]]; then
    arguments+=(--header 'Content-Type: application/json' --data-binary "@$request_file")
  fi
  for attempt in 1 2; do
    if ! status="$("$curl_bin" "${arguments[@]}" "$url")"; then
      printf '000'
      return 70
    fi
    if [[ ! "$status" =~ ^[0-9]{3}$ ]]; then
      _fly_machine_session_error "curl returned an invalid HTTP status"
      return 1
    fi
    if (( attempt == 1 )) && [[ "$status" == 401 || "$status" == 403 ]]; then
      _fly_machine_session_refresh_auth "$directory" || return 1
      # Reads are safe to replay. Mutating callers receive the original status
      # and resolve state before deciding whether to issue another request.
      [[ "$method" == GET || "$method" == HEAD ]] && continue
    fi
    break
  done
  printf '%s' "$status"
}

_fly_machine_session_validate_machine() {
  local directory="$1"
  local response="$2"
  local machine_id
  machine_id="$(_fly_machine_session_read_line "$directory/machine-id")" || return 1
  jq -e --arg id "$machine_id" '
    type == "object"
    and .id == $id
    and (.state | type) == "string"
    and (.instance_id | type) == "string"
    and (.instance_id | length) > 0
    and (.config | type) == "object"
  ' "$response" >/dev/null
}

_fly_machine_session_get_to() {
  local directory="$1"
  local output="$2"
  local status request_status=0
  status="$(_fly_machine_session_request "$directory" GET '' "$output")" || request_status=$?
  if (( request_status != 0 )) || [[ "$status" != 200 ]]; then
    _fly_machine_session_error "leased Machine GET failed with HTTP $status"
    return 1
  fi
  if ! _fly_machine_session_validate_machine "$directory" "$output"; then
    _fly_machine_session_error "leased Machine GET returned malformed data"
    return 1
  fi
}

fly_machine_session_create() {
  if [[ "$#" -ne 3 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_create DIRECTORY APP MACHINE_ID'
    return 2
  fi
  local directory="$1"
  local app="$2"
  local machine_id="$3"
  local api_host="${FLY_API_HOSTNAME:-https://api.machines.dev}"
  local token="${FLY_API_TOKEN:-}"

  if [[ ! "$app" =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]]; then
    _fly_machine_session_error "invalid Fly app name: $app"
    return 2
  fi
  if [[ ! "$machine_id" =~ ^[0-9a-f]{14}$ ]]; then
    _fly_machine_session_error "Machine ID must be exactly 14 lowercase hexadecimal characters"
    return 2
  fi
  if ! _fly_machine_session_api_origin_is_valid "$api_host"; then
    _fly_machine_session_error \
      "FLY_API_HOSTNAME must be one exact HTTPS origin without userinfo, path, query, or fragment"
    return 2
  fi
  if [[ -e "$directory" && ( ! -d "$directory" || -L "$directory" ) ]]; then
    _fly_machine_session_error "session path is not a directory: $directory"
    return 1
  fi
  mkdir -p -- "$directory"
  chmod 700 "$directory"
  if [[ -n "$(find "$directory" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
    _fly_machine_session_error "session directory must be empty: $directory"
    return 1
  fi
  if [[ -z "$token" ]]; then
    token="$(flyctl auth token)" || {
      _fly_machine_session_error "could not obtain a Fly API token"
      return 1
    }
  fi
  if [[ -z "$token" || "$(printf '%s' "$token" | tr -d '\r\n')" != "$token" ]]; then
    _fly_machine_session_error "Fly API token must be one nonempty line"
    return 1
  fi

  (
    umask 077
    printf '%s\n' "$app" > "$directory/app"
    printf '%s\n' "$machine_id" > "$directory/machine-id"
    printf '%s\n' "$api_host" > "$directory/api-host"
    printf 'Authorization: Bearer %s\n' "$token" > "$directory/auth.header"
    chmod 600 "$directory/app" "$directory/machine-id" "$directory/api-host" \
      "$directory/auth.header"
  )
}

fly_machine_app_secrets_version() {
  if [[ "$#" -ne 1 ]]; then
    _fly_machine_session_error 'usage: fly_machine_app_secrets_version APP'
    return 2
  fi
  local app="$1"
  local config_file="${FLY_CONFIG_FILE:-${FLY_CONFIG_DIR:-$HOME/.fly}/config.yml}"
  local version
  if [[ ! "$app" =~ ^[a-z0-9][a-z0-9-]{0,62}$ ]]; then
    _fly_machine_session_error "invalid Fly app name: $app"
    return 2
  fi
  if ! flyctl secrets sync --stage --app "$app" >/dev/null; then
    _fly_machine_session_error "could not synchronize staged secrets for $app"
    return 1
  fi
  if [[ ! -r "$config_file" ]]; then
    _fly_machine_session_error "Fly config is not readable: $config_file"
    return 1
  fi
  version="$(awk -v app="$app" '
    /^app_secrets_minvers:/ { in_minvers = 1; next }
    in_minvers && /^[^[:space:]]/ { exit }
    in_minvers {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      key = line
      sub(/:.*/, "", key)
      if (key == app) {
        sub(/^[^:]*:[[:space:]]*/, "", line)
        print line
        exit
      }
    }
  ' "$config_file")"
  if [[ ! "$version" =~ ^(0|[1-9][0-9]*)$ ]]; then
    _fly_machine_session_error "Fly did not persist a numeric staged-secrets version for $app"
    return 1
  fi
  printf '%s\n' "$version"
}

fly_machine_session_acquire() {
  if [[ "$#" -ne 3 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_acquire DIRECTORY DESCRIPTION TTL_SECONDS'
    return 2
  fi
  local directory="$1"
  local description="$2"
  local ttl="$3"
  local request="$directory/lease-request.json"
  local response="$directory/lease-response.json"
  local next_header="$directory/lease.header.next"
  local status request_status=0 nonce

  _fly_machine_session_require "$directory" || return 1
  if [[ -e "$directory/lease.header" || -e "$directory/lease-ambiguous" ]]; then
    _fly_machine_session_error "session already has lease state"
    return 1
  fi
  if [[ -z "$description" || "$description" == *$'\n'* || "$description" == *$'\r'* ]]; then
    _fly_machine_session_error "lease description must be one nonempty line"
    return 2
  fi
  if [[ ! "$ttl" =~ ^[1-9][0-9]*$ ]]; then
    _fly_machine_session_error "lease TTL must be a positive integer"
    return 2
  fi
  jq -n --arg description "$description" --argjson ttl "$ttl" \
    '{description:$description, ttl:$ttl}' > "$request"
  chmod 600 "$request"
  status="$(_fly_machine_session_request "$directory" POST '/lease' "$response" "$request" 0)" \
    || request_status=$?
  if (( request_status != 0 )); then
    : > "$directory/lease-ambiguous"
    chmod 600 "$directory/lease-ambiguous"
    _fly_machine_session_error "lease acquisition response was lost"
    return 1
  fi
  if [[ "$status" != 201 ]]; then
    _fly_machine_session_error "lease acquisition failed with HTTP $status"
    return 1
  fi
  if ! nonce="$(jq -er '.data.nonce | select(type == "string" and length > 0)' "$response")" \
    || [[ ! "$nonce" =~ ^[A-Za-z0-9._~:/+=-]+$ ]]; then
    : > "$directory/lease-ambiguous"
    chmod 600 "$directory/lease-ambiguous"
    _fly_machine_session_error "lease response did not contain a safe nonce"
    return 1
  fi
  printf 'fly-machine-lease-nonce: %s\n' "$nonce" > "$next_header"
  chmod 600 "$next_header"
  mv "$next_header" "$directory/lease.header"
}

fly_machine_session_lease_response() {
  if [[ "$#" -ne 2 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_lease_response DIRECTORY OUTPUT_JSON'
    return 2
  fi
  local directory="$1"
  local output="$2"
  local response="$directory/lease-response.json"
  _fly_machine_session_require_lease "$directory" || return 1
  if [[ ! -f "$response" || -L "$response" ]] \
    || ! jq -e '.data.nonce | type == "string" and length > 0' "$response" >/dev/null; then
    _fly_machine_session_error "preserved lease response is missing or malformed"
    return 1
  fi
  cp -- "$response" "$output"
  chmod 600 "$output"
}

fly_machine_session_get() {
  if [[ "$#" -ne 2 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_get DIRECTORY OUTPUT_JSON'
    return 2
  fi
  local directory="$1"
  local output="$2"
  local response="$directory/machine-response.json"
  _fly_machine_session_get_to "$directory" "$response" || return 1
  cp -- "$response" "$output"
  chmod 600 "$output"
}

fly_machine_session_update() {
  if [[ "$#" -lt 3 || "$#" -gt 4 ]]; then
    _fly_machine_session_error \
      'usage: fly_machine_session_update DIRECTORY FULL_CONFIG_JSON MIN_SECRETS_VERSION [OUTPUT_JSON]'
    return 2
  fi
  local directory="$1"
  local config="$2"
  local min_secrets_version="$3"
  local output="${4:-}"
  local current="$directory/machine-before-update.json"
  local request="$directory/update-request.json"
  local response="$directory/update-response.json"
  local current_version status request_status=0

  _fly_machine_session_require_lease "$directory" || return 1
  if [[ ! -f "$config" || -L "$config" ]] || ! jq -e 'type == "object"' "$config" >/dev/null; then
    _fly_machine_session_error "full Machine config must be a JSON object in a regular file"
    return 2
  fi
  if [[ ! "$min_secrets_version" =~ ^(0|[1-9][0-9]*)$ ]]; then
    _fly_machine_session_error "minimum secrets version must be a JSON integer"
    return 2
  fi
  _fly_machine_session_get_to "$directory" "$current" || return 1
  if ! jq -e '
    .host_status == "ok"
    and ((.incomplete_config // false) | not)
  ' "$current" >/dev/null; then
    _fly_machine_session_error "refusing an entire-config update from an incomplete Machine response"
    return 1
  fi
  current_version="$(jq -er '.instance_id' "$current")" || return 1
  jq -n \
    --slurpfile config "$config" \
    --arg current_version "$current_version" \
    --argjson min_secrets_version "$min_secrets_version" '
      {
        config: $config[0],
        current_version: $current_version,
        skip_launch: true,
        min_secrets_version: $min_secrets_version
      }
    ' > "$request"
  chmod 600 "$request"
  status="$(_fly_machine_session_request "$directory" POST '' "$response" "$request")" \
    || request_status=$?
  if (( request_status != 0 )) || [[ "$status" != 200 ]]; then
    _fly_machine_session_error "entire-config Machine update failed with HTTP $status"
    return 1
  fi
  if ! _fly_machine_session_validate_machine "$directory" "$response"; then
    _fly_machine_session_error "Machine update returned malformed data"
    return 1
  fi
  if [[ -n "$output" ]]; then
    cp -- "$response" "$output"
    chmod 600 "$output"
  fi
}

fly_machine_session_start() {
  if [[ "$#" -ne 1 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_start DIRECTORY'
    return 2
  fi
  local directory="$1"
  local response="$directory/start-response.json"
  local current="$directory/machine-after-start.json"
  local attempts="${FLY_MACHINE_SESSION_START_POLL_ATTEMPTS:-30}"
  local interval="${FLY_MACHINE_SESSION_START_POLL_INTERVAL_SECONDS:-1}"
  local status request_status=0 state poll

  _fly_machine_session_require_lease "$directory" || return 1
  if [[ ! "$attempts" =~ ^[1-9][0-9]*$ || ! "$interval" =~ ^[0-9]+$ ]]; then
    _fly_machine_session_error "start polling settings must be integers"
    return 2
  fi
  : > "$directory/start.intent"
  chmod 600 "$directory/start.intent"
  if ! _fly_machine_session_get_to "$directory" "$current"; then
    _fly_machine_session_error "refusing to start without an authoritative Machine read"
    return 1
  fi
  state="$(jq -r '.state' "$current")"
  if [[ "$state" == started ]]; then
    return 0
  fi
  if [[ "$state" != stopped && "$state" != created ]]; then
    _fly_machine_session_error "refusing to start Machine from state=$state"
    return 1
  fi
  status="$(_fly_machine_session_request "$directory" POST '/start' "$response")" \
    || request_status=$?
  if (( request_status == 0 )) && [[ "$status" != 200 && "$status" != 202 ]]; then
    _fly_machine_session_error "Machine start returned HTTP $status; checking authoritative state"
  fi
  for ((poll = 1; poll <= attempts; poll += 1)); do
    if _fly_machine_session_get_to "$directory" "$current" \
      && [[ "$(jq -r '.state' "$current")" == started ]]; then
      return 0
    fi
    if (( poll < attempts && interval > 0 )); then
      sleep "$interval"
    fi
  done
  _fly_machine_session_error "Machine start could not be confirmed; start intent remains recorded"
  return 1
}

fly_machine_session_stop() {
  if [[ "$#" -lt 1 || "$#" -gt 4 ]]; then
    _fly_machine_session_error \
      'usage: fly_machine_session_stop DIRECTORY [RETRIES [POLL_ATTEMPTS [INTERVAL_SECONDS]]]'
    return 2
  fi
  local directory="$1"
  local retries="${2:-3}"
  local poll_attempts="${3:-10}"
  local interval="${4:-1}"
  local response="$directory/stop-response.json"
  local current="$directory/machine-after-stop.json"
  local status request_status state attempt poll

  _fly_machine_session_require_lease "$directory" || return 1
  if [[ ! "$retries" =~ ^[1-9][0-9]*$ || ! "$poll_attempts" =~ ^[1-9][0-9]*$ \
    || ! "$interval" =~ ^[0-9]+$ ]]; then
    _fly_machine_session_error "stop retry settings must be integers"
    return 2
  fi
  for ((attempt = 1; attempt <= retries; attempt += 1)); do
    if _fly_machine_session_get_to "$directory" "$current"; then
      state="$(jq -r '.state' "$current")"
      if [[ "$state" == stopped ]]; then
        rm -f -- "$directory/start.intent"
        return 0
      fi
    fi
    request_status=0
    status="$(_fly_machine_session_request "$directory" POST '/stop' "$response")" \
      || request_status=$?
    if (( request_status != 0 )) || [[ "$status" != 200 && "$status" != 202 ]]; then
      _fly_machine_session_error "Machine stop attempt $attempt returned HTTP $status"
    fi
    for ((poll = 1; poll <= poll_attempts; poll += 1)); do
      if _fly_machine_session_get_to "$directory" "$current" \
        && [[ "$(jq -r '.state' "$current")" == stopped ]]; then
        rm -f -- "$directory/start.intent"
        return 0
      fi
      if (( poll < poll_attempts && interval > 0 )); then
        sleep "$interval"
      fi
    done
  done
  _fly_machine_session_error "Machine stop could not be confirmed; lease remains held"
  return 1
}

fly_machine_session_release() {
  if [[ "$#" -ne 1 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_release DIRECTORY'
    return 2
  fi
  local directory="$1"
  local current="$directory/machine-before-release.json"
  local response="$directory/release-response.json"
  local status request_status=0

  _fly_machine_session_require_lease "$directory" || return 1
  if ! _fly_machine_session_get_to "$directory" "$current" \
    || [[ "$(jq -r '.state' "$current")" != stopped ]]; then
    _fly_machine_session_error "refusing to release lease without authoritative stopped state"
    return 1
  fi
  status="$(_fly_machine_session_request "$directory" DELETE '/lease' "$response")" \
    || request_status=$?
  if (( request_status != 0 )) || [[ "$status" != 200 && "$status" != 204 ]]; then
    _fly_machine_session_error "lease release failed with HTTP $status; nonce remains recorded"
    return 1
  fi
  rm -f -- "$directory/lease.header" "$directory/start.intent"
}

fly_machine_session_preserve() {
  if [[ "$#" -ne 2 ]]; then
    _fly_machine_session_error 'usage: fly_machine_session_preserve DIRECTORY DESTINATION'
    return 2
  fi
  local directory="$1"
  local destination="$2"
  local destination_parent
  _fly_machine_session_require_lease "$directory" || return 1
  if [[ -e "$destination" || -L "$destination" ]]; then
    _fly_machine_session_error "recovery destination already exists: $destination"
    return 1
  fi
  destination_parent="$(dirname -- "$destination")"
  mkdir -p -- "$destination_parent"
  mv -- "$directory" "$destination"
  chmod 700 "$destination"
  printf '%s\n' "$destination"
}
