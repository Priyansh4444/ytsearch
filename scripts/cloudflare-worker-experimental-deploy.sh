#!/usr/bin/env sh
set -eu

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ -n "${CF_API_TOKEN:-}" ] && [ -z "${CLOUDFLARE_API_TOKEN:-}" ]; then
  CLOUDFLARE_API_TOKEN="$CF_API_TOKEN"
fi

export VITE_API_BASE_URL=""
export VITE_INDEX_QUEUE_API_BASE_URL=""
export VITE_EXPERIMENTAL_RAW_SEARCH="true"
export CLOUDFLARE_API_TOKEN

bun run frontend:check
bun scripts/check-bundle-performance.ts
bunx wrangler deploy --config wrangler.experimental.jsonc

public_url="${PUBLIC_URL:-https://search.experimental.pronsh.dev}"
smoke_response="$(mktemp)"
trap 'rm -f "$smoke_response"' EXIT

curl -fsS "${public_url%/}/api/availability" >"$smoke_response"
jq -e '.status == "ready" and .search_enabled == true and .enforced == false' \
  "$smoke_response" >/dev/null

curl -fsS -G "${public_url%/}/api/search" \
  --data-urlencode 'q=blackhole' \
  --data-urlencode 'sort=views_desc' \
  --data-urlencode 'per_page=3' >"$smoke_response"
jq -e '
  .sort == "views_desc" and
  (.videos | length) > 0 and
  all(.videos[]; (.view_count | type) == "number") and
  [.videos[].view_count] as $views |
  all(range(1; $views | length); $views[. - 1] >= $views[.])
' "$smoke_response" >/dev/null

echo "Experimental deployment smoke test passed: ${public_url%/}"
