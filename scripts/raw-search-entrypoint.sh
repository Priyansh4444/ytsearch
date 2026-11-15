#!/bin/sh
set -eu

load_fly_secret_file() {
  variable="$1"
  path="$2"
  if [ ! -f "$path" ]; then
    [ "$require_file_secrets" = 0 ] && return 0
    echo "raw-search-entrypoint: required secret file is missing: $path" >&2
    exit 1
  fi
  [ -s "$path" ] || {
    echo "raw-search-entrypoint: $path is empty" >&2
    exit 1
  }
  # Fly decodes an app secret as base64 before materializing a Machine
  # config.files secret. Re-encode those bytes so file-backed and ordinary
  # environment injection expose the same application credential.
  value="$(base64 < "$path" | tr -d '\r\n')"
  export "$variable=$value"
}

require_file_secrets="${RAW_SEARCH_REQUIRE_FILE_SECRETS:-0}"
case "$require_file_secrets" in
  0|1) ;;
  *)
    echo "raw-search-entrypoint: RAW_SEARCH_REQUIRE_FILE_SECRETS must be 0 or 1" >&2
    exit 1
    ;;
esac

# Canary Machines receive deployed app secrets as Machine-scoped files.
# Production continues to use environment secrets when these files are absent.
load_fly_secret_file RAW_SEARCH_API_KEY \
  "${RAW_SEARCH_API_KEY_FILE:-/run/secrets/raw-search-api-key}"
load_fly_secret_file RAW_SEARCH_ADMIN_API_KEY \
  "${RAW_SEARCH_ADMIN_API_KEY_FILE:-/run/secrets/raw-search-admin-api-key}"

# Optional on every Machine: canaries without the file run the reader alone.
# A hex key survives the base64 round trip above unchanged.
semantic_key_file="${SEMANTIC_SEARCH_KEY_FILE:-/run/secrets/semantic-search-key}"
if [ -f "$semantic_key_file" ]; then
  load_fly_secret_file SEMANTIC_SEARCH_KEY "$semantic_key_file"
fi

chown rawsearch:rawsearch /data/mutable
# Hybrid search runs beside the reader when its collection is on this volume
# (see scripts/semantic-sidecar.sh); the reader never waits for it.
semantic_sidecar="${SEMANTIC_SIDECAR:-/usr/local/bin/semantic-sidecar}"
if [ -x "$semantic_sidecar" ]; then
  "$semantic_sidecar" </dev/null &
fi
exec gosu rawsearch youtube-raw-search serve-mutable
