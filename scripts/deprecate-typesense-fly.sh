#!/usr/bin/env sh
set -eu

# This marks the four known legacy Machines after Raw owns both reads and
# writes. Fly Machine metadata updates can restart a Machine, so this script is
# deliberately fail-closed and must not be run before the cutover proof passes.

if [ "${APPLY_TYPESENSE_DEPRECATION:-}" != "mark-known-typesense-machines" ]; then
  cat >&2 <<'EOF'
Dry run only. No Fly resources were changed.

After the Raw read, write, queue, publication, and restart gates pass, run:
  APPLY_TYPESENSE_DEPRECATION=mark-known-typesense-machines scripts/deprecate-typesense-fly.sh
EOF
  exit 0
fi

if ! flyctl auth whoami >/dev/null 2>&1; then
  echo "Fly authentication is required; run 'flyctl auth login' first." >&2
  exit 1
fi

mark_deprecated() {
  app="$1"
  machine="$2"
  successor="$3"
  flyctl machine update "$machine" \
    --app "$app" \
    --metadata "lifecycle=deprecated" \
    --metadata "deprecated_reason=raw-search-cutover" \
    --metadata "successor=$successor" \
    --skip-start \
    --yes
}

mark_deprecated ytsearch-typesense-staging 8147deb94937e8 ytsearch-raw-production
mark_deprecated ytsearch-typesense-staging d8de466f330468 ytsearch-raw-production
mark_deprecated ytsearch-api-typesense-fly-staging 1855eddf654298 ytsearch-api-raw-production
mark_deprecated ytsearch-indexer-staging 68352d2be17728 ytsearch-indexer-raw-production

echo "Marked the four known legacy Machines as deprecated. No app or volume was destroyed."
