#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

write_rows() {
  file="$1"; shift
  : > "$file"
  for total in "$@"; do
    printf 'case=exact backend=typesense http=200 total=%ss total_videos=4 total_moments=13\n' "$total" >> "$file"
  done
}

write_rows "$tmp/baseline" 0.200 0.210 0.220 0.230 0.240
write_rows "$tmp/faster" 0.100 0.110 0.120 0.130 0.140
"$SCRIPT_DIR/compare-autoresearch-search.sh" "$tmp/baseline" "$tmp/faster" | grep -F 'overall	keep' >/dev/null

write_rows "$tmp/noisy" 0.050 0.150 0.250 0.350 0.450
if "$SCRIPT_DIR/compare-autoresearch-search.sh" "$tmp/baseline" "$tmp/noisy" > "$tmp/output"; then
  echo "noisy candidate should not be promoted" >&2
  exit 1
fi
grep -F 'overall	discard' "$tmp/output" >/dev/null

sed 's/total_videos=4/total_videos=5/' "$tmp/faster" > "$tmp/drift"
if "$SCRIPT_DIR/compare-autoresearch-search.sh" "$tmp/baseline" "$tmp/drift" > "$tmp/output"; then
  echo "corpus drift should invalidate comparison" >&2
  exit 1
fi
grep -F 'overall	invalid' "$tmp/output" >/dev/null

echo "autoresearch_compare_tests=ok"
