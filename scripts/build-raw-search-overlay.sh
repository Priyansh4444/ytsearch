#!/usr/bin/env bash
set -euo pipefail

# Build only the Raw server binary and layer it over an existing checksummed
# corpus image. This avoids repeatedly uploading and rebuilding the multi-GB
# immutable index when a release changes Rust code only.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${APP_NAME:-ytsearch-raw-production}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to a tested Raw corpus image}"

for command in flyctl git tar; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  fi
done

source_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
IMAGE_LABEL="${IMAGE_LABEL:-raw-overlay-$(git -C "$REPO_ROOT" rev-parse --short "$source_commit")}"
"$SCRIPT_DIR/require-committed-rust-build-inputs.sh" "$source_commit" \
  Dockerfile.raw-search-overlay fly.raw-search-overlay.toml \
  scripts/build-raw-search-overlay.sh scripts/require-committed-rust-build-inputs.sh

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

git -C "$REPO_ROOT" archive "$source_commit" -- \
  .cargo Cargo.toml Cargo.lock crates \
  Dockerfile.raw-search-overlay fly.raw-search-overlay.toml \
  | tar -x -C "$tmp"
mv "$tmp/Dockerfile.raw-search-overlay" "$tmp/Dockerfile"
mv "$tmp/fly.raw-search-overlay.toml" "$tmp/fly.toml"

(
  cd "$tmp"
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
