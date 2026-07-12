#!/usr/bin/env sh
set -eu

# Retired intentionally. The queue is now part of the Typesense API app, which
# removes one always-on Machine and prevents this script from recreating it.
echo "The standalone queue app is retired. Deploy ytsearch-api-typesense-fly-staging instead." >&2
exit 1
