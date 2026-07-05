#!/usr/bin/env sh
set -eu

APP_NAME="${MEILI_APP_NAME:-ytsearch-meili-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${MEILI_APP_NAME:-$APP_NAME}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN to a Fly.io org token before running this script." >&2
  exit 1
fi

if [ -z "${MEILI_MASTER_KEY:-}" ]; then
  echo "Missing required environment variable: MEILI_MASTER_KEY" >&2
  exit 1
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

if ! flyctl volumes list --app "$APP_NAME" | grep -q 'meili_data'; then
  flyctl volumes create meili_data --app "$APP_NAME" --region "$FLY_REGION" --size "${MEILI_VOLUME_SIZE_GB:-3}" --yes
fi

flyctl secrets set MEILI_MASTER_KEY="$MEILI_MASTER_KEY" --app "$APP_NAME"
flyctl deploy --config fly.meilisearch.toml --app "$APP_NAME" --primary-region "$FLY_REGION"
