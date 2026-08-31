#!/usr/bin/env bash
set -euo pipefail

# Build only the Raw server binary and layer it over an existing checksummed
# corpus image. This avoids repeatedly uploading and rebuilding the multi-GB
# immutable index when a release changes Rust code only.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
APP_NAME="${APP_NAME:-ytsearch-raw-production}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to a tested Raw corpus image}"

for command in flyctl git jq tar; do
  if ! command -v "$command" >/dev/null 2>&1; then
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  fi
done

source_commit="$(git -C "$REPO_ROOT" rev-parse HEAD)"
if [[ ! "$BASE_IMAGE" =~ @sha256:([0-9a-f]{64})$ ]]; then
  printf 'BASE_IMAGE must name an immutable sha256 digest, got: %s\n' "$BASE_IMAGE" >&2
  exit 1
fi
base_digest="${BASH_REMATCH[1]}"
source_short="${source_commit:0:12}"
image_label="raw-overlay-${source_short}-${base_digest:0:12}"
if [[ -n "${IMAGE_LABEL:-}" && "$IMAGE_LABEL" != "$image_label" ]]; then
  printf 'IMAGE_LABEL is derived and must equal %s\n' "$image_label" >&2
  exit 1
fi
"$SCRIPT_DIR/require-committed-rust-build-inputs.sh" "$source_commit" \
  Dockerfile.raw-search-overlay fly.raw-search-overlay.toml \
  scripts/build-raw-search-overlay.sh scripts/raw-search-entrypoint.sh \
  scripts/require-committed-rust-build-inputs.sh

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

git -C "$REPO_ROOT" archive "$source_commit" -- \
  .cargo Cargo.toml Cargo.lock crates \
  Dockerfile.raw-search-overlay fly.raw-search-overlay.toml scripts/raw-search-entrypoint.sh \
  | tar -x -C "$tmp"
cp "$tmp/scripts/raw-search-entrypoint.sh" "$tmp/raw-search-entrypoint"
mv "$tmp/Dockerfile.raw-search-overlay" "$tmp/Dockerfile"
mv "$tmp/fly.raw-search-overlay.toml" "$tmp/fly.toml"

(
  cd "$tmp"
  flyctl deploy . \
    --app "$APP_NAME" \
    --config fly.toml \
    --build-arg "BASE_IMAGE=$BASE_IMAGE" \
    --build-arg "SOURCE_COMMIT=$source_commit" \
    --image-label "$image_label" \
    --build-only \
    --push \
    --remote-only
)

jq -cn \
  --arg app "$APP_NAME" \
  --arg source_commit "$source_commit" \
  --arg base_image "$BASE_IMAGE" \
  --arg image "registry.fly.io/$APP_NAME:$image_label" \
  '{schema_version:1,kind:"raw-reader-overlay",app:$app,source_commit:$source_commit,base_image:$base_image,image:$image}'
