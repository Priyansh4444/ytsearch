#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
LIST_FILE="${1:-$SCRIPT_DIR/indexed-channels.txt}"
if [ "$#" -gt 0 ]; then
  shift
fi

# One CLI process coalesces every creator's durable mutations into one verified
# Raw generation. Existing video IDs and classified failure records make the
# command safe to resume after a Machine or network interruption.
BACKENDS=raw \
MODE=update \
JOBS="${JOBS:-1}" \
CHANNEL_JOBS="${CHANNEL_JOBS:-1}" \
exec "$SCRIPT_DIR/index-all-backends.sh" "$LIST_FILE" \
  --stop-after-existing "${EXISTING_TAIL:-0}" \
  "$@"
