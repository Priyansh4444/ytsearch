#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(dirname -- "$(readlink -f -- "${BASH_SOURCE[0]}")")"
# shellcheck source=scripts/lib/test-fly-fakes.sh
source "$SCRIPT_DIR/lib/test-fly-fakes.sh"

if [[ "${0##*/}" == curl ]]; then
  fake_curl_parse "$@"
  case "$method:$url" in
    POST:*/lease)
      [[ -n "$request" ]]
      jq -e '.description == "test session" and .ttl == 120' "$request" >/dev/null
      case "$FAKE_MODE" in
        lease-conflict)
          printf '{"error":"conflict"}\n' > "$output"
          printf 409
          ;;
        malformed-lease)
          printf '{"data":{}}\n' > "$output"
          printf 201
          ;;
        *)
          printf '{"data":{"nonce":"test-lease-nonce","expires_at":4102444800}}\n' > "$output"
          printf 201
          ;;
      esac
      ;;
    GET:*/machines/0123456789abcd)
      if [[ "$FAKE_MODE" == expired-auth ]] \
        && grep -Fx 'Authorization: Bearer test-secret-token' "$auth_header" >/dev/null; then
        printf '{"error":"expired token"}\n' > "$output"
        printf 403
      elif [[ "$FAKE_MODE" == malformed-get ]]; then
        printf '[]\n' > "$output"
        printf 200
      else
        cp "$FAKE_MACHINE_STATE" "$output"
        printf 200
      fi
      ;;
    POST:*/machines/0123456789abcd/start)
      [[ -f "$FAKE_SESSION_DIRECTORY/start.intent" ]]
      jq '.state = "started"' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
      mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
      cp "$FAKE_MACHINE_STATE" "$output"
      if [[ "$FAKE_MODE" == start-response-loss ]]; then
        exit 7
      fi
      printf 200
      ;;
    POST:*/machines/0123456789abcd/stop)
      stop_count="$(cat "$FAKE_STOP_COUNT_FILE")"
      printf '%s\n' "$(( stop_count + 1 ))" > "$FAKE_STOP_COUNT_FILE"
      if [[ "$FAKE_MODE" == stop-transport-timeout ]]; then
        exit 28
      elif [[ "$FAKE_MODE" == stop-failure ]] \
        || [[ "$FAKE_MODE" == stop-first-failure && "$stop_count" == 0 ]]; then
        printf '{"error":"stop failed"}\n' > "$output"
        printf 500
      else
        jq '.state = "stopped"' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
        mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
        cp "$FAKE_MACHINE_STATE" "$output"
        printf 200
      fi
      ;;
    POST:*/machines/0123456789abcd)
      [[ -n "$request" ]]
      jq -e '
        .current_version == "version-1"
        and .skip_launch == true
        and (.min_secrets_version | type) == "number"
        and .min_secrets_version == 104052994
        and .config == {image:"registry.invalid/test:one",restart:{policy:"no"}}
      ' "$request" >/dev/null
      cp "$request" "$FAKE_UPDATE_CAPTURE"
      if [[ "$FAKE_MODE" == expired-auth-update ]] \
        && grep -Fx 'Authorization: Bearer test-secret-token' "$auth_header" >/dev/null; then
        printf '{"error":"expired token"}\n' > "$output"
        printf 403
      elif [[ "$FAKE_MODE" == update-conflict ]]; then
        printf '{"error":"stale version"}\n' > "$output"
        printf 409
      else
        jq --slurpfile request "$request" '
          .config = $request[0].config
          | .instance_id = "version-2"
          | .state = "created"
        ' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
        mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
        cp "$FAKE_MACHINE_STATE" "$output"
        printf 200
      fi
      ;;
    DELETE:*/lease)
      if [[ "$FAKE_MODE" == release-failure ]]; then
        printf '{"error":"release failed"}\n' > "$output"
        printf 500
      else
        printf '{}\n' > "$output"
        printf 204
      fi
      ;;
    *)
      printf 'unexpected fake curl request: %s %s\n' "$method" "$url" >&2
      exit 92
      ;;
  esac
  exit 0
fi

if [[ "${0##*/}" == flyctl ]]; then
  fake_flyctl "$@"
  exit 0
fi

SCRIPT_PATH="$SCRIPT_DIR/$(basename -- "${BASH_SOURCE[0]}")"
# shellcheck source=scripts/lib/fly-machine-session.sh
source "$SCRIPT_DIR/lib/fly-machine-session.sh"

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
mkdir -p "$temporary_directory/bin"
ln -s "$SCRIPT_PATH" "$temporary_directory/bin/curl"
ln -s "$SCRIPT_PATH" "$temporary_directory/bin/flyctl"
export PATH="$temporary_directory/bin:$PATH"
export FLY_MACHINE_SESSION_CURL_BIN="$temporary_directory/bin/curl"
export FLY_API_TOKEN=test-secret-token
export FLY_API_HOSTNAME=https://api.test
export FAKE_FLY_APP=test-app

