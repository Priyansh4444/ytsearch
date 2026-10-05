#!/usr/bin/env bash
set -euo pipefail

# Runs both overlay builds in a throwaway repository with stub Rust inputs and a
# fake flyctl that records its arguments and build context.

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

for dockerfile in "$REPOSITORY_ROOT"/Dockerfile.raw-{indexer,search}-overlay; do
  grep -Fq 'FROM rust:1.97-bookworm AS builder' "$dockerfile"
  grep -Fq 'ENV YTSEARCH_SOURCE_COMMIT=${SOURCE_COMMIT}' "$dockerfile"
  grep -Fq 'org.opencontainers.image.revision=${SOURCE_COMMIT}' "$dockerfile"
  grep -Fq '/usr/local/share/ytsearch-release/source-commit' "$dockerfile"
  grep -Fq '/usr/local/share/ytsearch-release/base-image' "$dockerfile"
done
indexer_dockerfile="$REPOSITORY_ROOT/Dockerfile.raw-indexer-overlay"
grep -Fq 'COPY --from=builder /build/target/release/ytsearch /usr/local/bin/ytsearch' \
  "$indexer_dockerfile"
grep -Fq '/usr/local/bin/ytsearch --help >/dev/null' "$indexer_dockerfile"
search_dockerfile="$REPOSITORY_ROOT/Dockerfile.raw-search-overlay"
grep -Fq 'COPY --from=builder /build/target/release/youtube-raw-search /usr/local/bin/youtube-raw-search' \
  "$search_dockerfile"
grep -Fq 'COPY raw-search-entrypoint /usr/local/bin/raw-search-entrypoint' "$search_dockerfile"
grep -Fq 'ENTRYPOINT ["raw-search-entrypoint"]' "$search_dockerfile"
grep -Fq '/usr/local/bin/youtube-raw-search --help >/dev/null' "$search_dockerfile"

repository="$tmp/repository"
mkdir -p "$repository/.cargo" "$repository/crates/stub/src" "$repository/target/release" "$tmp/bin"
cp -R "$REPOSITORY_ROOT/scripts" "$repository/scripts"
cp "$REPOSITORY_ROOT"/Dockerfile.raw-{indexer,search}-overlay \
  "$REPOSITORY_ROOT"/fly.raw-{indexer,search}-overlay.toml "$repository/"
printf '[build]\n' > "$repository/.cargo/config.toml"
printf '[workspace]\n' > "$repository/Cargo.toml"
printf 'version = 4\n' > "$repository/Cargo.lock"
printf '\n' > "$repository/crates/stub/src/lib.rs"
printf 'target/\n' > "$repository/.gitignore"
printf 'host-linked\n' > "$repository/target/release/ytsearch"
git -C "$repository" init -q
git -C "$repository" add -A
git -C "$repository" -c user.name=test -c user.email=test@example.invalid \
  commit -q -m fixture
source_commit="$(git -C "$repository" rev-parse HEAD)"

cat > "$tmp/bin/flyctl" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$@" > "$FAKE_FLY_ARGS"
find . -type f | sort > "$FAKE_FLY_CONTEXT"
cp Dockerfile "$FAKE_FLY_CONTEXT.dockerfile"
cp fly.toml "$FAKE_FLY_CONTEXT.fly-toml"
for entrypoint in cloud-index-queue raw-search-entrypoint; do
  if [[ -f "$entrypoint" ]]; then cp "$entrypoint" "$FAKE_FLY_CONTEXT.entrypoint"; fi
done
SH
chmod +x "$tmp/bin/flyctl"

base_digest="$(printf 'a%.0s' {1..64})"
base_image="registry.fly.io/ytsearch-raw-production:base@sha256:$base_digest"
export PATH="$tmp/bin:$PATH" \
  BASE_IMAGE="$base_image" \
  FAKE_FLY_ARGS="$tmp/fly-args" \
  FAKE_FLY_CONTEXT="$tmp/fly-context"

build() {
  local role="$1"
  shift
  rm -f "$tmp"/fly-*
  env "$@" "$repository/scripts/build-raw-overlay.sh" "$role"
}

