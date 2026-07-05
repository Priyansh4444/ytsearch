#!/usr/bin/env sh
set -eu

# Deploy three Cloudflare Pages previews, one per backend API.

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

deploy_backend "${MEILI_PAGES_BRANCH:-frontend-meilisearch}" \
  "${MEILI_API_BASE_URL:-}"

deploy_backend "${TYPESENSE_CLOUD_PAGES_BRANCH:-frontend-typesense-cloud}" \
  "${TYPESENSE_CLOUD_API_BASE_URL:-}"

deploy_backend "${TYPESENSE_FLY_PAGES_BRANCH:-frontend-typesense-fly}" \
  "${TYPESENSE_FLY_API_BASE_URL:-}"
