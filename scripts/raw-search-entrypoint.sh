#!/bin/sh
set -eu

load_secret_file() {
  variable="$1"
  path="$2"
  if [ ! -f "$path" ]; then
    [ "$require_file_secrets" = 0 ] && return 0
    echo "raw-search-entrypoint: required secret file is missing: $path" >&2
    exit 1
  fi
  value="$(cat "$path")"
  [ -n "$value" ] || {
    echo "raw-search-entrypoint: $path is empty" >&2
    exit 1
  }
  [ "$(printf '%s' "$value" | tr -d '\r\n')" = "$value" ] || {
    echo "raw-search-entrypoint: $path must contain one line" >&2
    exit 1
  }
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

# Canary Machines receive staged app secrets as Machine-scoped files. This
# validates a rotation without promoting it into the sole production reader.
# Production continues to use environment secrets when these files are absent.
load_secret_file RAW_SEARCH_API_KEY \
  "${RAW_SEARCH_API_KEY_FILE:-/run/secrets/raw-search-api-key}"
load_secret_file RAW_SEARCH_ADMIN_API_KEY \
  "${RAW_SEARCH_ADMIN_API_KEY_FILE:-/run/secrets/raw-search-admin-api-key}"

chown rawsearch:rawsearch /data/mutable
exec gosu rawsearch youtube-raw-search serve-mutable
