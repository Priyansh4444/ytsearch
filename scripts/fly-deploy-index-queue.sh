#!/usr/bin/env sh
set -eu

APP_NAME="${INDEX_QUEUE_APP_NAME:-ytsearch-index-queue-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${INDEX_QUEUE_APP_NAME:-$APP_NAME}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN to a Fly.io org token before running this script." >&2
  exit 1
fi

if [ -z "${YTSEARCH_INDEX_QUEUE_TOKEN:-}" ]; then
  echo "Missing required environment variable: YTSEARCH_INDEX_QUEUE_TOKEN" >&2
  echo "Generate one with: openssl rand -hex 32" >&2
  exit 1
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

if ! flyctl volumes list --app "$APP_NAME" | grep -q 'index_queue_data'; then
  flyctl volumes create index_queue_data \
    --app "$APP_NAME" \
    --region "$FLY_REGION" \
    --size "${INDEX_QUEUE_VOLUME_SIZE_GB:-1}" \
    --yes
fi

if [ -z "${TYPESENSE_SELFHOST_API_URL:-}" ]; then
  echo "Missing required environment variable: TYPESENSE_SELFHOST_API_URL" >&2
  exit 1
fi
if [ -z "${TYPESENSE_SELFHOST_API_KEY:-}" ]; then
  echo "Missing required environment variable: TYPESENSE_SELFHOST_API_KEY" >&2
  exit 1
fi

flyctl secrets set \
  TYPESENSE_API_URL="$TYPESENSE_SELFHOST_API_URL" \
  TYPESENSE_SEARCH_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  TYPESENSE_ADMIN_API_KEY="$TYPESENSE_SELFHOST_API_KEY" \
  YTSEARCH_INDEX_QUEUE_TOKEN="$YTSEARCH_INDEX_QUEUE_TOKEN" \
  --app "$APP_NAME"

flyctl deploy --config fly.index-queue.toml --app "$APP_NAME" --primary-region "$FLY_REGION"
