#!/usr/bin/env sh
set -eu

# Deploy Cloudflare Pages previews for active backend APIs.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

deploy_backend() {
  branch="$1"
  api_base_url="$2"
  queue_api_base_url="${INDEX_QUEUE_API_BASE_URL:-}"
  if [ -z "$api_base_url" ]; then
    echo "Missing API base URL for Cloudflare Pages branch: $branch" >&2
    exit 1
  fi
  if [ -z "$queue_api_base_url" ]; then
    echo "Missing required environment variable: INDEX_QUEUE_API_BASE_URL" >&2
    exit 1
  fi
  echo "Deploying frontend branch=$branch api=$api_base_url"
  VITE_API_BASE_URL="$api_base_url" \
    VITE_INDEX_QUEUE_API_BASE_URL="$queue_api_base_url" \
    scripts/cloudflare-pages-deploy.sh "$branch"
}

deploy_backend "${MEILI_PAGES_BRANCH:-meili}" \
  "${MEILI_API_BASE_URL:-}"

deploy_backend "${TYPESENSE_PAGES_BRANCH:-${TYPESENSE_FLY_PAGES_BRANCH:-typesense}}" \
  "${TYPESENSE_FLY_API_BASE_URL:-}"
