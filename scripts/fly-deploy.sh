#!/usr/bin/env sh
set -eu

APP_NAME="${APP_NAME:-ytsearch-api-meili-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"
FLY_CONFIG="${FLY_CONFIG:-fly.staging.toml}"

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

APP_NAME="${APP_NAME:-ytsearch-api-meili-staging}"
FLY_ORG="${FLY_ORG:-pronsh-sandbox}"
FLY_REGION="${FLY_REGION:-sjc}"
FLY_CONFIG="${FLY_CONFIG:-fly.staging.toml}"

if [ -n "${FLYIO_TOKEN:-}" ]; then
  export FLY_ACCESS_TOKEN="$FLYIO_TOKEN"
elif [ -z "${FLY_ACCESS_TOKEN:-}" ]; then
  echo "Set FLYIO_TOKEN or FLY_ACCESS_TOKEN to a Fly.io org token before running this script." >&2
  exit 1
fi

if ! flyctl apps list --org "$FLY_ORG" | grep -q "$APP_NAME"; then
  flyctl apps create "$APP_NAME" --org "$FLY_ORG"
fi

flyctl deploy --config "$FLY_CONFIG" --app "$APP_NAME" --primary-region "$FLY_REGION"
