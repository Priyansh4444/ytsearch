#!/usr/bin/env bash
set -euo pipefail

# Build one Rust binary over an existing checksummed Raw corpus image, so a
# release that changes only Rust code never re-uploads the multi-GB index.
#   reader:  the Raw server binary for the search app.
#   indexer: the queue/indexer binary. Its volume mounts outside /data at
#            runtime so the baked base sources stay visible to the builder.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
case "${1:-}" in
  reader)
    default_app=ytsearch-raw-production kind=raw-reader-overlay label_prefix=raw-overlay
    overlay=raw-search-overlay entrypoint=raw-search-entrypoint
    entrypoint_source=scripts/raw-search-entrypoint.sh
    ;;
  indexer)
    default_app=ytsearch-indexer-raw-production kind=raw-indexer-overlay
    label_prefix=raw-indexer overlay=raw-indexer-overlay entrypoint=cloud-index-queue
    entrypoint_source=scripts/cloud-index-queue-entrypoint.sh
    ;;
  *) printf 'usage: %s reader|indexer\n' "$0" >&2; exit 2 ;;
esac
APP_NAME="${APP_NAME:-$default_app}"
BASE_IMAGE="${BASE_IMAGE:?set BASE_IMAGE to a tested Raw corpus image}"

for command in flyctl git jq tar; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'missing required command: %s\n' "$command" >&2
    exit 1
  }
done

source_commit="$(git -C "$REPOSITORY_ROOT" rev-parse HEAD)"
if [[ ! "$BASE_IMAGE" =~ @sha256:([0-9a-f]{64})$ ]]; then
  printf 'BASE_IMAGE must name an immutable sha256 digest, got: %s\n' "$BASE_IMAGE" >&2
  exit 1
fi
base_digest="${BASH_REMATCH[1]}"
image_label="$label_prefix-${source_commit:0:12}-${base_digest:0:12}"
if [[ -n "${IMAGE_LABEL:-}" && "$IMAGE_LABEL" != "$image_label" ]]; then
  printf 'IMAGE_LABEL is derived and must equal %s\n' "$image_label" >&2
  exit 1
fi
"$SCRIPT_DIR/require-committed-rust-build-inputs.sh" "$source_commit" \
  "Dockerfile.$overlay" "fly.$overlay.toml" "$entrypoint_source" \
  scripts/build-raw-overlay.sh scripts/require-committed-rust-build-inputs.sh

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 130' INT TERM

# Archive only committed inputs; never copy a host-linked binary.
git -C "$REPOSITORY_ROOT" archive "$source_commit" -- \
  .cargo Cargo.toml Cargo.lock crates "Dockerfile.$overlay" "fly.$overlay.toml" \
  "$entrypoint_source" | tar -x -C "$tmp"
cp "$tmp/$entrypoint_source" "$tmp/$entrypoint"
mv "$tmp/Dockerfile.$overlay" "$tmp/Dockerfile"
mv "$tmp/fly.$overlay.toml" "$tmp/fly.toml"

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
  --arg kind "$kind" \
  --arg app "$APP_NAME" \
  --arg source_commit "$source_commit" \
  --arg base_image "$BASE_IMAGE" \
  --arg image "registry.fly.io/$APP_NAME:$image_label" \
  '{schema_version:1,kind:$kind,app:$app,source_commit:$source_commit,base_image:$base_image,image:$image}'
