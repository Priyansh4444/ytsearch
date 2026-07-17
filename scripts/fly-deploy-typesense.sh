#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${TYPESENSE_APP_NAME:-ytsearch-typesense}"
FLY_REGION="${FLY_REGION:-sjc}"

if [ -z "${FLY_ORG:-}" ]; then
  echo "Set FLY_ORG to the Fly.io organization that should own the app." >&2
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
  echo "Generate one with: openssl rand -hex 32" >&2
  exit 1
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

if ! flyctl volumes list --app "$APP_NAME" | grep -q 'typesense_data'; then
  flyctl volumes create typesense_data \
    --app "$APP_NAME" \
    --region "$FLY_REGION" \
    --size "${TYPESENSE_VOLUME_SIZE_GB:-3}" \
    --yes
fi

flyctl secrets set TYPESENSE_API_KEY="$TYPESENSE_SELFHOST_API_KEY" --app "$APP_NAME"
flyctl deploy --config fly.typesense.toml --app "$APP_NAME" --primary-region "$FLY_REGION"
