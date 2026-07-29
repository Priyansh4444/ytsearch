#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ -n "${CF_API_TOKEN:-}" ] && [ -z "${CLOUDFLARE_API_TOKEN:-}" ]; then
  CLOUDFLARE_API_TOKEN="$CF_API_TOKEN"
fi

# Search and queue requests use the Worker proxy on the same origin.
export VITE_API_BASE_URL=""
export VITE_INDEX_QUEUE_API_BASE_URL=""
export CLOUDFLARE_API_TOKEN

bun run frontend:check
bunx wrangler deploy --keep-vars

PUBLIC_URL="${PUBLIC_URL:-https://search.pronsh.dev}"
smoke_response="$(mktemp)"
trap 'rm -f "$smoke_response"' EXIT

curl -fsS "${PUBLIC_URL%/}/api/availability" >"$smoke_response"
if ! jq -e '
  (.status | type == "string") and
  (.search_enabled | type == "boolean") and
  .time_zone == "America/Los_Angeles"
' "$smoke_response" >/dev/null; then
  echo "Deployment smoke test failed: /api/availability returned an invalid response" >&2
  exit 1
fi

echo "Deployment smoke test passed: ${PUBLIC_URL%/}/api/availability"
