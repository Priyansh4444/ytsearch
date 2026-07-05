#!/usr/bin/env sh
set -eu

MODE=rewipe exec "$(dirname "$0")/index-all-backends.sh" "$@"
