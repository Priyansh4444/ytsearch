#!/usr/bin/env sh
set -eu

if [ "${ALL_VIDEOS:-0}" != "1" ] && [ -z "${MAX_VIDEOS:-}" ]; then
  set -- "$@" --max-videos "${RECENT_VIDEOS:-50}"
elif [ -n "${MAX_VIDEOS:-}" ]; then
  set -- "$@" --max-videos "$MAX_VIDEOS"
fi

MODE=update exec "$(dirname "$0")/index-all-backends.sh" "$@"