expect_failure_before_flyctl() {
  local message="$1"
  shift
  if build "$@" >/dev/null 2>&1; then
    printf '%s\n' "$message" >&2
    exit 1
  fi
  if [[ -e "$tmp/fly-args" ]]; then
    printf 'flyctl ran despite: %s\n' "$message" >&2
    exit 1
  fi
}

check_build() {
  local role="$1" kind="$2" app="$3" label_prefix="$4" overlay="$5" entrypoint="$6"
  local entrypoint_source="$7"
  local label="$label_prefix-${source_commit:0:12}-${base_digest:0:12}"
  build "$role" APP_NAME="$app" > "$tmp/output.json"
  jq -e --arg kind "$kind" --arg app "$app" --arg source "$source_commit" \
    --arg base "$base_image" --arg image "registry.fly.io/$app:$label" '
    . == {schema_version:1,kind:$kind,app:$app,source_commit:$source,
      base_image:$base,image:$image}
  ' "$tmp/output.json" >/dev/null
  printf '%s\n' deploy . --app "$app" --config fly.toml \
    --build-arg "BASE_IMAGE=$base_image" --build-arg "SOURCE_COMMIT=$source_commit" \
    --image-label "$label" --build-only --push --remote-only > "$tmp/expected-args"
  diff "$tmp/expected-args" "$tmp/fly-args"
  printf '%s\n' ./.cargo/config.toml ./Cargo.lock ./Cargo.toml \
    ./crates/stub/src/lib.rs ./Dockerfile ./fly.toml "./$entrypoint" \
    "./$entrypoint_source" | sort > "$tmp/expected-context"
  diff "$tmp/expected-context" "$tmp/fly-context"
  cmp "$repository/Dockerfile.$overlay" "$tmp/fly-context.dockerfile"
  cmp "$repository/fly.$overlay.toml" "$tmp/fly-context.fly-toml"
  cmp "$repository/$entrypoint_source" "$tmp/fly-context.entrypoint"

  expect_failure_before_flyctl "$role: BASE_IMAGE must be digest-pinned" \
    "$role" BASE_IMAGE=registry.fly.io/ytsearch-raw-production:base
  expect_failure_before_flyctl "$role: a different IMAGE_LABEL must be rejected" \
    "$role" IMAGE_LABEL=other
  build "$role" IMAGE_LABEL="$label" >/dev/null
}

check_build reader raw-reader-overlay ytsearch-raw-production raw-overlay \
  raw-search-overlay raw-search-entrypoint scripts/raw-search-entrypoint.sh
check_build indexer raw-indexer-overlay ytsearch-indexer-raw-production raw-indexer \
  raw-indexer-overlay cloud-index-queue scripts/cloud-index-queue-entrypoint.sh
build reader > "$tmp/default-app.json"
jq -e '.app == "ytsearch-raw-production"' "$tmp/default-app.json" >/dev/null
build indexer > "$tmp/default-app.json"
jq -e '.app == "ytsearch-indexer-raw-production"' "$tmp/default-app.json" >/dev/null

# Every input that reaches the image, and the build scripts themselves, must be
# committed.
for path in crates/stub/src/lib.rs Dockerfile.raw-search-overlay \
  Dockerfile.raw-indexer-overlay scripts/raw-search-entrypoint.sh \
  scripts/cloud-index-queue-entrypoint.sh scripts/build-raw-overlay.sh \
  scripts/require-committed-rust-build-inputs.sh; do
  cp "$repository/$path" "$tmp/original"
  printf '\n# uncommitted\n' >> "$repository/$path"
  case "$path" in
    *search*) roles=(reader) ;;
    *indexer*|*cloud-index*) roles=(indexer) ;;
    *) roles=(reader indexer) ;;
  esac
  for role in "${roles[@]}"; do
    expect_failure_before_flyctl "$role: uncommitted $path must be rejected" "$role"
  done
  cp "$tmp/original" "$repository/$path"
done
printf '\n' > "$repository/crates/stub/src/untracked.rs"
expect_failure_before_flyctl 'untracked Rust input must be rejected' reader
expect_failure_before_flyctl 'untracked Rust input must be rejected' indexer
rm "$repository/crates/stub/src/untracked.rs"
build reader >/dev/null
if build other >/dev/null 2>&1; then
  printf 'an unknown overlay role must be rejected\n' >&2
  exit 1
fi

printf 'raw_overlay_builds=ok\n'
