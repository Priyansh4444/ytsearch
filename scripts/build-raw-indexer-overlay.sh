#!/usr/bin/env bash
set -euo pipefail

# Build the queue/indexer binary over the exact immutable Raw corpus image.
# The existing indexer volume is mounted outside /data at runtime so the baked
# base sources remain visible to the detached generation builder.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${APP_NAME:-ytsearch-indexer-raw-production}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to the promoted Raw corpus image}"

for command in flyctl git tar; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
IMAGE_LABEL="${IMAGE_LABEL:-raw-indexer-$(git -C "$REPOSITORY_ROOT" rev-parse --short "$source_commit")}"
"$SCRIPT_DIR/require-committed-rust-build-inputs.sh" "$source_commit" \
  Dockerfile.raw-indexer-overlay fly.raw-indexer-overlay.toml \
  scripts/build-raw-indexer-overlay.sh scripts/cloud-index-queue-entrypoint.sh \
  scripts/require-committed-rust-build-inputs.sh

temporary_directory="$(mktemp -d)"
trap 'rm -rf "$temporary_directory"' EXIT INT TERM

git -C "$REPOSITORY_ROOT" archive "$source_commit" -- \
  .cargo Cargo.toml Cargo.lock crates Dockerfile.raw-indexer-overlay \
  fly.raw-indexer-overlay.toml scripts/cloud-index-queue-entrypoint.sh \
  | tar -x -C "$temporary_directory"
cp "$temporary_directory/scripts/cloud-index-queue-entrypoint.sh" \
  "$temporary_directory/cloud-index-queue"
mv "$temporary_directory/Dockerfile.raw-indexer-overlay" "$temporary_directory/Dockerfile"
mv "$temporary_directory/fly.raw-indexer-overlay.toml" "$temporary_directory/fly.toml"

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
