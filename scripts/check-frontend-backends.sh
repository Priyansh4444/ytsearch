#!/usr/bin/env sh
set -eu

# Smoke-check the three frontend variants and the API each one is meant to use.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

QUERY="${QUERY:-fable}"
PER_PAGE="${PER_PAGE:-3}"
CURL_MAX_TIME="${CURL_MAX_TIME:-15}"

MEILI_FRONTEND_URL="${MEILI_FRONTEND_URL:-https://frontend-meilisearch.ytsearch-755.pages.dev}"
TYPESENSE_CLOUD_FRONTEND_URL="${TYPESENSE_CLOUD_FRONTEND_URL:-https://frontend-typesense-cloud.ytsearch-755.pages.dev}"
TYPESENSE_FLY_FRONTEND_URL="${TYPESENSE_FLY_FRONTEND_URL:-https://frontend-typesense-fly.ytsearch-755.pages.dev}"

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
  query="$(printf '%s' "$QUERY" | jq -sRr @uri)"
  url="$base_url/api/search?q=$query&per_page=$PER_PAGE"
  check_url "$label api" "$url"
  jq -e '(.hits | type == "array") and (.videos | type == "array")' \
    /tmp/ytsearch-check-response.json >/dev/null
}

required_env MEILI_API_BASE_URL
required_env TYPESENSE_CLOUD_API_BASE_URL
required_env TYPESENSE_FLY_API_BASE_URL

check_url "frontend meilisearch" "$MEILI_FRONTEND_URL"
check_api "frontend meilisearch" "$MEILI_API_BASE_URL"

check_url "frontend typesense-cloud" "$TYPESENSE_CLOUD_FRONTEND_URL"
check_api "frontend typesense-cloud" "$TYPESENSE_CLOUD_API_BASE_URL"

check_url "frontend typesense-fly" "$TYPESENSE_FLY_FRONTEND_URL"
check_api "frontend typesense-fly" "$TYPESENSE_FLY_API_BASE_URL"
