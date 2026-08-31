#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM
mkdir -p "$temporary_directory/bin"

cat > "$temporary_directory/bin/chown" <<'EOF'
#!/bin/sh
exit 0
EOF
cat > "$temporary_directory/bin/gosu" <<'EOF'
#!/bin/sh
shift
printf 'api=%s\nadmin=%s\ncommand=%s\n' \
  "${RAW_SEARCH_API_KEY:-}" "${RAW_SEARCH_ADMIN_API_KEY:-}" "$*"
EOF
chmod +x "$temporary_directory/bin/chown" "$temporary_directory/bin/gosu"

printf 'staged-search' > "$temporary_directory/search"
printf 'staged-admin' > "$temporary_directory/admin"
PATH="$temporary_directory/bin:$PATH" \
RAW_SEARCH_API_KEY=deployed-search \
RAW_SEARCH_ADMIN_API_KEY=deployed-admin \
RAW_SEARCH_API_KEY_FILE="$temporary_directory/search" \
RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" > "$temporary_directory/with-files"
grep -Fx 'api=staged-search' "$temporary_directory/with-files" >/dev/null
grep -Fx 'admin=staged-admin' "$temporary_directory/with-files" >/dev/null
grep -Fx 'command=youtube-raw-search serve-mutable' "$temporary_directory/with-files" >/dev/null

PATH="$temporary_directory/bin:$PATH" \
RAW_SEARCH_API_KEY=deployed-search \
RAW_SEARCH_ADMIN_API_KEY=deployed-admin \
RAW_SEARCH_API_KEY_FILE="$temporary_directory/missing-search" \
RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/missing-admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" > "$temporary_directory/without-files"
grep -Fx 'api=deployed-search' "$temporary_directory/without-files" >/dev/null
grep -Fx 'admin=deployed-admin' "$temporary_directory/without-files" >/dev/null

if PATH="$temporary_directory/bin:$PATH" \
  RAW_SEARCH_REQUIRE_FILE_SECRETS=1 \
  RAW_SEARCH_API_KEY_FILE="$temporary_directory/missing-search" \
  RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/missing-admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" >/dev/null 2>&1; then
  printf 'required secret files must fail closed when missing\n' >&2
  exit 1
fi

printf 'bad\nvalue\n' > "$temporary_directory/multiline"
if PATH="$temporary_directory/bin:$PATH" \
  RAW_SEARCH_API_KEY_FILE="$temporary_directory/multiline" \
  RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/missing-admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" >/dev/null 2>&1; then
  printf 'multiline secret files must fail closed\n' >&2
  exit 1
fi

printf 'bad\rvalue' > "$temporary_directory/carriage-return"
if PATH="$temporary_directory/bin:$PATH" \
  RAW_SEARCH_API_KEY_FILE="$temporary_directory/carriage-return" \
  RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/missing-admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" >/dev/null 2>&1; then
  printf 'secret files containing carriage returns must fail closed\n' >&2
  exit 1
fi

printf 'raw_search_entrypoint=ok\n'
