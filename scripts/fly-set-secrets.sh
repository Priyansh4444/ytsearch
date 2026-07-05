#!/usr/bin/env sh
set -eu

APP_NAME="${APP_NAME:-ytsearch-api-pronsh-sandbox}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${APP_NAME:-ytsearch-api-pronsh-sandbox}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN to a Fly.io org token before running this script." >&2
  exit 1
fi

required_env() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -z "$value" ]; then
    echo "Missing required environment variable: $name" >&2
    exit 1
  fi
}

required_env TYPESENSE_API_URL
required_env TYPESENSE_SEARCH_API_KEY

if [ "${INDEXING_ENABLED:-false}" = "true" ]; then
  required_env YOUTUBE_API_KEY
  required_env TYPESENSE_ADMIN_API_KEY
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

flyctl secrets set \
  TYPESENSE_API_URL="$TYPESENSE_API_URL" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SEARCH_API_KEY" \
  --app "$APP_NAME"

optional_secret() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -n "$value" ]; then
    flyctl secrets set "$name=$value" --app "$APP_NAME"
  fi
}

optional_secret TYPESENSE_COLLECTION
optional_secret YOUTUBE_API_KEY
optional_secret TYPESENSE_ADMIN_API_KEY
optional_secret INDEXING_ENABLED
optional_secret YTSEARCH_INDEX_MAX_VIDEOS
optional_secret YTSEARCH_INDEX_JOBS
optional_secret YTSEARCH_INDEX_RATE_LIMIT_PER_HOUR
optional_secret YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE
optional_secret AXIOM_TOKEN
optional_secret AXIOM_API_KEY
optional_secret AXIOM_DATASET
optional_secret AXIOM_DOMAIN
optional_secret AXIOM_URL
optional_secret AXIOM_EDGE
optional_secret AXIOM_EDGE_URL
