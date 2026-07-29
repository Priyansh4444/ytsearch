#!/usr/bin/env sh
set -eu

# Smoke-check active frontend variants and the API each one is meant to use.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUERY="${QUERY:-fable}"
PER_PAGE="${PER_PAGE:-3}"
CURL_MAX_TIME="${CURL_MAX_TIME:-15}"

TYPESENSE_FRONTEND_URL="${TYPESENSE_FRONTEND_URL:-${TYPESENSE_FLY_FRONTEND_URL:-https://search.pronsh.dev}}"

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

check_url() {
  label="$1"
  url="$2"
  http="$(curl -sS --max-time "$CURL_MAX_TIME" -o /tmp/ytsearch-check-response.json -w "%{http_code}" "$url" || true)"
  if [ "$http" != "200" ]; then
    echo "$label failed: HTTP $http $url" >&2
    exit 1
  fi
  echo "$label ok: HTTP $http"
}

check_api() {
  label="$1"
  base_url="$2"
  availability_file="/tmp/ytsearch-check-availability.json"
  availability_http="$(curl -sS --max-time "$CURL_MAX_TIME" -o "$availability_file" -w "%{http_code}" "$base_url/api/availability" || true)"
  if [ "$availability_http" != "200" ]; then
    echo "$label availability failed: HTTP $availability_http" >&2
    exit 1
  fi
  if ! jq -e '.search_enabled | type == "boolean"' "$availability_file" >/dev/null; then
    echo "$label availability failed: invalid response" >&2
    exit 1
  fi
  query="$(printf '%s' "$QUERY" | jq -sRr @uri)"
  url="$base_url/api/search?q=$query&per_page=$PER_PAGE"
  http="$(curl -sS --max-time "$CURL_MAX_TIME" -o /tmp/ytsearch-check-response.json -w "%{http_code}" "$url" || true)"
  if jq -e '.search_enabled' "$availability_file" >/dev/null; then
    if [ "$http" != "200" ]; then
      echo "$label api failed during open window: HTTP $http" >&2
      exit 1
    fi
    jq -e '(.hits | type == "array") and (.videos | type == "array")' \
      /tmp/ytsearch-check-response.json >/dev/null
    echo "$label api ok: HTTP $http"
  else
    if [ "$http" != "503" ] ||
      ! jq -e '.code == "scheduled_offline"' /tmp/ytsearch-check-response.json >/dev/null; then
      echo "$label api failed during closed window: expected scheduled HTTP 503, got $http" >&2
      exit 1
    fi
    echo "$label api ok: scheduled offline"
  fi
}

check_frontend_config() {
  label="$1"
  frontend_url="$2"
  expected_api_url="$3"
  html_file="/tmp/ytsearch-check-frontend.html"
  bundle_file="/tmp/ytsearch-check-frontend.js"

  curl -fsS --max-time "$CURL_MAX_TIME" "$frontend_url" >"$html_file"
  asset_path="$(sed -n 's/.*src="\([^"?]*assets\/index-[^"]*\.js\)".*/\1/p' "$html_file" | head -n 1)"
  if [ -z "$asset_path" ]; then
    echo "$label failed: could not find the frontend entry bundle" >&2
    exit 1
  fi

  asset_path="${asset_path#/}"
  curl -fsS --max-time "$CURL_MAX_TIME" "${frontend_url%/}/$asset_path" >"$bundle_file"
  if grep -q 'ytsearch-index-queue-staging.fly.dev' "$bundle_file"; then
    echo "$label failed: bundle still references the retired queue API" >&2
    exit 1
  fi
  if ! grep -q "${expected_api_url#https://}" "$bundle_file"; then
    # The production Worker intentionally uses same-origin /api routes and
    # proxies them to the configured Fly API, so no backend hostname appears
    # in the browser bundle.
    if ! grep -q '/api/search' "$bundle_file" ||
      ! grep -q '/api/index-requests' "$bundle_file"; then
      echo "$label failed: neither the expected API origin nor same-origin routes are present" >&2
      exit 1
    fi
  fi
  echo "$label config ok: active API owns search and queue routes"
}

required_env TYPESENSE_FLY_API_BASE_URL

check_url "frontend typesense" "$TYPESENSE_FRONTEND_URL"
check_frontend_config "frontend typesense" "$TYPESENSE_FRONTEND_URL" "$TYPESENSE_FLY_API_BASE_URL"
check_api "frontend typesense" "$TYPESENSE_FRONTEND_URL"
check_url "frontend typesense queue" "$TYPESENSE_FLY_API_BASE_URL/api/index-status"
