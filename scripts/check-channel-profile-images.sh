#!/usr/bin/env sh
set -eu

# Audit API-owned channel images without printing full channel records or image bodies.
# Set SHOW_ALL=1 only when the complete image inventory is needed.

# shellcheck disable=SC1091
. "$(dirname "$0")/load-env.sh"

API_BASE_URL="${API_BASE_URL:-${RAW_SEARCH_API_BASE_URL:-}}"
if [ -z "$API_BASE_URL" ]; then
  echo "Missing API_BASE_URL or RAW_SEARCH_API_BASE_URL" >&2
  exit 1
fi

require_command() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}

require_command curl
require_command jq
require_command file

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT INT TERM
channels="$work_dir/channels.tsv"
curl -fsS "$API_BASE_URL/api/youtubers" |
  jq -r '.channels[] | select(.channel_id != "" and .avatar_url != null) | [.handle, .channel_id] | @tsv' >"$channels"

total=0
square=0
flagged=0

while IFS="$(printf '\t')" read -r handle channel_id; do
  [ -n "$channel_id" ] || continue
  total=$((total + 1))
  image="$work_dir/$total.jpg"
  if ! curl -fsS --max-time 15 -o "$image" "$API_BASE_URL/api/channel-images/$channel_id"; then
    printf 'invalid\thandle=%s\treason=fetch-failed\n' "$handle"
    flagged=$((flagged + 1))
    continue
  fi
  dimensions="$(file -b "$image" | sed -n 's/.* \([0-9][0-9]*\)x\([0-9][0-9]*\).*/\1x\2/p')"
  width="${dimensions%x*}"
  height="${dimensions#*x}"
  if [ -z "$dimensions" ] || [ "$width" = "$dimensions" ] || [ "$width" -ne "$height" ]; then
    printf 'invalid\thandle=%s\tdimensions=%s\n' "$handle" "${dimensions:-unknown}"
    flagged=$((flagged + 1))
    continue
  fi
  square=$((square + 1))
  if [ "${SHOW_ALL:-0}" = "1" ]; then
    printf 'ok\thandle=%s\tdimensions=%s\n' "$handle" "$dimensions"
  fi
done <"$channels"

printf 'summary\tprofiles=%s\tsquare=%s\tflagged=%s\n' "$total" "$square" "$flagged"
