#!/usr/bin/env sh
# Publishes search.experimental.pronsh.dev: production's Worker and pages with the
# hybrid toggle and moment sharing on, in front of ytsearch-semantic-experimental.
# It deploys nothing to production.
set -eu
cd "$(dirname "$0")/.."

export VITE_API_BASE_URL=""
export VITE_INDEX_QUEUE_API_BASE_URL=""
export VITE_ENGAGEMENT_SORTS="true"
export VITE_HYBRID_TOGGLE="true"
export VITE_MOMENT_SHARING="true"
export VITE_POSTHOG_KEY=""

bun run build
bunx wrangler deploy --config wrangler.experimental.jsonc

PUBLIC_URL="https://search.experimental.pronsh.dev"
curl -fsS "$PUBLIC_URL/api/availability" | jq -e '.search_enabled | type == "boolean"' >/dev/null
echo "experimental site is live: $PUBLIC_URL"
