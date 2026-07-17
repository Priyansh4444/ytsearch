#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${APP_NAME:-ytsearch-api}"

if [ -z "${FLY_ORG:-}" ]; then
  echo "Set FLY_ORG to the Fly.io organization that should own the API." >&2
  exit 1
fi

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN to a Fly.io org token before running this script." >&2
  exit 1
fi

if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
  echo "Missing required environment variable: TYPESENSE_SELFHOST_API_KEY" >&2
  exit 1
fi
if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
  echo "Missing required environment variable: TYPESENSE_SELFHOST_API_URL" >&2
  exit 1
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

flyctl secrets set \
  TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  --app "$APP_NAME"

optional_secret() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -n "$value" ]; then
    flyctl secrets set "$name=$value" --app "$APP_NAME"
  fi
}

optional_secret TYPESENSE_COLLECTION
optional_secret YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE
optional_secret AXIOM_TOKEN
optional_secret AXIOM_API_KEY
optional_secret AXIOM_DATASET
optional_secret AXIOM_DOMAIN
optional_secret AXIOM_URL
optional_secret AXIOM_EDGE
optional_secret AXIOM_EDGE_URL