invalid_origin_number=0
assert_invalid_api_origin() {
  local origin="$1"
  local session="$temporary_directory/invalid-origin-$invalid_origin_number"
  invalid_origin_number=$(( invalid_origin_number + 1 ))
  if FLY_API_HOSTNAME="$origin" \
    fly_machine_session_create "$session" test-app 0123456789abcd >/dev/null 2>&1; then
    printf 'unsafe Fly API origin must be rejected: %s\n' "$origin" >&2
    exit 1
  fi
  [[ ! -e "$session/auth.header" ]]
}

assert_invalid_api_origin 'https://api.machines.dev@evil.example'
assert_invalid_api_origin 'http://api.test'
assert_invalid_api_origin 'https://api.test/'
assert_invalid_api_origin 'https://api.test/path'
assert_invalid_api_origin 'https://api.test?query=1'
assert_invalid_api_origin 'https://api.test#fragment'
assert_invalid_api_origin 'https://api.test:0'
assert_invalid_api_origin 'https://api.test:65536'

valid_port_session="$temporary_directory/valid-port-origin"
FLY_API_HOSTNAME=https://api.test:8443 \
  fly_machine_session_create "$valid_port_session" test-app 0123456789abcd
grep -Fx 'https://api.test:8443' "$valid_port_session/api-host" >/dev/null

new_session() {
  local name="$1"
  FAKE_SESSION_DIRECTORY="$temporary_directory/$name"
  FAKE_MACHINE_STATE="$temporary_directory/$name-machine.json"
  FAKE_CURL_LOG="$temporary_directory/$name-curl.log"
  FAKE_UPDATE_CAPTURE="$temporary_directory/$name-update.json"
  FAKE_STOP_COUNT_FILE="$temporary_directory/$name-stop-count"
  FAKE_FLY_LOG="$temporary_directory/$name-fly.log"
  FAKE_MODE=normal
  export FAKE_SESSION_DIRECTORY FAKE_MACHINE_STATE FAKE_CURL_LOG FAKE_UPDATE_CAPTURE \
    FAKE_STOP_COUNT_FILE FAKE_FLY_LOG FAKE_MODE
  printf '%s\n' '{
    "id":"0123456789abcd",
    "instance_id":"version-1",
    "state":"stopped",
    "host_status":"ok",
    "incomplete_config":false,
    "config":{"image":"registry.invalid/original:one"}
  }' > "$FAKE_MACHINE_STATE"
  : > "$FAKE_CURL_LOG"
  printf '0\n' > "$FAKE_STOP_COUNT_FILE"
  : > "$FAKE_FLY_LOG"
  fly_machine_session_create "$FAKE_SESSION_DIRECTORY" test-app 0123456789abcd
  [[ "$(stat -c '%a' "$FAKE_SESSION_DIRECTORY")" == 700 ]]
  [[ "$(stat -c '%a' "$FAKE_SESSION_DIRECTORY/auth.header")" == 600 ]]
}

acquire_session() {
  fly_machine_session_acquire "$FAKE_SESSION_DIRECTORY" 'test session' 120
  [[ "$(stat -c '%a' "$FAKE_SESSION_DIRECTORY/lease.header")" == 600 ]]
}

new_session success
acquire_session
fly_machine_session_lease_response "$FAKE_SESSION_DIRECTORY" \
  "$temporary_directory/lease-copy.json"
jq -e '.data.expires_at == 4102444800' "$temporary_directory/lease-copy.json" >/dev/null
fly_machine_session_get "$FAKE_SESSION_DIRECTORY" "$temporary_directory/get.json"
jq -e '.id == "0123456789abcd" and .state == "stopped"' "$temporary_directory/get.json" >/dev/null
printf '%s\n' '{"image":"registry.invalid/test:one","restart":{"policy":"no"}}' \
  > "$temporary_directory/config.json"
fly_machine_session_update "$FAKE_SESSION_DIRECTORY" "$temporary_directory/config.json" \
  104052994 "$temporary_directory/updated.json"
jq -e '
  .current_version == "version-1"
  and .skip_launch == true
  and (.min_secrets_version | type) == "number"
  and .config.image == "registry.invalid/test:one"
' "$FAKE_UPDATE_CAPTURE" >/dev/null
export FAKE_MODE=start-response-loss
FLY_MACHINE_SESSION_START_POLL_ATTEMPTS=1 \
FLY_MACHINE_SESSION_START_POLL_INTERVAL_SECONDS=0 \
  fly_machine_session_start "$FAKE_SESSION_DIRECTORY"
