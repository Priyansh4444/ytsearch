#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="${RUST_BUILD_REPOSITORY_ROOT:-$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)}"
EXPECTED_COMMIT="${1:?pass the full release commit OID}"
shift

case "$EXPECTED_COMMIT" in
  *[!0-9a-f]*|'')
    echo "Rust release builds require a full lowercase hexadecimal commit OID." >&2
    exit 1
    ;;
esac
if [ "${#EXPECTED_COMMIT}" -ne 40 ] \
  || ! git -C "$REPOSITORY_ROOT" cat-file -e "$EXPECTED_COMMIT^{commit}" 2>/dev/null; then
  echo "Rust release builds require an existing full commit OID." >&2
  exit 1
fi
if [ "$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)" != "$EXPECTED_COMMIT" ]; then
  echo "HEAD changed after the Rust release commit was captured; restart the build." >&2
  exit 1
fi

if ! git -C "$REPOSITORY_ROOT" diff --quiet "$EXPECTED_COMMIT" -- \
  .cargo Cargo.toml Cargo.lock crates "$@"; then
  echo "Rust release inputs differ from HEAD; commit them before building an image." >&2
  exit 1
fi

untracked="$(git -C "$REPOSITORY_ROOT" ls-files --others --exclude-standard -- \
  .cargo Cargo.toml Cargo.lock crates "$@")"
if [ -n "$untracked" ]; then
  echo "Rust release inputs contain untracked files; commit them before building an image:" >&2
  printf '%s\n' "$untracked" >&2
  exit 1
fi
