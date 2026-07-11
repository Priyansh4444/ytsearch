#!/usr/bin/env sh
set -eu

# Deploy Cloudflare Pages previews for active backend APIs.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

deploy_backend() {
  branch="$1"
  api_base_url="$2"
  # The Typesense API owns the queue routes, so one public API origin serves
  # search and queueing without a second always-on Fly machine.
  queue_api_base_url="$api_base_url"
  if [ -z "$api_base_url" ]; then
    echo "Missing API base URL for Cloudflare Pages branch: $branch" >&2
    exit 1
  fi
  echo "Deploying frontend branch=$branch api=$api_base_url"
  VITE_API_BASE_URL="$api_base_url" \
    VITE_INDEX_QUEUE_API_BASE_URL="$queue_api_base_url" \
    scripts/cloudflare-pages-deploy.sh "$branch"
}

deploy_backend "${TYPESENSE_PAGES_BRANCH:-${TYPESENSE_FLY_PAGES_BRANCH:-head}}" \
  "${TYPESENSE_FLY_API_BASE_URL:-}"