[[ -f "$FAKE_SESSION_DIRECTORY/start.intent" ]]
jq -e '.state == "started"' "$FAKE_MACHINE_STATE" >/dev/null
fly_machine_session_start "$FAKE_SESSION_DIRECTORY"
[[ "$(grep -c 'POST .*machines/0123456789abcd/start' "$FAKE_CURL_LOG")" == 1 ]]
export FAKE_MODE=normal
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 1 1 0
[[ ! -e "$FAKE_SESSION_DIRECTORY/start.intent" ]]
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 1 1 0
[[ "$(grep -c 'POST .*machines/0123456789abcd/stop' "$FAKE_CURL_LOG")" == 1 ]]
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"
[[ ! -e "$FAKE_SESSION_DIRECTORY/lease.header" ]]

cat > "$temporary_directory/fly-config.yml" <<'YAML'
app_secrets_minvers:
  test-app: 104052994
YAML
FLY_CONFIG_FILE="$temporary_directory/fly-config.yml" \
  fly_machine_app_secrets_version test-app > "$temporary_directory/secrets-version"
[[ "$(cat "$temporary_directory/secrets-version")" == 104052994 ]]
grep -Fx 'secrets sync --stage --app test-app' "$FAKE_FLY_LOG" >/dev/null

new_session malformed-lease
export FAKE_MODE=malformed-lease
expect_failure 'malformed lease response must fail' \
  fly_machine_session_acquire "$FAKE_SESSION_DIRECTORY" 'test session' 120
[[ -f "$FAKE_SESSION_DIRECTORY/lease-ambiguous" ]]
[[ ! -e "$FAKE_SESSION_DIRECTORY/lease.header" ]]

new_session lease-conflict
export FAKE_MODE=lease-conflict
expect_failure 'lease conflict must fail' \
  fly_machine_session_acquire "$FAKE_SESSION_DIRECTORY" 'test session' 120
[[ ! -e "$FAKE_SESSION_DIRECTORY/lease.header" ]]
[[ ! -e "$FAKE_SESSION_DIRECTORY/lease-ambiguous" ]]

new_session update-conflict
acquire_session
export FAKE_MODE=update-conflict
expect_failure 'version conflict must fail' \
  fly_machine_session_update "$FAKE_SESSION_DIRECTORY" "$temporary_directory/config.json" \
  104052994
[[ -f "$FAKE_SESSION_DIRECTORY/lease.header" ]]
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session expired-auth-update
acquire_session
export FAKE_MODE=expired-auth-update
expect_failure 'an authentication-rejected mutation must not be replayed automatically' \
  fly_machine_session_update "$FAKE_SESSION_DIRECTORY" "$temporary_directory/config.json" \
  104052994
[[ "$(grep -c 'POST .*machines/0123456789abcd$' "$FAKE_CURL_LOG")" == 1 ]]
grep -Fx 'Authorization: Bearer test-secret-token' \
  "$FAKE_SESSION_DIRECTORY/auth.header" >/dev/null
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session malformed-get
acquire_session
export FAKE_MODE=malformed-get
expect_failure 'malformed Machine response must fail' \
  fly_machine_session_get "$FAKE_SESSION_DIRECTORY" "$temporary_directory/malformed.json"
[[ -f "$FAKE_SESSION_DIRECTORY/lease.header" ]]
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session expired-auth
acquire_session
export FAKE_MODE=expired-auth
expect_failure 'an authentication-rejected read must require an explicit replacement token' \
  fly_machine_session_get "$FAKE_SESSION_DIRECTORY" \
  "$temporary_directory/rejected-auth-get.json"
grep -Fx 'Authorization: Bearer test-secret-token' \
  "$FAKE_SESSION_DIRECTORY/auth.header" >/dev/null
[[ "$(grep -c 'GET .*machines/0123456789abcd' "$FAKE_CURL_LOG")" == 1 ]]
! grep -q '^auth token$' "$FAKE_FLY_LOG"
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session expired-auth-refresh
acquire_session
export FAKE_MODE=expired-auth
export FLY_MACHINE_SESSION_ALLOW_CLI_TOKEN_REFRESH=1
fly_machine_session_get "$FAKE_SESSION_DIRECTORY" \
  "$temporary_directory/refreshed-auth-get.json"
grep -Fx 'Authorization: Bearer test-refreshed-token' \
  "$FAKE_SESSION_DIRECTORY/auth.header" >/dev/null
