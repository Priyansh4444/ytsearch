#!/usr/bin/env bash
# Fake flyctl and Fly Machines API (curl) shared by the script tests. The fakes
# read FAKE_* variables at run time. Credentials: FLY_API_TOKEN must be
# test-secret-token, `flyctl auth token` returns test-refreshed-token, and the
# lease nonce is test-lease-nonce.

# Writes curl and flyctl stubs into a bin directory that run fake_fly_curl and
# fake_flyctl from this file.
fake_fly_install() {
  local library
  library="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/test-fly-fakes.sh"
  mkdir -p "$1"
  printf '#!/usr/bin/env bash\nset -euo pipefail\nsource %q\nfake_fly_curl "$@"\n' \
    "$library" > "$1/curl"
  printf '#!/usr/bin/env bash\nset -euo pipefail\nsource %q\nfake_flyctl "$@"\n' \
    "$library" > "$1/flyctl"
  chmod +x "$1/curl" "$1/flyctl"
}

# flyctl: logs to FAKE_FLY_LOG, lists FAKE_FLY_STATE (FAKE_TARGET_STATE for the
# Raw search app when set) and stages secrets for FAKE_FLY_APP only.
fake_flyctl() {
  printf '%s\n' "$*" >> "$FAKE_FLY_LOG"
  case " $* " in
    " machine list "*"--app ytsearch-raw-production "*)
      cat "${FAKE_TARGET_STATE:-$FAKE_FLY_STATE}" ;;
    " machine list "*) cat "$FAKE_FLY_STATE" ;;
    " secrets sync --stage --app $FAKE_FLY_APP ") ;;
    " auth token ") printf 'test-refreshed-token\n' ;;
    *) printf 'unexpected fake flyctl command: %s\n' "$*" >&2; exit 93 ;;
  esac
}

# Parses the curl arguments fly-machine-session.sh sends into method, output,
# request, url, auth_header and lease_header, logs them to FAKE_CURL_LOG, and
# checks the mode-600 credential header files.
fake_curl_parse() {
  method=GET output="" request="" url="" auth_header="" lease_header=""
  printf '%s\n' "$*" >> "$FAKE_CURL_LOG"
  while (( $# > 0 )); do
    case "$1" in
      --request) method="$2"; shift 2 ;;
      --output) output="$2"; shift 2 ;;
      --data-binary) request="${2#@}"; shift 2 ;;
      --header)
        if [[ "$2" == @* ]]; then
          local header_file="${2#@}"
          if grep -q '^Authorization:' "$header_file"; then
            auth_header="$header_file"
          elif grep -q '^fly-machine-lease-nonce:' "$header_file"; then
            lease_header="$header_file"
          fi
        fi
        shift 2
        ;;
      --write-out|--connect-timeout|--max-time) shift 2 ;;
      --silent|--show-error) shift ;;
      http://*|https://*) url="$1"; shift ;;
      *) printf 'unexpected fake curl argument: %s\n' "$1" >&2; exit 91 ;;
    esac
  done
  [[ -n "$output" && -n "$auth_header" && -n "$url" ]]
  [[ "$(stat -c '%a' "$auth_header")" == 600 ]]
  grep -Fx -e 'Authorization: Bearer test-secret-token' \
    -e 'Authorization: Bearer test-refreshed-token' "$auth_header" >/dev/null
  if [[ "$url" != */lease || "$method" == DELETE ]]; then
    [[ -n "$lease_header" && "$(stat -c '%a' "$lease_header")" == 600 ]]
    grep -Fx 'fly-machine-lease-nonce: test-lease-nonce' "$lease_header" >/dev/null
  fi
}

# Machines API for the first machine in FAKE_FLY_STATE, whose id is
# FAKE_MACHINE_ID. An update must satisfy the jq filter FAKE_UPDATE_FILTER; it
# replaces the config, bumps instance_id from -1 to -2 and resolves the image
# to FAKE_RESOLVED_DIGEST.
fake_fly_curl() {
  fake_curl_parse "$@"
  case "$method:$url" in
    POST:*/lease)
      jq -e '.ttl == 300' "$request" >/dev/null
      printf '{"data":{"nonce":"test-lease-nonce","expires_at":4102444800}}\n' > "$output"
      printf 201
      ;;
    GET:*/machines/"$FAKE_MACHINE_ID")
      jq '.[0]' "$FAKE_FLY_STATE" > "$output"
      printf 200
      ;;
    POST:*/machines/"$FAKE_MACHINE_ID"/stop)
      jq '.[0].state = "stopped"' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
      mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
      printf '{}\n' > "$output"
      printf 200
      ;;
    POST:*/machines/"$FAKE_MACHINE_ID")
      jq -e "$FAKE_UPDATE_FILTER" "$request" >/dev/null
      jq --slurpfile request "$request" --arg digest "$FAKE_RESOLVED_DIGEST" '
        .[0].config = $request[0].config |
        .[0].instance_id |= sub("-1$"; "-2") |
        .[0].state = "created" |
        .[0].image_ref.digest = ("sha256:" + $digest)
      ' "$FAKE_FLY_STATE" > "$FAKE_FLY_STATE.next"
      mv "$FAKE_FLY_STATE.next" "$FAKE_FLY_STATE"
      jq '.[0]' "$FAKE_FLY_STATE" > "$output"
      printf 200
      ;;
    DELETE:*/lease)
      printf '{}\n' > "$output"
      printf 204
      ;;
    *) printf 'unexpected fake curl request: %s %s\n' "$method" "$url" >&2; exit 92 ;;
  esac
}

# Runs a command that must fail, quietly; exits with the message if it passes.
expect_failure() {
  local message="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    printf '%s\n' "$message" >&2
    exit 1
  fi
}

# Prepares a test directory for fake_fly_curl and exports the environment the
# Fly scripts read: fake bin first on PATH, a flyctl config staging
# min_secrets_version for app, the production inventory fixture, and a base
# image with base_digest that resolves to resolved_digest.
fake_fly_setup() {
  local directory="$1" app="$2" min_secrets_version="$3"
  fake_fly_install "$directory/bin"
  printf 'app_secrets_minvers:\n  %s: %s\n' "$app" "$min_secrets_version" \
    > "$directory/fly-config.yml"
  : > "$directory/fly.log"
  : > "$directory/curl.log"
  source_commit="$(git -C "$(dirname -- "${BASH_SOURCE[0]}")" rev-parse HEAD)"
  base_digest="$(printf '1%.0s' {1..64})"
  resolved_digest="$(printf '2%.0s' {1..64})"
  export PATH="$directory/bin:$PATH" \
    FLY_CONFIG_FILE="$directory/fly-config.yml" \
    FLY_API_TOKEN=test-secret-token \
    FLY_API_HOSTNAME=https://api.test \
    FLY_PRODUCTION_INVENTORY_PATH="$(dirname -- "${BASH_SOURCE[0]}")/../testdata/fly-production-resources.json" \
    BASE_IMAGE="registry.fly.io/ytsearch-raw-production:base@sha256:$base_digest" \
    FAKE_FLY_APP="$app" \
    FAKE_FLY_STATE="$directory/state.json" \
    FAKE_FLY_LOG="$directory/fly.log" \
    FAKE_CURL_LOG="$directory/curl.log" \
    FAKE_RESOLVED_DIGEST="$resolved_digest"
}
