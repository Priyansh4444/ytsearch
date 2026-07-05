#!/usr/bin/env sh
set -eu

# Incrementally index every handle in a plain-text list file.
# Accepts lines like:
#   @t3dotgg
#   @t3dotgg | Theo - t3․gg | 973 indexed
# Blank lines and lines starting with # are ignored.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

if [ "$#" -lt 1 ]; then
  echo "Usage: scripts/index-youtubers-from-list.sh youtubers.txt [extra index args]" >&2
  exit 1
fi

LIST_FILE="$1"
shift

if [ ! -f "$LIST_FILE" ]; then
  echo "List file not found: $LIST_FILE" >&2
  exit 1
fi

exec cargo run --release -p youtube_cli -- index-youtubers "$LIST_FILE" "$@"
