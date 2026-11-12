#!/usr/bin/env sh
# Publishes search.experimental.pronsh.dev, which now redirects every path to the
# same path on search.pronsh.dev. It deploys nothing to production.
set -eu
cd "$(dirname "$0")/.."

bunx wrangler deploy --config wrangler.experimental.jsonc

location="$(curl -fsS -o /dev/null -w '%{redirect_url}' 'https://search.experimental.pronsh.dev/s/x?t=1')"
[ "$location" = "https://search.pronsh.dev/s/x?t=1" ] || {
  echo "unexpected redirect: $location" >&2
  exit 1
}
echo "experimental site redirects to production"
