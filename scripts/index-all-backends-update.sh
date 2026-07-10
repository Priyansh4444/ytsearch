#!/usr/bin/env sh
set -eu

if [ -n "${MAX_VIDEOS:-}" ]; then
  set -- "$@" --max-videos "$MAX_VIDEOS"
fi

if [ -n "${STOP_AFTER_EXISTING:-}" ]; then
  set -- "$@" --stop-after-existing "$STOP_AFTER_EXISTING"
else
  set -- "$@" --stop-after-existing "${EXISTING_TAIL:-0}"
fi

MODE=update exec "$(dirname "$0")/index-all-backends.sh" "$@"
