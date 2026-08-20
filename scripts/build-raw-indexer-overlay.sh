#!/usr/bin/env bash
set -euo pipefail

# Build the queue/indexer binary over the exact immutable Raw corpus image.
# The existing indexer volume is mounted outside /data at runtime so the baked
# base sources remain visible to the detached generation builder.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${APP_NAME:-ytsearch-indexer-raw-production}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to the promoted Raw corpus image}"
IMAGE_LABEL="${IMAGE_LABEL:-raw-indexer-$(git -C "$REPOSITORY_ROOT" rev-parse --short HEAD)}"

for command in cargo flyctl; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM

cargo build --locked --release -p youtube_cli --manifest-path "$REPOSITORY_ROOT/Cargo.toml"
cp "$REPOSITORY_ROOT/target/release/ytsearch" "$temporary_directory/ytsearch"
cp "$REPOSITORY_ROOT/scripts/cloud-index-queue-entrypoint.sh" \
  "$temporary_directory/cloud-index-queue"
cp "$REPOSITORY_ROOT/Dockerfile.raw-indexer-overlay" "$temporary_directory/Dockerfile"
cp "$REPOSITORY_ROOT/fly.raw-indexer-overlay.toml" "$temporary_directory/fly.toml"

(
  cd "$temporary_directory"
  flyctl deploy . \
    --app "$APP_NAME" \
    --config fly.toml \
    --build-arg "BASE_IMAGE=$BASE_IMAGE" \
    --image-label "$IMAGE_LABEL" \
    --build-only \
    --push \
    --remote-only
)

printf 'image=registry.fly.io/%s:%s\n' "$APP_NAME" "$IMAGE_LABEL"
