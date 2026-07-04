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

if [ "$#" -eq 0 ]; then
  echo "Usage: scripts/fly-run-indexer.sh -- index-youtuber @handle --no-wipe --skip-existing" >&2
  exit 1
fi

if [ "${1:-}" = "--" ]; then
  shift
fi

exec fly ssh console \
  --app "$APP_NAME" \
  --org "$FLY_ORG" \
  --command "ytsearch $*"
