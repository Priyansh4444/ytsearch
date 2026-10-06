#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' HUP INT TERM

cat > "$tmp/example" <<'EOF'
RAW_SEARCH_API_URL=
RAW_SEARCH_ADMIN_API_KEY=
RAW_SEARCH_API_KEY=
YTSEARCH_INDEX_QUEUE_TOKEN=
UNCHANGED=value
EOF

YTSEARCH_SETUP_ENV_EXAMPLE="$tmp/example" \
YTSEARCH_SETUP_ENV_FILE="$tmp/.env" \
YTSEARCH_SETUP_SKIP_INSTALL=1 \
  "$SCRIPT_DIR/setup-dev.sh" > "$tmp/first-output"

grep -F 'RAW_SEARCH_API_URL=http://127.0.0.1:8108' "$tmp/.env" >/dev/null
grep -E '^RAW_SEARCH_ADMIN_API_KEY=[0-9a-f]{48}$' "$tmp/.env" >/dev/null
admin_key="$(sed -n 's/^RAW_SEARCH_ADMIN_API_KEY=//p' "$tmp/.env")"
grep -F "RAW_SEARCH_API_KEY=$admin_key" "$tmp/.env" >/dev/null
grep -E '^YTSEARCH_INDEX_QUEUE_TOKEN=[0-9a-f]{48}$' "$tmp/.env" >/dev/null
grep -F 'UNCHANGED=value' "$tmp/.env" >/dev/null
[ "$(stat -c '%a' "$tmp/.env")" = "600" ]

printf '\nCUSTOM=value\n' >> "$tmp/.env"
before="$(sha256sum "$tmp/.env")"
YTSEARCH_SETUP_ENV_EXAMPLE="$tmp/example" \
YTSEARCH_SETUP_ENV_FILE="$tmp/.env" \
YTSEARCH_SETUP_SKIP_INSTALL=1 \
  "$SCRIPT_DIR/setup-dev.sh" > "$tmp/second-output"
after="$(sha256sum "$tmp/.env")"

[ "$before" = "$after" ]
grep -F 'Kept existing' "$tmp/second-output" >/dev/null
echo "setup_dev_tests=ok"
