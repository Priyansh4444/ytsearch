#!/usr/bin/env sh
set -eu

if [ -n "${MAX_VIDEOS:-}" ]; then
  set -- "$@" --max-videos "$MAX_VIDEOS"
fi

if [ "${ALL_VIDEOS:-0}" != "1" ] && [ -z "${STOP_AFTER_EXISTING:-}" ]; then
  set -- "$@" --stop-after-existing "${EXISTING_TAIL:-50}"
elif [ -n "${STOP_AFTER_EXISTING:-}" ]; then
  set -- "$@" --stop-after-existing "$STOP_AFTER_EXISTING"
fi

MODE=update exec "$(dirname "$0")/index-all-backends.sh" "$@"
