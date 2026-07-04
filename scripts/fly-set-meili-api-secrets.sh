#!/usr/bin/env sh
set -eu

APP_NAME="${APP_NAME:-ytsearch-api-meili-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${APP_NAME:-ytsearch-api-meili-staging}"
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

required_env MEILI_HTTP_ADDR
if [ -z "${MEILI_SEARCH_API_KEY:-}" ] && [ -n "${MEILI_MASTER_KEY:-}" ]; then
  MEILI_SEARCH_API_KEY="$MEILI_MASTER_KEY"
fi
required_env MEILI_SEARCH_API_KEY

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

flyctl secrets set \
  MEILI_HTTP_ADDR="$MEILI_HTTP_ADDR" \
  MEILI_SEARCH_API_KEY="$MEILI_SEARCH_API_KEY" \
  --app "$APP_NAME"

optional_secret() {
  name="$1"
  eval "value=\${$name:-}"
  if [ -n "$value" ]; then
    flyctl secrets set "$name=$value" --app "$APP_NAME"
  fi
}

optional_secret MEILI_TRANSCRIPTS_INDEX
optional_secret YTSEARCH_SEARCH_RATE_LIMIT_PER_MINUTE
optional_secret AXIOM_TOKEN
optional_secret AXIOM_API_KEY
optional_secret AXIOM_DATASET
optional_secret AXIOM_DOMAIN
