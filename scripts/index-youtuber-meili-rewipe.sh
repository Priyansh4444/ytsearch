#!/usr/bin/env sh
set -eu

# DEPRECATED: Meilisearch was retired and its data was destroyed. This wrapper
# remains only as a historical migration reference and must not be used.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

echo "Meilisearch is retired and its data was destroyed. There is no rewipe target." >&2
exit 1
