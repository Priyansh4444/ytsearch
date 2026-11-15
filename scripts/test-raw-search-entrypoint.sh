#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT
trap 'exit 130' INT TERM
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

staged_search='0123456789abcdef0123456789abcdef0123456789abcdef'
staged_admin='fedcba9876543210fedcba9876543210fedcba9876543210'
printf '%s' "$staged_search" | base64 -d > "$temporary_directory/search"
printf '%s' "$staged_admin" | base64 -d > "$temporary_directory/admin"
PATH="$temporary_directory/bin:$PATH" \
RAW_SEARCH_API_KEY=deployed-search \
RAW_SEARCH_ADMIN_API_KEY=deployed-admin \
RAW_SEARCH_API_KEY_FILE="$temporary_directory/search" \
RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/admin" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" > "$temporary_directory/with-files"
grep -Fx "api=$staged_search" "$temporary_directory/with-files" >/dev/null
grep -Fx "admin=$staged_admin" "$temporary_directory/with-files" >/dev/null
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

# The semantic key file is optional, even where the reader's files are required,
# and a hex key reaches the sidecar unchanged through Fly's base64 decoding.
semantic_key=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
printf '%s' "$semantic_key" | base64 -d > "$temporary_directory/semantic"
cat > "$temporary_directory/bin/sidecar" <<EOF
#!/bin/sh
printf 'semantic=%s\n' "\${SEMANTIC_SEARCH_KEY:-}" > "$temporary_directory/sidecar-ran"
EOF
chmod +x "$temporary_directory/bin/sidecar"
PATH="$temporary_directory/bin:$PATH" \
RAW_SEARCH_REQUIRE_FILE_SECRETS=1 \
RAW_SEARCH_API_KEY_FILE="$temporary_directory/search" \
RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/admin" \
SEMANTIC_SEARCH_KEY_FILE="$temporary_directory/semantic" \
SEMANTIC_SIDECAR="$temporary_directory/bin/sidecar" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" > /dev/null
for _ in $(seq 50); do [ -s "$temporary_directory/sidecar-ran" ] && break; sleep 0.1; done
grep -Fx "semantic=$semantic_key" "$temporary_directory/sidecar-ran" >/dev/null
PATH="$temporary_directory/bin:$PATH" \
RAW_SEARCH_REQUIRE_FILE_SECRETS=1 \
RAW_SEARCH_API_KEY_FILE="$temporary_directory/search" \
RAW_SEARCH_ADMIN_API_KEY_FILE="$temporary_directory/admin" \
SEMANTIC_SEARCH_KEY_FILE="$temporary_directory/missing-semantic" \
SEMANTIC_SIDECAR="$temporary_directory/missing-sidecar" \
  "$SCRIPT_DIR/raw-search-entrypoint.sh" > "$temporary_directory/no-semantic"
grep -Fx 'command=youtube-raw-search serve-mutable' "$temporary_directory/no-semantic" >/dev/null

printf 'raw_search_entrypoint=ok\n'
