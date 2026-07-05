#!/usr/bin/env sh
set -eu

PROJECT_NAME="${CLOUDFLARE_PAGES_PROJECT:-${CF_PAGES_PROJECT:-ytsearch}}"
PRODUCTION_BRANCH="${CLOUDFLARE_PAGES_BRANCH:-${CF_PAGES_BRANCH:-main}}"
CURRENT_BRANCH="$(git branch --show-current 2>/dev/null || printf '%s' "$PRODUCTION_BRANCH")"
DEPLOY_BRANCH="${1:-$CURRENT_BRANCH}"

# Load local secrets/config after defaults so .env can override project/branch/token values.
# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

PROJECT_NAME="${CLOUDFLARE_PAGES_PROJECT:-${CF_PAGES_PROJECT:-$PROJECT_NAME}}"
PRODUCTION_BRANCH="${CLOUDFLARE_PAGES_BRANCH:-${CF_PAGES_BRANCH:-$PRODUCTION_BRANCH}}"
DEPLOY_BRANCH="${1:-${DEPLOY_BRANCH:-$CURRENT_BRANCH}}"

if [ -n "${CF_API_TOKEN:-}" ] && [ -z "${CLOUDFLARE_API_TOKEN:-}" ]; then
  export CLOUDFLARE_API_TOKEN="$CF_API_TOKEN"
fi

if [ -z "${VITE_API_BASE_URL:-}" ]; then
  echo "Missing required environment variable: VITE_API_BASE_URL" >&2
  exit 1
fi

if [ -z "${CLOUDFLARE_API_TOKEN:-}" ]; then
  if ! bunx wrangler whoami >/dev/null 2>&1; then
    echo "Missing CLOUDFLARE_API_TOKEN/CF_API_TOKEN and Wrangler is not logged in. Add a token to .env or run wrangler login." >&2
    exit 1
  fi
fi

bun run frontend:check

if ! bunx wrangler pages project create "$PROJECT_NAME" \
  --production-branch "$PRODUCTION_BRANCH" \
  --compatibility-date 2026-07-04 >/tmp/ytsearch-pages-create.log 2>&1; then
  if ! grep -qi "already exists" /tmp/ytsearch-pages-create.log; then
    cat /tmp/ytsearch-pages-create.log >&2
    exit 1
  fi
fi

bunx wrangler pages deploy dist \
  --project-name "$PROJECT_NAME" \
  --branch "$DEPLOY_BRANCH" \
  --commit-dirty=true
