#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
dockerfile="$REPOSITORY_ROOT/Dockerfile.raw-indexer-overlay"
builder="$SCRIPT_DIR/build-raw-indexer-overlay.sh"

grep -Fq 'FROM rust:1.97-bookworm AS builder' "$dockerfile"
grep -Fq 'COPY --from=builder /build/target/release/ytsearch /usr/local/bin/ytsearch' "$dockerfile"
grep -Fq '/usr/local/bin/ytsearch --help >/dev/null' "$dockerfile"
grep -Fq 'source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"' "$builder"
grep -Fq 'git -C "$REPOSITORY_ROOT" archive "$source_commit"' "$builder"
grep -Fq 'require-committed-rust-build-inputs.sh' "$builder"

if grep -Fq 'target/release/ytsearch' "$builder"; then
  echo "the Raw overlay must not copy a host-linked Rust binary" >&2
  exit 1
fi

echo "raw_indexer_overlay_build=ok"
