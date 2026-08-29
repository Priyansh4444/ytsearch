#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
dockerfile="$REPOSITORY_ROOT/Dockerfile.raw-search-overlay"
builder="$SCRIPT_DIR/build-raw-search-overlay.sh"

grep -Fq 'FROM rust:1.97-bookworm AS builder' "$dockerfile"
grep -Fq 'COPY --from=builder /build/target/release/youtube-raw-search /usr/local/bin/youtube-raw-search' "$dockerfile"
grep -Fq '/usr/local/bin/youtube-raw-search --help >/dev/null' "$dockerfile"
grep -Fq 'source_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"' "$builder"
grep -Fq 'git -C "$REPO_ROOT" archive "$source_commit"' "$builder"
grep -Fq 'require-committed-rust-build-inputs.sh' "$builder"

if grep -Fq 'target/release/youtube-raw-search' "$builder"; then
  echo "the Raw reader overlay must not copy a host-linked Rust binary" >&2
  exit 1
fi

echo "raw_search_overlay_build=ok"
