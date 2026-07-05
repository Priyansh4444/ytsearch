#!/usr/bin/env sh
set -eu

MODE=update exec "$(dirname "$0")/index-all-backends.sh" "$@"
