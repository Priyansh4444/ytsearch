#!/usr/bin/env bash
set -euo pipefail

# Build only the Raw server binary and layer it over an existing checksummed
# corpus image. This avoids repeatedly uploading and rebuilding the multi-GB
# immutable index when a release changes Rust code only.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${APP_NAME:-ytsearch-raw-production}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to a tested Raw corpus image}"
IMAGE_LABEL="${IMAGE_LABEL:-raw-overlay-$(git -C "$REPO_ROOT" rev-parse --short HEAD)}"

for command in cargo flyctl; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  fi
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

cargo build --locked --release -p youtube_raw_search --manifest-path "$REPO_ROOT/Cargo.toml"
cp "$REPO_ROOT/target/release/youtube-raw-search" "$tmp/youtube-raw-search"
cp "$REPO_ROOT/Dockerfile.raw-search-overlay" "$tmp/Dockerfile"
cp "$REPO_ROOT/fly.raw-search-overlay.toml" "$tmp/fly.toml"

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
