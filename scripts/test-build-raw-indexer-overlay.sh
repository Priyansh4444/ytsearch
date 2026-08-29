#!/usr/bin/env sh
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
REPOSITORY_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
dockerfile="$REPOSITORY_ROOT/Dockerfile.raw-indexer-overlay"
builder="$SCRIPT_DIR/build-raw-indexer-overlay.sh"

grep -Fq 'FROM rust:1.97-bookworm AS builder' "$dockerfile"
grep -Fq 'COPY --from=builder /build/target/release/ytsearch /usr/local/bin/ytsearch' "$dockerfile"
grep -Fq '/usr/local/bin/ytsearch --help >/dev/null' "$dockerfile"
grep -Fq 'ENV YTSEARCH_SOURCE_COMMIT=${SOURCE_COMMIT}' "$dockerfile"
grep -Fq 'org.opencontainers.image.revision=${SOURCE_COMMIT}' "$dockerfile"
grep -Fq 'source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"' "$builder"
grep -Fq 'git -C "$REPOSITORY_ROOT" archive "$source_commit"' "$builder"
grep -Fq 'require-committed-rust-build-inputs.sh' "$builder"
grep -Fq 'BASE_IMAGE must name an immutable sha256 digest' "$builder"
grep -Fq 'image_label="raw-indexer-${source_short}-${base_digest:0:12}"' "$builder"
grep -Fq -- '--build-arg "SOURCE_COMMIT=$source_commit"' "$builder"

if grep -Fq 'target/release/ytsearch' "$builder"; then
  echo "the Raw overlay must not copy a host-linked Rust binary" >&2
  exit 1
fi

echo "raw_indexer_overlay_build=ok"