[[ "$(grep -c 'GET .*machines/0123456789abcd' "$FAKE_CURL_LOG")" == 2 ]]
[[ "$(grep -c '^auth token$' "$FAKE_FLY_LOG")" == 1 ]]
jq '.state = "started"' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY"
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"
unset FLY_MACHINE_SESSION_ALLOW_CLI_TOKEN_REFRESH

new_session expired-auth-update-refresh
acquire_session
export FAKE_MODE=expired-auth-update
export FLY_MACHINE_SESSION_ALLOW_CLI_TOKEN_REFRESH=1
expect_failure 'a rejected mutation must not replay after credential refresh' \
  fly_machine_session_update "$FAKE_SESSION_DIRECTORY" "$temporary_directory/config.json" \
  104052994
[[ "$(grep -c 'POST .*machines/0123456789abcd$' "$FAKE_CURL_LOG")" == 1 ]]
grep -Fx 'Authorization: Bearer test-refreshed-token' \
  "$FAKE_SESSION_DIRECTORY/auth.header" >/dev/null
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"
unset FLY_MACHINE_SESSION_ALLOW_CLI_TOKEN_REFRESH

new_session incomplete-get
acquire_session
jq '.incomplete_config = true' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
expect_failure 'incomplete Machine response must block an entire-config update' \
  fly_machine_session_update "$FAKE_SESSION_DIRECTORY" "$temporary_directory/config.json" \
  104052994
jq '.incomplete_config = false' "$FAKE_MACHINE_STATE" > "$FAKE_MACHINE_STATE.next"
mv "$FAKE_MACHINE_STATE.next" "$FAKE_MACHINE_STATE"
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session stop-retry
acquire_session
fly_machine_session_start "$FAKE_SESSION_DIRECTORY"
export FAKE_MODE=stop-first-failure
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 2 1 0
[[ "$(cat "$FAKE_STOP_COUNT_FILE")" == 2 ]]
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session stop-failure
acquire_session
fly_machine_session_start "$FAKE_SESSION_DIRECTORY"
export FAKE_MODE=stop-failure
expect_failure 'unconfirmed stop must fail' \
  fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 2 2 0
[[ -f "$FAKE_SESSION_DIRECTORY/start.intent" ]]
[[ -f "$FAKE_SESSION_DIRECTORY/lease.header" ]]
delete_count_before="$(grep -c 'DELETE .*lease' "$FAKE_CURL_LOG" || true)"
expect_failure 'release must reject a Machine that is not stopped' \
  fly_machine_session_release "$FAKE_SESSION_DIRECTORY"
delete_count_after="$(grep -c 'DELETE .*lease' "$FAKE_CURL_LOG" || true)"
[[ "$delete_count_before" == "$delete_count_after" ]]
export FAKE_MODE=normal
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 1 1 0
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session stop-transport-timeout
acquire_session
fly_machine_session_start "$FAKE_SESSION_DIRECTORY"
export FAKE_MODE=stop-transport-timeout
if FLY_MACHINE_SESSION_CONNECT_TIMEOUT_SECONDS=2 \
  FLY_MACHINE_SESSION_REQUEST_TIMEOUT_SECONDS=3 \
  fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 1 1 0 >/dev/null 2>&1; then
  printf 'stop transport timeouts must fail closed\n' >&2
  exit 1
fi
[[ -f "$FAKE_SESSION_DIRECTORY/lease.header" ]]
grep -F -- '--connect-timeout 2 --max-time 3' "$FAKE_CURL_LOG" >/dev/null
recovery_directory="$temporary_directory/preserved-stop-session"
fly_machine_session_preserve "$FAKE_SESSION_DIRECTORY" "$recovery_directory" >/dev/null
[[ -f "$recovery_directory/lease.header" ]]
export FAKE_SESSION_DIRECTORY="$recovery_directory"
export FAKE_MODE=normal
fly_machine_session_stop "$FAKE_SESSION_DIRECTORY" 1 1 0
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

new_session release-failure
acquire_session
export FAKE_MODE=release-failure
expect_failure 'release API failure must fail' \
  fly_machine_session_release "$FAKE_SESSION_DIRECTORY"
[[ -f "$FAKE_SESSION_DIRECTORY/lease.header" ]]
export FAKE_MODE=normal
fly_machine_session_release "$FAKE_SESSION_DIRECTORY"

if grep -Eq 'test-secret-token|test-lease-nonce' "$temporary_directory"/*-curl.log; then
  printf 'credentials must not appear in curl arguments\n' >&2
  exit 1
fi
if grep -q '^[[:space:]]*trap ' "$SCRIPT_DIR/lib/fly-machine-session.sh"; then
  printf 'the session library must not install traps\n' >&2
  exit 1
fi

printf 'fly Machine session tests passed\n'
